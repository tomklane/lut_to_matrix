//! Fitting a correction from a measured chart to its target values.
//!
//! The correction runs where the DCTL runs: on DaVinci Wide Gamut / Intermediate
//! footage. Measured patches are decoded to CIE XYZ (D65) with the still's encoding; the
//! fit finds what takes them to the target (the published chart, or another camera's
//! chart):
//! - **Matrix**: one 3 × 3 matrix in linear XYZ, exactly as the LUT matrices (same DCTL).
//! - **Matrix + tone**: a per-channel power curve with a black offset in linear DWG
//!   first, then the matrix, for footage whose neutral row is not proportional to light
//!   (a baked-in tone curve, flare). The curve is linear below a toe so it stays monotonic
//!   and finite for negative values.
//!
//! - **Preserve neutrals** (either model): equal R, G, B in linear DWG stay equal (every
//!   row of the matrix in DWG sums to the same gain; one curve for all three channels), so
//!   clipped white stays white. The correction can then no longer white-balance: the
//!   grays keep what is left of the camera's cast (DECISIONS.md D42).
//!
//! Errors are ΔE2000 in CIELAB with the target's white (Y = 1 for a perfect reflector,
//! scaled by the exposure). The optimiser minimises ΔL*a*b* (smooth), neutral patches
//! weighted, seeded by least squares in XYZ (DECISIONS.md D39).

use nalgebra::{DMatrix, DVector};
use serde::{Deserialize, Serialize};

use crate::difference::{delta_e_2000, xyz_to_lab};
use crate::fit::{least_squares, least_squares_constrained, refine, Param};
use crate::math::{whites, Mat3, Vec3};
use crate::spaces::davinci;

/// What happens to the chart's exposure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Exposure {
    /// The correction also sets the exposure: the target's neutral patches land at the
    /// target's level (the published reflectances, or the other chart's exposure).
    #[default]
    Match,
    /// The correction keeps the still's exposure: unity gain on the neutral patches.
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    #[default]
    Matrix,
    MatrixTone,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FitSettings {
    pub model: Model,
    pub exposure: Exposure,
    /// Map the measured neutral (sum of neutral 8 … neutral 3.5) exactly onto the
    /// target's, so gray stays gray.
    pub lock_neutral: bool,
    /// Weight of each neutral patch relative to a colour patch.
    pub neutral_weight: f64,
    /// Pull the matrix towards a plain white balance (per-channel gains in DWG that take
    /// the measured gray to the target's): 0 = none (best chart match), 1 = strong.
    /// Limits how hard the matrix works on colours the camera barely recorded (e.g. red
    /// under water), which amplifies noise and makes colours off the chart less
    /// predictable.
    pub restraint: f64,
    /// Equal R, G, B (linear DWG) in give equal R, G, B out, like an RGB mixer whose rows
    /// sum to 1: clipped white stays white. Replaces `lock_neutral` (both can't hold for
    /// a camera whose gray isn't neutral).
    pub preserve_neutrals: bool,
}

impl Default for FitSettings {
    fn default() -> Self {
        FitSettings {
            model: Model::Matrix,
            exposure: Exposure::Match,
            lock_neutral: true,
            neutral_weight: 2.0,
            restraint: 0.0,
            preserve_neutrals: false,
        }
    }
}

/// Per-channel tone curve in linear DWG: y = 0.18·((x − black)/0.18)^power above the
/// toe (x − black ≥ [`TONE_TOE`]), the tangent line below it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tone {
    pub power: [f64; 3],
    pub black: [f64; 3],
}

/// Where the tone curve turns into its tangent line (linear DWG, 8 stops under 18%).
pub const TONE_TOE: f64 = 0.18 / 256.0;

impl Tone {
    pub const IDENTITY: Tone = Tone { power: [1.0; 3], black: [0.0; 3] };

    pub fn channel(&self, k: usize, x: f64) -> f64 {
        tone_channel(x, self.power[k], self.black[k])
    }

    pub fn apply(&self, dwg: &Vec3) -> Vec3 {
        Vec3::new(self.channel(0, dwg.x), self.channel(1, dwg.y), self.channel(2, dwg.z))
    }
}

pub fn tone_channel(x: f64, p: f64, b: f64) -> f64 {
    let t = x - b;
    let y0 = 0.18 * (TONE_TOE / 0.18).powf(p);
    if t >= TONE_TOE {
        0.18 * (t / 0.18).powf(p)
    } else {
        y0 + p * y0 / TONE_TOE * (t - TONE_TOE)
    }
}

/// One patch in the fit.
#[derive(Debug, Clone, Copy)]
pub struct FitPatch {
    /// Measured, CIE XYZ D65.
    pub xyz: Vec3,
    /// Target, CIE XYZ D65, at the target's exposure.
    pub target: Vec3,
    pub neutral: bool,
    pub used: bool,
}

/// The fitted correction.
#[derive(Debug, Clone, Serialize)]
pub struct ChartFit {
    pub settings: FitSettings,
    /// XYZ (D65) in → XYZ out, row-major.
    #[serde(serialize_with = "crate::math::ser::mat")]
    pub m_xyz: Mat3,
    /// The same as one matrix in linear DWG (XYZ_TO_DWG · M · DWG_TO_XYZ).
    #[serde(serialize_with = "crate::math::ser::mat")]
    pub m_dwg: Mat3,
    pub tone: Option<Tone>,
    /// Measured luminance over target luminance on the neutral patches (log-average):
    /// the still's exposure relative to the target, in stops = log2.
    pub exposure_ratio: f64,
    /// Neutral gain of the correction (Y out / Y in for the measured neutral).
    pub neutral_gain: f64,
    /// ΔE2000 per patch, every patch (the means count used patches only): before
    /// (exposure matched to the target), after.
    pub de_before: Vec<f64>,
    pub de_after: Vec<f64>,
    pub mean_before: f64,
    pub mean_after: f64,
    pub max_after: f64,
    pub neutral_after: f64,
    pub used: usize,
    /// The camera's mid gray in linear DWG, green = 1 (its cast; a white balance in the
    /// raw decode by the inverse gains makes it neutral).
    pub gray_dwg: [f64; 3],
    pub notes: Vec<String>,
}

impl ChartFit {
    /// Corrected XYZ for a measured XYZ.
    pub fn apply(&self, xyz: &Vec3) -> Vec3 {
        let x = match &self.tone {
            Some(t) => davinci::dwg_to_xyz() * t.apply(&(davinci::xyz_to_dwg() * xyz)),
            None => *xyz,
        };
        self.m_xyz * x
    }
}

fn lab(x: &Vec3, wn: &Vec3) -> Vec3 {
    xyz_to_lab(&x.map(|v| v.max(0.0)), wn)
}

/// Mid neutrals (neutral 8 … neutral 3.5): white often clips, black carries flare.
fn mid_neutrals(p: &[FitPatch]) -> Vec<usize> {
    (19..23).filter(|&i| p[i].used).collect()
}

/// Fit `patches` (24, chart order). Needs at least 6 used patches with 2 neutrals.
pub fn fit_chart(patches: &[FitPatch], s: &FitSettings) -> Result<ChartFit, String> {
    let used: Vec<usize> = (0..patches.len()).filter(|&i| patches[i].used).collect();
    let mids = mid_neutrals(patches);
    if used.len() < 6 {
        return Err(format!("only {} usable patches (6 needed): too many are clipped or excluded", used.len()));
    }
    let neutrals: Vec<usize> = used.iter().copied().filter(|&i| patches[i].neutral).collect();
    if neutrals.len() < 2 {
        return Err("fewer than 2 usable neutral patches".into());
    }
    let mut notes = Vec::new();
    // Exposure: log-average of measured/target luminance over mid neutrals (else all
    // usable neutrals).
    let ex_idx = if mids.len() >= 2 { mids.clone() } else { neutrals.clone() };
    let ratio = (ex_idx.iter().map(|&i| (patches[i].xyz.y.max(1e-9) / patches[i].target.y).ln()).sum::<f64>()
        / ex_idx.len() as f64)
        .exp();
    let k = match s.exposure {
        Exposure::Match => 1.0,
        Exposure::Keep => ratio,
    };
    // Lab white: a perfect reflector at the target exposure (times k when keeping).
    let wn = whites::D65.to_xyz() * k;
    let target: Vec<Vec3> = patches.iter().map(|p| p.target * k).collect();
    let weight = |i: usize| if patches[i].neutral { s.neutral_weight } else { 1.0 };

    // Before: the measured chart, exposure matched to the target.
    let de_before: Vec<f64> = (0..patches.len())
        .map(|i| {
            let x = patches[i].xyz * (k / ratio);
            delta_e_2000(&lab(&x, &wn), &lab(&target[i], &wn))
        })
        .collect();

    let resid_m = |tone: Option<&Tone>, m: &Mat3| -> Vec<f64> {
        let mut r = Vec::with_capacity(used.len() * 3);
        for &i in &used {
            let x = tone_xyz(tone, &patches[i].xyz);
            let d = lab(&(m * x), &wn) - lab(&target[i], &wn);
            let w = weight(i).sqrt();
            r.extend([d.x * w, d.y * w, d.z * w]);
        }
        r
    };

    // Neutral direction for the lock.
    let lock = |tone: Option<&Tone>| -> Option<(Vec3, Vec3)> {
        let idx = if mids.len() >= 2 { &mids } else { &neutrals };
        let w: Vec3 = idx.iter().map(|&i| tone_xyz(tone, &patches[i].xyz)).sum();
        let t: Vec3 = idx.iter().map(|&i| target[i]).sum();
        (s.lock_neutral && !s.preserve_neutrals && w.norm() > 0.0).then_some((w, t))
    };

    // The plain white balance the restraint pulls towards.
    let wb = |tone: Option<&Tone>| -> Mat3 {
        let idx = if mids.len() >= 2 { &mids } else { &neutrals };
        let w = davinci::xyz_to_dwg() * idx.iter().map(|&i| tone_xyz(tone, &patches[i].xyz)).sum::<Vec3>();
        let t = davinci::xyz_to_dwg() * idx.iter().map(|&i| target[i]).sum::<Vec3>();
        let g = Vec3::new(t.x / w.x, t.y / w.y, t.z / w.z).map(|v| if v.is_finite() && v > 0.0 { v } else { 1.0 });
        davinci::dwg_to_xyz() * Mat3::from_diagonal(&g) * davinci::xyz_to_dwg()
    };
    let penalty = |m: &Mat3, m_wb: &Mat3, r: &mut Vec<f64>| {
        if s.restraint > 0.0 {
            // Per element, in units of the matrix in DWG (where "1" is a whole channel).
            let d = davinci::xyz_to_dwg() * (m - m_wb) * davinci::dwg_to_xyz();
            let scale = 20.0 * s.restraint * (used.len() as f64 / 24.0).sqrt();
            r.extend(d.iter().map(|v| v * scale));
        }
    };

    let ls = |tone: Option<&Tone>| -> Option<Mat3> {
        let x: Vec<Vec3> = used.iter().map(|&i| tone_xyz(tone, &patches[i].xyz)).collect();
        let y: Vec<Vec3> = used.iter().map(|&i| target[i]).collect();
        // Weight by 1/Y so dark patches count (an XYZ fit is dominated by bright ones).
        let wt: Vec<f64> = used.iter().map(|&i| weight(i) / target[i].y.max(1e-3)).collect();
        match lock(tone) {
            Some((w, t)) => least_squares_constrained(&x, &y, Some(&wt), &w, &t),
            None => least_squares(&x, &y, Some(&wt)),
        }
    };

    // Black offsets are bounded relative to each channel's measured neutral-5-ish level in
    // DWG (the flare a black offset models is a fraction of it).
    let lvl: Vec3 = {
        let idx = if mids.len() >= 2 { &mids } else { &neutrals };
        idx.iter().map(|&i| davinci::xyz_to_dwg() * patches[i].xyz).sum::<Vec3>() / idx.len() as f64
    };
    let gray_dwg = if lvl.y.abs() > 1e-12 { [lvl.x / lvl.y, 1.0, lvl.z / lvl.y] } else { [1.0; 3] };

    // Preserve neutrals: the matrix in DWG with every row summing to g,
    // q = [g, a01, a02, a10, a12, a20, a21]; the tone model adds one power and one black
    // for all channels in front.
    let preserved = s.preserve_neutrals.then(|| {
        let to_xyz = |q: &[f64]| -> Mat3 {
            let g = q[0];
            let d = Mat3::new(g - q[1] - q[2], q[1], q[2], q[3], g - q[3] - q[4], q[4], q[5], q[6], g - q[5] - q[6]);
            davinci::dwg_to_xyz() * d * davinci::xyz_to_dwg()
        };
        let pack = |m: &Mat3| -> Vec<f64> {
            let d = davinci::xyz_to_dwg() * m * davinci::dwg_to_xyz();
            let g = (d.row(0).sum() + d.row(1).sum() + d.row(2).sum()) / 3.0;
            vec![g, d[(0, 1)], d[(0, 2)], d[(1, 0)], d[(1, 2)], d[(2, 0)], d[(2, 1)]]
        };
        // Restraint: towards a plain gain (an untouched mixer).
        let pull = |q: &[f64], r: &mut Vec<f64>| {
            if s.restraint > 0.0 {
                let scale = 20.0 * s.restraint * (used.len() as f64 / 24.0).sqrt();
                r.extend(q[1..7].iter().map(|v| v * scale));
                r.extend([q[1] + q[2], q[3] + q[4], q[5] + q[6]].map(|v| v * scale));
            }
        };
        let level = ((lvl.x + lvl.y + lvl.z).abs() / 3.0).max(1e-4);
        let tone_of = |p: f64, b: f64| Tone { power: [p; 3], black: [b * level; 3] };
        let resid_for = |with_tone: bool| {
            move |q: &[f64]| -> Vec<f64> {
                let off = if with_tone { 2 } else { 0 };
                let t = with_tone.then(|| tone_of(q[0], q[1]));
                let mut r = resid_m(t.as_ref(), &to_xyz(&q[off..]));
                pull(&q[off..], &mut r);
                r
            }
        };
        let resid = resid_for(false);
        // Seeds: a plain gain, and the free least-squares fit made neutral-preserving.
        let xs: Vec<Vec3> = used.iter().map(|&i| patches[i].xyz).collect();
        let ys: Vec<Vec3> = used.iter().map(|&i| target[i]).collect();
        let wt: Vec<f64> = used.iter().map(|&i| weight(i) / target[i].y.max(1e-3)).collect();
        let mut seeds = vec![pack(&(Mat3::identity() * (k / ratio)))];
        if let Some(m0) = least_squares(&xs, &ys, Some(&wt)) {
            seeds.push(pack(&m0));
        }
        let (lo, hi) = (vec![f64::NEG_INFINITY; 7], vec![f64::INFINITY; 7]);
        let cost = |q: &[f64]| resid(q).iter().map(|v| v * v).sum::<f64>();
        let best = seeds
            .into_iter()
            .map(|q| bounded_lm(&q, &lo, &hi, &resid, 300))
            .min_by(|a, b| cost(a).total_cmp(&cost(b)))
            .expect("at least one seed");
        if s.model == Model::Matrix {
            return (to_xyz(&best), None);
        }
        // Tone model: from the matrix (a power of 1 and no black are the identity), so it
        // is never worse than the matrix alone.
        let q0: Vec<f64> = [1.0, 0.0].into_iter().chain(best).collect();
        let lo: Vec<f64> = [0.5, -0.3].into_iter().chain([f64::NEG_INFINITY; 7]).collect();
        let hi: Vec<f64> = [2.0, 0.6].into_iter().chain([f64::INFINITY; 7]).collect();
        let q = bounded_lm(&q0, &lo, &hi, &resid_for(true), 300);
        (to_xyz(&q[2..]), Some(tone_of(q[0], q[1])))
    });
    if let Some((_, Some(t))) = &preserved {
        if t.power[0] <= 0.501 || t.power[0] >= 1.999 {
            notes.push("the tone curve reached its power limit (0.5 or 2): the neutral row is far from linear".into());
        }
    }

    let (m, tone) = match (preserved, s.model) {
        (Some(r), _) => r,
        (None, Model::Matrix) => {
            let m0 = ls(None).ok_or("the measured patches don't span three colour dimensions")?;
            let m_wb = wb(None);
            let res = |m: &Mat3| {
                let mut r = resid_m(None, m);
                penalty(m, &m_wb, &mut r);
                r
            };
            let m = match lock(None) {
                Some((w, _)) => refine(&m0, &Param::Neutral { w }, &res),
                None => refine(&m0, &Param::Free, &res),
            };
            (m, None)
        }
        (None, Model::MatrixTone) => {
            let lvl = lvl.map(|v| v.abs().max(1e-4));
            let m0 = ls(None).ok_or("the measured patches don't span three colour dimensions")?;
            let mut p0: Vec<f64> = vec![1.0, 1.0, 1.0, 0.0, 0.0, 0.0];
            p0.extend(m0.transpose().as_slice());
            let lo: Vec<f64> = [0.5, 0.5, 0.5, -0.3, -0.3, -0.3].into_iter().chain([f64::NEG_INFINITY; 9]).collect();
            let hi: Vec<f64> = [2.0, 2.0, 2.0, 0.6, 0.6, 0.6].into_iter().chain([f64::INFINITY; 9]).collect();
            let unpack = |p: &[f64]| -> (Tone, Mat3) {
                let t = Tone { power: [p[0], p[1], p[2]], black: [p[3] * lvl.x, p[4] * lvl.y, p[5] * lvl.z] };
                (t, Mat3::from_row_slice(&p[6..15]))
            };
            let resid = |p: &[f64]| -> Vec<f64> {
                let (t, m) = unpack(p);
                let mut r = resid_m(Some(&t), &m);
                penalty(&m, &wb(Some(&t)), &mut r);
                if let Some((w, tt)) = lock(Some(&t)) {
                    // Strong penalty instead of an exact constraint (the curve moves w).
                    let d = lab(&(m * w / mids.len().max(2) as f64), &wn) - lab(&(tt / mids.len().max(2) as f64), &wn);
                    r.extend([30.0 * d.x, 30.0 * d.y, 30.0 * d.z]);
                }
                r
            };
            let p = bounded_lm(&p0, &lo, &hi, &resid, 300);
            let (t, m) = unpack(&p);
            // Make the neutral exact like the matrix model: one more constrained matrix
            // refinement with the curve fixed.
            let m = match lock(Some(&t)) {
                Some((w, tt)) => {
                    let base = least_squares_constrained(
                        &used.iter().map(|&i| tone_xyz(Some(&t), &patches[i].xyz)).collect::<Vec<_>>(),
                        &used.iter().map(|&i| target[i]).collect::<Vec<_>>(),
                        Some(&used.iter().map(|&i| weight(i) / target[i].y.max(1e-3)).collect::<Vec<_>>()),
                        &w,
                        &tt,
                    )
                    .unwrap_or(m);
                    let m_wb = wb(Some(&t));
                    let res = |mm: &Mat3| {
                        let mut r = resid_m(Some(&t), mm);
                        penalty(mm, &m_wb, &mut r);
                        r
                    };
                    let start = if (res(&base).iter().map(|v| v * v).sum::<f64>()) < 1e30 { base } else { m };
                    refine(&start, &Param::Neutral { w }, &res)
                }
                None => m,
            };
            if t.power.iter().any(|p| *p <= 0.501 || *p >= 1.999) {
                notes.push("the tone curve reached its power limit (0.5 or 2): the neutral row is far from linear".into());
            }
            (m, Some(t))
        }
    };

    let fit = ChartFit {
        settings: *s,
        m_xyz: m,
        m_dwg: davinci::xyz_to_dwg() * m * davinci::dwg_to_xyz(),
        tone,
        exposure_ratio: ratio,
        neutral_gain: 0.0,
        de_before,
        de_after: Vec::new(),
        mean_before: 0.0,
        mean_after: 0.0,
        max_after: 0.0,
        neutral_after: 0.0,
        used: used.len(),
        gray_dwg,
        notes,
    };
    let de_after: Vec<f64> = (0..patches.len())
        .map(|i| delta_e_2000(&lab(&fit.apply(&patches[i].xyz), &wn), &lab(&target[i], &wn)))
        .collect();
    let mean = |v: &[f64], idx: &[usize]| idx.iter().map(|&i| v[i]).sum::<f64>() / idx.len().max(1) as f64;
    let g = {
        let idx = if mids.len() >= 2 { &mids } else { &neutrals };
        let w: Vec3 = idx.iter().map(|&i| patches[i].xyz).sum();
        fit.apply(&w).y / w.y
    };
    let mut fit = fit;
    fit.mean_before = mean(&fit.de_before, &used);
    fit.mean_after = mean(&de_after, &used);
    fit.max_after = used.iter().map(|&i| de_after[i]).fold(0.0, f64::max);
    fit.neutral_after = mean(&de_after, &neutrals);
    fit.neutral_gain = g;
    fit.de_after = de_after;
    let [r, _, b] = gray_dwg;
    let cast = r.max(1e-9).log2().abs().max(b.max(1e-9).log2().abs());
    if s.preserve_neutrals && cast > 0.15 {
        fit.notes.push(format!(
            "with neutrals preserved the correction can't white-balance: this camera's gray is R {r:.2} : G 1 : B {b:.2} in \
             linear DWG. White-balance it in the raw decode (before anything clips) and fit again on a still decoded that way"
        ));
    }
    let w = fit.m_dwg * Vec3::repeat(1.0);
    if !s.preserve_neutrals && w.min() < 0.8 * w.max() {
        fit.notes.push(format!(
            "equal R, G, B (clipped white) comes out R {:.2} : G {:.2} : B {:.2}: turn on highlight protection for the DCTL, \
             or white-balance in the raw decode and preserve neutrals",
            w.x / w.max(),
            w.y / w.max(),
            w.z / w.max()
        ));
    }
    if fit.mean_after > 6.0 {
        fit.notes.push(format!(
            "mean error after correction is {:.1} ΔE2000: the camera's colours differ from the chart in ways a {} can't \
             undo (spectral differences, e.g. strongly coloured or filtered light, or a non-linear look baked into the still)",
            fit.mean_after,
            if s.model == Model::Matrix { "3×3 matrix" } else { "matrix and tone curve" }
        ));
    }
    Ok(fit)
}

fn tone_xyz(tone: Option<&Tone>, xyz: &Vec3) -> Vec3 {
    match tone {
        Some(t) => davinci::dwg_to_xyz() * t.apply(&(davinci::xyz_to_dwg() * xyz)),
        None => *xyz,
    }
}

/// Box-bounded Levenberg–Marquardt (central differences, projected steps).
pub(crate) fn bounded_lm(p0: &[f64], lo: &[f64], hi: &[f64], resid: &dyn Fn(&[f64]) -> Vec<f64>, iters: usize) -> Vec<f64> {
    let n = p0.len();
    let mut p: Vec<f64> = p0.iter().enumerate().map(|(k, v)| v.clamp(lo[k], hi[k])).collect();
    let mut r = resid(&p);
    let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let mut c = cost(&r);
    let mut lambda = 1e-3;
    for _ in 0..iters {
        let m = r.len();
        let mut j = DMatrix::<f64>::zeros(m, n);
        for k in 0..n {
            let h = 1e-6 * (1.0 + p[k].abs());
            let mut pp = p.clone();
            pp[k] = (p[k] + h).min(hi[k]);
            let up = pp[k];
            let rp = resid(&pp);
            pp[k] = (p[k] - h).max(lo[k]);
            let dn = pp[k];
            let rm = resid(&pp);
            let span = up - dn;
            if span <= 0.0 {
                continue;
            }
            for i in 0..m {
                j[(i, k)] = (rp[i] - rm[i]) / span;
            }
        }
        let rv = DVector::from_vec(r.clone());
        let jtj = j.transpose() * &j;
        let g = j.transpose() * &rv;
        let free: Vec<usize> = (0..n).filter(|&k| !((p[k] <= lo[k] && g[k] > 0.0) || (p[k] >= hi[k] && g[k] < 0.0))).collect();
        if free.is_empty() || free.iter().all(|&k| g[k].abs() < 1e-12) {
            break;
        }
        let mut improved = false;
        for _ in 0..20 {
            let nf = free.len();
            let mut a = DMatrix::from_fn(nf, nf, |i, k| jtj[(free[i], free[k])]);
            for d in 0..nf {
                a[(d, d)] += lambda * jtj[(free[d], free[d])].max(1e-12);
            }
            let b = DVector::from_fn(nf, |i, _| -g[free[i]]);
            let Some(step) = a.lu().solve(&b) else {
                lambda *= 10.0;
                continue;
            };
            let mut pn = p.clone();
            for (i, &k) in free.iter().enumerate() {
                pn[k] = (p[k] + step[i]).clamp(lo[k], hi[k]);
            }
            let rn = resid(&pn);
            let cn = cost(&rn);
            if cn < c {
                let rel = (c - cn) / c.max(1e-300);
                p = pn;
                r = rn;
                c = cn;
                lambda = (lambda * 0.3).max(1e-12);
                improved = true;
                if rel < 1e-10 {
                    return p;
                }
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::reference::ChartEdition;

    fn patches(f: impl Fn(&Vec3) -> Vec3) -> Vec<FitPatch> {
        ChartEdition::After2014
            .xyz_d65()
            .iter()
            .enumerate()
            .map(|(i, t)| FitPatch { xyz: f(t), target: *t, neutral: i >= 18, used: true })
            .collect()
    }

    fn cam() -> Mat3 {
        // A camera "look" that keeps D65 neutral, plus an exposure of −1 stop.
        let m = Mat3::new(1.12, -0.10, 0.03, -0.04, 1.05, -0.02, 0.02, -0.09, 1.15);
        let w = whites::D65.to_xyz();
        let g = m * w;
        Mat3::from_diagonal(&w.component_div(&g)) * m * 0.5
    }

    #[test]
    fn matrix_recovers_the_inverse() {
        let c = cam();
        let p = patches(|t| c * t);
        let f = fit_chart(&p, &FitSettings::default()).unwrap();
        assert!(f.mean_after < 1e-4, "{}", f.mean_after);
        assert!((f.m_xyz * c - Mat3::identity()).abs().max() < 1e-5, "{}", f.m_xyz * c);
        assert!((f.exposure_ratio - 0.5).abs() < 0.01, "{}", f.exposure_ratio);
        assert!(f.mean_before > 1.0);
        // Keep: same colours, unity neutral gain.
        let f = fit_chart(&p, &FitSettings { exposure: Exposure::Keep, ..Default::default() }).unwrap();
        assert!(f.mean_after < 1e-4 && (f.neutral_gain - 1.0).abs() < 1e-3, "{} {}", f.mean_after, f.neutral_gain);
    }

    #[test]
    fn tone_model_undoes_a_power_curve() {
        let c = cam();
        let tone = Tone { power: [1.25, 1.2, 1.3], black: [0.0; 3] };
        // Footage = inverse tone of the true colours (in DWG), so the fitted tone is ~tone.
        let inv = |x: f64, p: f64| if x > 0.0 { 0.18 * (x / 0.18).powf(1.0 / p) } else { x };
        let p = patches(|t| {
            let d = davinci::xyz_to_dwg() * (c * t);
            let d = Vec3::new(inv(d.x, tone.power[0]), inv(d.y, tone.power[1]), inv(d.z, tone.power[2]));
            davinci::dwg_to_xyz() * d
        });
        let m = fit_chart(&p, &FitSettings::default()).unwrap();
        let t = fit_chart(&p, &FitSettings { model: Model::MatrixTone, ..Default::default() }).unwrap();
        assert!(t.mean_after < 0.3 && t.mean_after < 0.3 * m.mean_after, "tone {} vs matrix {}", t.mean_after, m.mean_after);
    }

    /// A cyan underwater-like cast in DWG (red barely recorded), as XYZ → XYZ.
    fn cast() -> Mat3 {
        let c = Mat3::new(0.5, 0.05, 0.0, 0.0, 0.8, 0.02, 0.0, 0.03, 1.0);
        davinci::dwg_to_xyz() * c * davinci::xyz_to_dwg()
    }

    fn row_sums_dwg(f: &ChartFit) -> Vec3 {
        f.m_dwg * Vec3::repeat(1.0)
    }

    #[test]
    fn preserve_neutrals_keeps_equal_rgb_equal() {
        // A look that keeps D65 neutral: preserving neutrals costs nothing.
        let c = cam();
        let p = patches(|t| c * t);
        let s = FitSettings { preserve_neutrals: true, ..Default::default() };
        let f = fit_chart(&p, &s).unwrap();
        assert!(f.mean_after < 1e-3, "{}", f.mean_after);
        let r = row_sums_dwg(&f);
        assert!((r.x - r.y).abs() < 1e-9 && (r.z - r.y).abs() < 1e-9, "{r}");
        // A cast: the free fit turns equal RGB red, the preserving one can't, and says
        // the grays keep a cast.
        let c = cast();
        let p = patches(|t| c * t);
        let free = fit_chart(&p, &FitSettings::default()).unwrap();
        let w = row_sums_dwg(&free);
        assert!(w.x > 1.5 * w.y, "free fit keeps white? {w}");
        let f = fit_chart(&p, &s).unwrap();
        let r = row_sums_dwg(&f);
        assert!((r.x - r.y).abs() < 1e-9 && (r.z - r.y).abs() < 1e-9, "{r}");
        assert!(f.neutral_after > free.neutral_after + 2.0, "{} vs {}", f.neutral_after, free.neutral_after);
        assert!(f.notes.iter().any(|n| n.contains("raw decode")), "{:?}", f.notes);
        assert!(free.notes.iter().any(|n| n.contains("clipped white")), "{:?}", free.notes);
        // Gray = row sums of the cast: R 0.55 : G 0.82 : B 1.03.
        assert!((f.gray_dwg[0] - 0.55 / 0.82).abs() < 0.01 && (f.gray_dwg[2] - 1.03 / 0.82).abs() < 0.01, "{:?}", f.gray_dwg);
        // Neutral lock is ignored, the result the same.
        let g = fit_chart(&p, &FitSettings { lock_neutral: false, ..s }).unwrap();
        assert!((g.m_xyz - f.m_xyz).abs().max() < 1e-6);
        // Tone model: one curve for all channels, still neutral-preserving.
        let t = fit_chart(&p, &FitSettings { model: Model::MatrixTone, ..s }).unwrap();
        let tone = t.tone.unwrap();
        assert!(tone.power[0] == tone.power[1] && tone.power[1] == tone.power[2] && tone.black[0] == tone.black[2]);
        let x = davinci::dwg_to_xyz() * Vec3::repeat(0.7);
        let y = davinci::xyz_to_dwg() * t.apply(&x);
        assert!((y.x - y.y).abs() < 1e-9 * y.y.abs().max(1.0) && (y.z - y.y).abs() < 1e-9 * y.y.abs().max(1.0), "{y}");
        // (It minimises weighted ΔLab, so the mean ΔE2000 can move a little either way.)
        assert!(t.mean_after < f.mean_after + 0.3, "{} vs {}", t.mean_after, f.mean_after);
    }

    #[test]
    fn highlight_protection_turns_clipped_white_white() {
        use crate::chart::highlights::{ClipSource, Highlights};
        let c = cast();
        let p = patches(|t| c * t);
        let f = fit_chart(&p, &FitSettings::default()).unwrap();
        let hl = Highlights {
            enabled: true,
            clip_di: 0.68,
            source: ClipSource::Set,
            measured_di: 0.68,
            measured_source: ClipSource::Set,
            softness: 1.0,
            strength: 1.0,
            detect: 0.0,
        };
        let lc = davinci::di_decode(0.68);
        let chroma = |v: &Vec3| (v.x / (v.x + v.y + v.z), v.y / (v.x + v.y + v.z));
        let d65 = chroma(&(davinci::dwg_to_xyz() * Vec3::repeat(1.0)));
        // Clipped in every channel, or green only: neutral out, nothing like the
        // unprotected colour.
        for px in [Vec3::repeat(lc), Vec3::new(0.6 * lc, lc, 0.8 * lc)] {
            let raw = chroma(&f.apply(&(davinci::dwg_to_xyz() * px)));
            let out = chroma(&hl.apply(&f, &px));
            assert!((out.0 - d65.0).abs() < 1e-9 && (out.1 - d65.1).abs() < 1e-9, "{out:?}");
            assert!((raw.0 - d65.0).abs() > 0.02, "unprotected {raw:?}");
        }
        // Well below the clip: untouched.
        let px = Vec3::new(0.05, 0.1, 0.12);
        assert!((hl.apply(&f, &px) - f.apply(&(davinci::dwg_to_xyz() * px))).abs().max() < 1e-12);
        assert_eq!(hl.weight(&Vec3::repeat(lc * 0.5)), 0.0);
        assert!((hl.weight(&Vec3::repeat(lc * 0.5f64.sqrt())) - 0.5).abs() < 1e-9);
        // Detect: one channel at the clip counts fully with 0, not at all with 1.
        let one = Vec3::new(0.2 * lc, lc, 0.3 * lc);
        assert_eq!(hl.weight(&one), 1.0);
        assert_eq!(Highlights { detect: 1.0, ..hl }.weight(&one), 0.0);
        assert_eq!(Highlights { detect: 1.0, ..hl }.weight(&Vec3::repeat(lc)), 1.0);
        // Off: the plain correction.
        let off = Highlights { enabled: false, ..hl };
        let px = Vec3::repeat(lc);
        assert!((off.apply(&f, &px) - f.apply(&(davinci::dwg_to_xyz() * px))).abs().max() < 1e-12);
    }

    #[test]
    fn tone_curve_is_monotonic_and_continuous() {
        for p in [0.5, 0.8, 1.0, 1.4, 2.0] {
            let mut prev = f64::NEG_INFINITY;
            for k in -200..400 {
                let x = k as f64 * 0.0005 - 0.02;
                let y = tone_channel(x, p, 0.01);
                assert!(y > prev, "p {p} x {x}");
                prev = y;
            }
            let a = tone_channel(0.01 + TONE_TOE - 1e-12, p, 0.01);
            let b = tone_channel(0.01 + TONE_TOE + 1e-12, p, 0.01);
            assert!((a - b).abs() < 1e-9);
        }
    }
}
