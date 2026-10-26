//! Fit OpenDRT's look parameters so that OpenDRT reproduces a LUT ("LUT Match" presets).
//!
//! OpenDRT is fed DaVinci Wide Gamut / DaVinci Intermediate and renders to the LUT's own
//! display space; the LUT is fed the same scene colours in its camera encoding. The fit
//! minimises the perceptual difference (ΔE2000 for SDR, ΔE ITP for PQ) between the two
//! over a gray ramp and CIELAB hue rings across exposures (DECISIONS.md D24, D28):
//!
//! 1. Search every look × tonescale preset × creative white (D65/D75/D60) with the
//!    Display Grey Luminance matched to the LUT's middle gray.
//! 2. Refine the best few starts with bounded Levenberg–Marquardt over every look
//!    parameter (slider ranges as bounds) plus the grey level. Modules a preset has off
//!    are switched on at zero strength so the optimiser can use them.
//!    Optionally the LUT's gray-axis cast is removed from the targets first
//!    ([`crate::gray_axis`], D33), since OpenDRT's gray is always neutral.
//! 3. Safety checks on the result: finite in-range output, luminance that never falls as
//!    exposure rises (else the fit is redone with brilliance frozen), and smoothness
//!    compared with the stock presets.

use std::time::Instant;

use lutmatrix_opendrt::tables::{CWP_LABELS, LOOK_PRESET_LABELS, TONESCALE_PRESET_LABELS};
use lutmatrix_opendrt::{Config, DisplayGamut as DrtGamut, Eotf, Kind, LookParams, OpenDrt, Presets};
use nalgebra::{DMatrix, DVector};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::difference::{Metric, Scorer};
use crate::gray_axis::{GrayAxis, GrayPoint, NeutralGray, VISIBLE_DUV};
use crate::lut::{Interpolation, Transform};
use crate::math::{whites, Vec3};
use crate::sample::{hue_ring, RingLimit, Settings};
use crate::spaces::curves::{di_decode, di_encode};
use crate::spaces::{davinci, CameraSpaceId, DisplayGamut, DisplayTransfer, InputSpace, OutputSpace};

#[derive(Debug, Error)]
pub enum MatchError {
    #[error("OpenDRT has no output matching {0}")]
    UnsupportedOutput(String),
    #[error("only {0} samples are inside the LUT's input range; is the input space right?")]
    TooFewSamples(usize),
}

/// Knobs of the fit. Defaults reproduce the earlier Python tool's method (D24).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchSettings {
    pub interpolation: Interpolation,
    pub metric: Metric,
    /// Gamut that bounds the sample hue rings. `None`: the LUT's input gamut, except
    /// ACES AP0 (mostly imaginary colours), which uses ARRI Wide Gamut 3 as a realistic
    /// camera gamut.
    pub ring_gamut: Option<CameraSpaceId>,
    /// Look parameters kept at the start preset's values, by name prefix ("brl" keeps
    /// brilliance and post brilliance). "grey" keeps the start grey level.
    pub freeze: Vec<String>,
    /// Pull towards the start preset (per parameter, in slider-range units).
    pub regularization: f64,
    /// Levenberg–Marquardt iterations per start.
    pub max_iterations: usize,
    /// Distinct (look, tonescale) starts to refine.
    pub starts: usize,
    /// Creative whites tried in the search (OpenDRT index: 0 D93 … 2 D65 … 5 D50).
    pub creative_whites: Vec<usize>,
    /// If the exposure sweep finds luminance falling with exposure, refit with
    /// brilliance frozen.
    pub auto_freeze_brilliance: bool,
    /// Remove the LUT's gray-axis cast before matching (D33).
    pub neutral_gray: NeutralGray,
    /// Strength of that correction, 0…1.
    pub neutral_amount: f64,
    /// Worker threads (0 = all cores).
    pub threads: usize,
}

impl Default for MatchSettings {
    fn default() -> Self {
        MatchSettings {
            interpolation: Interpolation::Tetrahedral,
            metric: Metric::Auto,
            ring_gamut: None,
            freeze: Vec::new(),
            regularization: 0.0,
            max_iterations: 150,
            starts: 3,
            creative_whites: vec![2, 1, 3],
            auto_freeze_brilliance: true,
            neutral_gray: NeutralGray::Off,
            neutral_amount: 1.0,
            threads: 0,
        }
    }
}

/// Where the refinement started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Start {
    pub look: usize,
    pub tonescale: usize,
    /// OpenDRT creative white index (0 D93 … 5 D50).
    pub creative_white: usize,
    pub description: String,
    /// Weighted mean error of the unmodified preset (grey matched).
    pub score: f64,
}

/// Mean errors (ΔE2000 or ΔE ITP) over the training samples.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Score {
    pub mean: f64,
    /// Gray samples count 4×.
    pub weighted_mean: f64,
    pub p95: f64,
    pub max: f64,
    /// Mean over the gray ramp only (how well the gray axis is matched).
    #[serde(default)]
    pub gray_mean: f64,
}

/// A luminance reversal found by the exposure sweep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reversal {
    /// "gray" or "hue 120°, 0.95 of camera gamut limit".
    pub colour: String,
    /// Exposure (stops relative to 18% gray) where luminance starts to fall.
    pub at_stops: f64,
    /// Fall in luminance, percent of the level before.
    pub drop_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Safety {
    /// Every output over a 17³ input cube (−0.05…1.05) is finite and within 0–1.
    pub finite_in_range: bool,
    pub reversals: Vec<Reversal>,
    /// Largest second difference of output code along the lines of a 33³ DaVinci
    /// Intermediate cube: inside the camera gamut, ≤ +6 stops, unclipped channels only.
    pub roughness: f64,
    /// The same, largest over the stock looks.
    pub stock_roughness: f64,
}

impl Safety {
    pub fn ok(&self) -> bool {
        self.finite_in_range && self.reversals.is_empty() && self.roughness <= 1.5 * self.stock_roughness
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenDrtMatch {
    pub lut_name: Option<String>,
    pub input: InputSpace,
    pub output: OutputSpace,
    pub metric: Metric,
    /// The fitted OpenDRT settings; `config.grey_nits` is the fitted Display Grey
    /// Luminance at 100 nits peak.
    pub config: Config,
    pub start: Start,
    /// Standard look + ACES-2.0 tonescale at 10 nits grey (a stock baseline).
    pub stock: Score,
    /// The best unmodified preset (grey matched).
    pub best_preset: Score,
    pub fitted: Score,
    pub samples_used: usize,
    pub samples_total: usize,
    pub frozen: Vec<String>,
    /// The LUT's gray axis as measured (before any correction), at whole stops.
    #[serde(default)]
    pub gray_axis: Vec<GrayPoint>,
    /// The gray-axis correction the targets were given (scores are against the corrected
    /// LUT).
    #[serde(default)]
    pub neutral_gray: NeutralGray,
    #[serde(default = "one")]
    pub neutral_amount: f64,
    pub safety: Safety,
    pub notes: Vec<String>,
    pub seconds: f64,
}

fn one() -> f64 {
    1.0
}

struct Sample {
    di: [f32; 3],
    target: Vec3,
    weight: f64,
    gray: bool,
}

/// The fitting problem: samples plus how to score an OpenDRT config on them.
struct Problem {
    samples: Vec<Sample>,
    output: OutputSpace,
    scorer: Scorer,
    threads: usize,
}

/// OpenDRT display settings for a LUT output space.
fn drt_display(output: &OutputSpace) -> Result<(DrtGamut, Eotf, f32), MatchError> {
    let gamut = match output.gamut {
        DisplayGamut::Rec709 => DrtGamut::Rec709,
        DisplayGamut::P3D65 => DrtGamut::P3D65,
        DisplayGamut::P3Dci => DrtGamut::P3Dci,
        DisplayGamut::Rec2020 => DrtGamut::Rec2020P3Limited,
    };
    let (eotf, peak) = match output.transfer {
        DisplayTransfer::Gamma { gamma } if (gamma - 2.2).abs() < 1e-6 => (Eotf::Power22, 100.0),
        DisplayTransfer::Gamma { gamma } if (gamma - 2.4).abs() < 1e-6 => (Eotf::Power24, 100.0),
        DisplayTransfer::Gamma { gamma } if (gamma - 2.6).abs() < 1e-6 => (Eotf::Power26, 100.0),
        DisplayTransfer::Pq { peak_nits } => (Eotf::Pq, peak_nits as f32),
        _ => return Err(MatchError::UnsupportedOutput(output.name())),
    };
    Ok((gamut, eotf, peak))
}

/// Scene XYZ (D65, Y = 1 white) → OpenDRT input (DaVinci Intermediate, DWG).
fn to_di(xyz: &Vec3) -> [f32; 3] {
    let dwg = davinci::xyz_to_dwg() * xyz;
    [di_encode(dwg.x) as f32, di_encode(dwg.y) as f32, di_encode(dwg.z) as f32]
}

fn threads(n: usize) -> usize {
    if n > 0 {
        n
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
    }
}

impl Problem {
    fn display_xyz(&self, code: [f32; 3]) -> Vec3 {
        let c = Vec3::new(code[0] as f64, code[1] as f64, code[2] as f64);
        self.output.rgb_to_xyz() * self.output.decode(&c)
    }

    /// Per-sample error of `cfg`.
    fn errors(&self, cfg: &Config) -> Vec<f64> {
        let drt = OpenDrt::new(cfg);
        self.samples
            .iter()
            .map(|s| {
                let d = self.scorer.delta(&self.display_xyz(drt.apply(s.di)), &s.target);
                if d.is_finite() {
                    d
                } else {
                    1e3
                }
            })
            .collect()
    }

    fn score(&self, cfg: &Config) -> Score {
        summarize(&self.errors(cfg), &self.samples)
    }

    fn weighted(&self, cfg: &Config) -> f64 {
        let e = self.errors(cfg);
        let wsum: f64 = self.samples.iter().map(|s| s.weight).sum();
        e.iter().zip(&self.samples).map(|(e, s)| e * s.weight).sum::<f64>() / wsum
    }
}

fn summarize(e: &[f64], samples: &[Sample]) -> Score {
    let n = e.len().max(1) as f64;
    let gray: Vec<f64> = e.iter().zip(samples).filter(|(_, s)| s.gray).map(|(e, _)| *e).collect();
    let wsum: f64 = samples.iter().map(|s| s.weight).sum();
    let mut sorted = e.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let p95 = sorted.get(((sorted.len() as f64 * 0.95) as usize).min(sorted.len().saturating_sub(1))).copied();
    Score {
        mean: e.iter().sum::<f64>() / n,
        weighted_mean: e.iter().zip(samples).map(|(e, s)| e * s.weight).sum::<f64>() / wsum,
        p95: p95.unwrap_or(0.0),
        max: sorted.last().copied().unwrap_or(0.0),
        gray_mean: gray.iter().sum::<f64>() / gray.len().max(1) as f64,
    }
}

/// Training colours (XYZ, weight, is gray): gray ramp −7…+6 stops (weight 4); hue rings (24 hues × 0.3/0.6/0.9
/// of the camera ∩ display chroma limit) at −3…+2 stops; beyond-display rings (12 hues ×
/// 0.6/0.9 of the camera limit) at −1…+1 stops.
fn training_colours(ring_space: &InputSpace, output: &OutputSpace) -> Vec<(Vec3, f64, bool)> {
    let wn = whites::D65.to_xyz();
    let mut out = Vec::new();
    for k in 0..=26 {
        let s = -7.0 + 0.5 * k as f64;
        out.push((wn * (0.18 * 2f64.powf(s)), 4.0, true));
    }
    let ring = |hues: usize, sats: &[f64], limit: RingLimit, stops: f64| {
        let st = Settings { hues, saturations: sats.to_vec(), ring_limit: limit, ..Settings::default() };
        hue_ring(&st, ring_space, output, 0.18 * 2f64.powf(stops)).into_iter().map(|(_, x)| (x, 1.0, false))
    };
    for s in -3..=2 {
        out.extend(ring(24, &[0.3, 0.6, 0.9], RingLimit::CameraAndDisplay, s as f64));
    }
    for s in -1..=1 {
        out.extend(ring(12, &[0.6, 0.9], RingLimit::Camera, s as f64));
    }
    out
}

/// Modules a preset may have off, with the parameters that make them a no-op at 0.
const NEUTRAL: [(&str, &[&str]); 9] = [
    ("tn_hcon_enable", &["tn_hcon"]),
    ("tn_lcon_enable", &["tn_lcon"]),
    ("ptl_enable", &["ptl_c", "ptl_m", "ptl_y"]),
    ("ptm_enable", &["ptm_low", "ptm_high"]),
    ("brl_enable", &["brl", "brl_r", "brl_g", "brl_b"]),
    ("brlp_enable", &["brlp", "brlp_r", "brlp_g", "brlp_b"]),
    ("hc_enable", &["hc_r"]),
    ("hs_rgb_enable", &["hs_r", "hs_g", "hs_b"]),
    ("hs_cmy_enable", &["hs_c", "hs_m", "hs_y"]),
];

const GREY_MIN: f64 = 3.0;
const GREY_MAX: f64 = 25.0;

/// (look, tonescale, creative white) of a stock preset.
type PresetId = (usize, usize, usize);

/// The optimisation variables: indices of free look parameters (+ grey) in [0, 1] units.
struct Vars {
    look_idx: Vec<usize>,
    grey: bool,
}

impl Vars {
    fn new(freeze: &[String]) -> Vars {
        let frozen = |name: &str| freeze.iter().any(|f| name.starts_with(f.as_str()));
        let look_idx = LookParams::info()
            .iter()
            .enumerate()
            .filter(|(_, p)| p.kind == Kind::Float && !frozen(p.name))
            .map(|(i, _)| i)
            .collect();
        Vars { look_idx, grey: !freeze.iter().any(|f| f == "grey") }
    }

    fn to_unit(&self, cfg: &Config) -> Vec<f64> {
        let a = cfg.look.to_array();
        let info = LookParams::info();
        let mut u: Vec<f64> = self
            .look_idx
            .iter()
            .map(|&i| ((a[i] - info[i].min) as f64 / (info[i].max - info[i].min) as f64).clamp(0.0, 1.0))
            .collect();
        if self.grey {
            u.push(((cfg.grey_nits as f64 - GREY_MIN) / (GREY_MAX - GREY_MIN)).clamp(0.0, 1.0));
        }
        u
    }

    fn apply(&self, base: &Config, u: &[f64]) -> Config {
        let mut cfg = *base;
        let mut a = cfg.look.to_array();
        let info = LookParams::info();
        for (k, &i) in self.look_idx.iter().enumerate() {
            a[i] = (info[i].min as f64 + u[k] * (info[i].max - info[i].min) as f64) as f32;
        }
        cfg.look = LookParams::from_array(&a);
        if self.grey {
            cfg.grey_nits = (GREY_MIN + u[self.look_idx.len()] * (GREY_MAX - GREY_MIN)) as f32;
        }
        cfg
    }
}

/// Bounded Levenberg–Marquardt on u ∈ [0,1]ⁿ: forward-difference Jacobian (columns in
/// parallel), variables pinned at a bound by the gradient left out of the step, steps
/// projected onto the box.
fn bounded_lm(u0: Vec<f64>, resid: &(dyn Fn(&[f64]) -> Vec<f64> + Sync), max_iter: usize, threads: usize) -> Vec<f64> {
    let n = u0.len();
    let mut u = u0;
    let mut r = resid(&u);
    let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let mut c = cost(&r);
    let mut lambda = 1e-2;
    for _ in 0..max_iter {
        let m = r.len();
        let mut cols: Vec<Vec<f64>> = vec![Vec::new(); n];
        std::thread::scope(|sc| {
            let chunk = n.div_ceil(threads.max(1));
            for (ci, out) in cols.chunks_mut(chunk).enumerate() {
                let (u, r) = (&u, &r);
                sc.spawn(move || {
                    for (j, col) in out.iter_mut().enumerate() {
                        let k = ci * chunk + j;
                        let h = if u[k] + 2e-3 > 1.0 { -2e-3 } else { 2e-3 };
                        let mut up = u.clone();
                        up[k] += h;
                        let rp = resid(&up);
                        *col = rp.iter().zip(r).map(|(a, b)| (a - b) / h).collect();
                    }
                });
            }
        });
        let j = DMatrix::from_fn(m, n, |i, k| cols[k][i]);
        let rv = DVector::from_column_slice(&r);
        let jtj = j.transpose() * &j;
        let g = j.transpose() * &rv;
        let free: Vec<usize> = (0..n).filter(|&k| !((u[k] <= 0.0 && g[k] > 0.0) || (u[k] >= 1.0 && g[k] < 0.0))).collect();
        if free.is_empty() || free.iter().all(|&k| g[k].abs() < 1e-10) {
            break;
        }
        let mut improved = false;
        for _ in 0..12 {
            let nf = free.len();
            let mut a = DMatrix::from_fn(nf, nf, |p, q| jtj[(free[p], free[q])]);
            for d in 0..nf {
                a[(d, d)] += lambda * jtj[(free[d], free[d])].max(1e-9);
            }
            let b = DVector::from_fn(nf, |p, _| -g[free[p]]);
            let Some(step) = a.lu().solve(&b) else {
                lambda *= 10.0;
                continue;
            };
            let mut un = u.clone();
            for (p, &k) in free.iter().enumerate() {
                un[k] = (u[k] + step[p]).clamp(0.0, 1.0);
            }
            let rn = resid(&un);
            let cn = cost(&rn);
            if cn < c {
                let rel = (c - cn) / c.max(1e-300);
                u = un;
                r = rn;
                c = cn;
                lambda = (lambda * 0.3).max(1e-9);
                improved = true;
                if rel < 1e-7 {
                    return u;
                }
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    u
}

/// A stock preset as a stickshift config with creative white `cwp` and grey `grey`.
fn preset_config(base: &Config, look: usize, tonescale: usize, cwp: usize, grey: f32) -> Config {
    let p = Config::from_presets(&Presets { look, tonescale, ..Presets::default() }).expect("preset exists");
    let mut cfg = *base;
    cfg.look = p.look;
    cfg.look.cwp = cwp as f32;
    cfg.look.cwp_lm = 0.25;
    cfg.grey_nits = grey;
    cfg
}

fn describe(look: usize, tonescale: usize, cwp: usize) -> String {
    let tone = if tonescale == 0 { "its own".to_string() } else { TONESCALE_PRESET_LABELS[tonescale].to_string() };
    format!("{} look + {} tonescale, creative white {}", LOOK_PRESET_LABELS[look], tone, CWP_LABELS[cwp + 1])
}

/// Fit OpenDRT to `lut` (camera `input` → display `output`).
pub fn match_opendrt(
    lut: &dyn Transform,
    lut_name: Option<String>,
    input: InputSpace,
    output: OutputSpace,
    settings: &MatchSettings,
) -> Result<OpenDrtMatch, MatchError> {
    let t0 = Instant::now();
    let (dgamut, eotf, peak) = drt_display(&output)?;
    let nthreads = threads(settings.threads);
    let base = Config { display_gamut: dgamut, eotf, peak_nits: peak, surround: 0, clamp: true, ..Config::default() };

    // Samples inside the camera gamut and the LUT's input domain.
    let ring_id = settings.ring_gamut.unwrap_or(match input.id {
        CameraSpaceId::Aces2065Ap0 => CameraSpaceId::ArriLogC3Awg3,
        id => id,
    });
    let ring_space = InputSpace { id: ring_id, ..input };
    let colours = training_colours(&ring_space, &output);
    let to_cam = input.xyz_d65_to_rgb();
    let metric = settings.metric.resolve(output.transfer.is_hdr());
    let scorer = Scorer { metric, lab_white: whites::D65.to_xyz() * output.peak_nits() };
    let disp = output.rgb_to_xyz();
    let axis = GrayAxis::measure(lut, &input, &output, settings.interpolation);
    let corrector = axis.corrector(settings.neutral_gray, settings.neutral_amount, &output);
    let target = |code: &Vec3| {
        let x = disp * output.decode(&lut.apply_rgb(code, settings.interpolation));
        corrector.as_ref().map_or(x, |c| c.apply(&x))
    };
    let samples_total = colours.len();
    let samples: Vec<Sample> = colours
        .into_iter()
        .filter_map(|(x, w, gray)| {
            let cam = to_cam * x;
            let lut_in = input.encode(&cam);
            (cam.min() >= -1e-9 && lut.in_domain_rgb(&lut_in)).then(|| Sample {
                di: to_di(&x),
                target: target(&lut_in),
                weight: w,
                gray,
            })
        })
        .collect();
    if samples.len() < 40 {
        return Err(MatchError::TooFewSamples(samples.len()));
    }
    let samples_used = samples.len();

    // The LUT's 18% gray, as OpenDRT's Display Grey Luminance (defined at 100 nits).
    let gray = whites::D65.to_xyz() * 0.18;
    let gray_nits = target(&input.encode(&(to_cam * gray))).y;
    let grey0 = match output.transfer {
        DisplayTransfer::Pq { peak_nits } => gray_nits / (1.0 + 0.13 * (peak_nits / 100.0).log2()),
        _ => 100.0 * gray_nits / output.peak_nits(),
    }
    .clamp(GREY_MIN, GREY_MAX) as f32;

    let mut notes = Vec::new();
    if let Some(w) = axis.worst().filter(|w| w.duv > VISIBLE_DUV) {
        let at = format!("{:+} stops", w.stops);
        notes.push(match corrector {
            None => format!(
                "the LUT's gray axis is not neutral (largest cast Δu′v′ {:.4}, {}, at {at}); OpenDRT's gray is \
                 neutral, so the fit trades other colours to get closer. Neutralize Gray removes the cast first",
                w.duv,
                w.cast()
            ),
            Some(_) => format!(
                "gray-axis cast removed before matching ({}, {:.0}%; largest was Δu′v′ {:.4}, {}, at {at}); \
                 errors are against the corrected LUT",
                settings.neutral_gray.label(),
                100.0 * settings.neutral_amount.clamp(0.0, 1.0),
                w.duv,
                w.cast()
            ),
        });
    }

    let prob = Problem { samples, output, scorer, threads: nthreads };

    // 1. Preset search.
    let mut grid = Vec::new();
    for look in 0..LOOK_PRESET_LABELS.len() {
        for tone in 0..TONESCALE_PRESET_LABELS.len() {
            for &cwp in &settings.creative_whites {
                grid.push((look, tone, cwp));
            }
        }
    }
    let scored = parallel_map(&grid, prob.threads, |&(l, t, c)| prob.weighted(&preset_config(&base, l, t, c, grey0)));
    let mut ranked: Vec<(f64, PresetId)> = scored.into_iter().zip(grid).collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut starts: Vec<(f64, PresetId)> = Vec::new();
    for c in &ranked {
        if !starts.iter().any(|s| s.1 .0 == c.1 .0 && s.1 .1 == c.1 .1) {
            starts.push(*c);
        }
        if starts.len() == settings.starts.max(1) {
            break;
        }
    }

    let fit = |freeze: &[String]| -> (Config, (f64, PresetId)) {
        let vars = Vars::new(freeze);
        let mut best: Option<(f64, Config, (f64, PresetId))> = None;
        for st in &starts {
            let (l, t, c) = st.1;
            let mut cfg0 = preset_config(&base, l, t, c, grey0);
            for (en, strengths) in NEUTRAL {
                if cfg0.look.get(en) == Some(0.0) {
                    cfg0.look.set(en, 1.0);
                    for s in strengths {
                        cfg0.look.set(s, 0.0);
                    }
                }
            }
            let u0 = vars.to_unit(&cfg0);
            let sqrt_w: Vec<f64> = prob.samples.iter().map(|s| s.weight.sqrt()).collect();
            let reg = settings.regularization;
            let resid = |u: &[f64]| -> Vec<f64> {
                let mut r: Vec<f64> =
                    prob.errors(&vars.apply(&cfg0, u)).iter().zip(&sqrt_w).map(|(e, w)| e * w).collect();
                if reg > 0.0 {
                    r.extend(u.iter().zip(&u0).map(|(a, b)| reg * (a - b)));
                }
                r
            };
            let u = bounded_lm(u0.clone(), &resid, settings.max_iterations, prob.threads);
            let cfg = vars.apply(&cfg0, &u);
            let s = prob.weighted(&cfg);
            if best.as_ref().map_or(true, |b| s < b.0) {
                best = Some((s, cfg, *st));
            }
        }
        let b = best.expect("at least one start");
        (b.1, b.2)
    };

    let mut frozen = settings.freeze.clone();
    let (mut cfg, mut start) = fit(&frozen);
    let mut safety = safety_checks(&cfg, &ring_space, &output);
    if !safety.reversals.is_empty() && settings.auto_freeze_brilliance && !frozen.iter().any(|f| f == "brl") {
        notes.push(format!(
            "luminance fell with rising exposure for {} colour(s); refitted with brilliance frozen",
            safety.reversals.len()
        ));
        frozen.push("brl".into());
        (cfg, start) = fit(&frozen);
        safety = safety_checks(&cfg, &ring_space, &output);
    }
    if !safety.reversals.is_empty() {
        notes.push(format!("luminance still falls with exposure for {} colour(s)", safety.reversals.len()));
    }
    if !safety.finite_in_range {
        notes.push("non-finite or out-of-range output found".into());
    }
    if safety.roughness > 1.5 * safety.stock_roughness {
        notes.push(format!(
            "sharper transitions than the stock looks (max second difference {:.3} vs {:.3}); \
             usually bright saturated colours beyond the LUT's data. Check in Resolve, or refit \
             with regularisation / frozen modules",
            safety.roughness, safety.stock_roughness
        ));
    }

    let (l, t, c) = start.1;
    let stock = prob.score(&preset_config(&base, 0, 11, 2, 10.0));
    let best_preset = prob.score(&preset_config(&base, l, t, c, grey0));
    let fitted = prob.score(&cfg);
    Ok(OpenDrtMatch {
        lut_name,
        input,
        output,
        metric,
        config: cfg,
        start: Start { look: l, tonescale: t, creative_white: c, description: describe(l, t, c), score: start.0 },
        stock,
        best_preset,
        fitted,
        samples_used,
        samples_total,
        frozen,
        gray_axis: axis.whole_stops(),
        neutral_gray: settings.neutral_gray,
        neutral_amount: settings.neutral_amount,
        safety,
        notes,
        seconds: t0.elapsed().as_secs_f64(),
    })
}

fn parallel_map<T: Sync, R: Send + Default + Clone>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let mut out = vec![R::default(); items.len()];
    let chunk = items.len().div_ceil(threads.max(1)).max(1);
    std::thread::scope(|sc| {
        for (o, it) in out.chunks_mut(chunk).zip(items.chunks(chunk)) {
            let f = &f;
            sc.spawn(move || {
                for (a, b) in o.iter_mut().zip(it) {
                    *a = f(b);
                }
            });
        }
    });
    out
}

/// Output luminance (cd/m²) of OpenDRT for scene XYZ.
fn drt_nits(drt: &OpenDrt, output: &OutputSpace, xyz: &Vec3) -> f64 {
    let o = drt.apply(to_di(xyz));
    let c = Vec3::new(o[0] as f64, o[1] as f64, o[2] as f64);
    (output.rgb_to_xyz() * output.decode(&c)).y
}

/// Luminance reversals over an exposure sweep (−8…+8 stops in ⅛ stops) of gray and of
/// 36 hues (0.5 and 0.95 of the camera ∩ display chroma limit; 0.8, 0.95 and 1 of the
/// camera limit) taken at 18% gray and scaled with exposure.
pub fn exposure_reversals(cfg: &Config, ring_space: &InputSpace, output: &OutputSpace) -> Vec<Reversal> {
    let drt = OpenDrt::new(cfg);
    let mut colours = vec![("gray".to_string(), whites::D65.to_xyz() * 0.18)];
    for (limit, sats, what) in [
        (RingLimit::CameraAndDisplay, vec![0.5, 0.95], "display"),
        (RingLimit::Camera, vec![0.8, 0.95, 1.0], "camera"),
    ] {
        let st = Settings { hues: 36, saturations: sats, ring_limit: limit, ..Settings::default() };
        for (k, x) in hue_ring(&st, ring_space, output, 0.18) {
            if let crate::sample::SampleKind::Hue { hue_deg, saturation } = k {
                colours.push((format!("hue {hue_deg:.0}°, {saturation} of {what} gamut limit"), x));
            }
        }
    }
    let mut out = Vec::new();
    for (name, x) in colours {
        let mut prev: Option<(f64, f64)> = None;
        let mut worst: Option<(f64, f64)> = None;
        for k in 0..=128 {
            let s = -8.0 + k as f64 / 8.0;
            let y = drt_nits(&drt, output, &(x * 2f64.powf(s)));
            if let Some((ps, py)) = prev {
                // Allow float noise and the flat top of a clipped output.
                if y < py * (1.0 - 2e-3) - 1e-4 {
                    let drop = 100.0 * (py - y) / py;
                    if worst.map_or(true, |w| drop > w.1) {
                        worst = Some((ps, drop));
                    }
                }
            }
            prev = Some((s, y));
        }
        if let Some((at, drop)) = worst {
            out.push(Reversal { colour: name, at_stops: at, drop_percent: drop });
        }
    }
    out
}

/// Largest second difference of output code along the lines of an n³ input cube (DaVinci
/// Intermediate), over inputs inside `ring_space`'s gamut and at most 6 stops over 18%
/// gray (the training range), ignoring channels that clip.
fn roughness(cfg: &Config, ring_space: &InputSpace, n: usize) -> f64 {
    let drt = OpenDrt::new(cfg);
    let to_cam = ring_space.xyz_d65_to_rgb() * davinci::dwg_to_xyz();
    let v = |i: usize| i as f64 / (n - 1) as f64;
    let mut cube = vec![([0f32; 3], false); n * n * n];
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                let lin = Vec3::new(v(r), v(g), v(b)).map(di_decode);
                let real = (to_cam * lin).min() >= 0.0 && lin.max() <= 0.18 * 64.0;
                cube[(r * n + g) * n + b] = (drt.apply([v(r) as f32, v(g) as f32, v(b) as f32]), real);
            }
        }
    }
    let at = |r: usize, g: usize, b: usize| cube[(r * n + g) * n + b];
    let mut worst = 0f64;
    for a in 0..n {
        for b in 0..n {
            for i in 1..n - 1 {
                for (p, q, s) in [
                    (at(i - 1, a, b), at(i, a, b), at(i + 1, a, b)),
                    (at(a, i - 1, b), at(a, i, b), at(a, i + 1, b)),
                    (at(a, b, i - 1), at(a, b, i), at(a, b, i + 1)),
                ] {
                    if !(p.1 && q.1 && s.1) {
                        continue;
                    }
                    for c in 0..3 {
                        // A channel clipping at 0 or 1 is a kink in every look; skip it.
                        if [p.0[c], q.0[c], s.0[c]].iter().any(|x| *x <= 1e-6 || *x >= 1.0 - 1e-6) {
                            continue;
                        }
                        worst = worst.max((p.0[c] - 2.0 * q.0[c] + s.0[c]).abs() as f64);
                    }
                }
            }
        }
    }
    worst
}

fn safety_checks(cfg: &Config, ring_space: &InputSpace, output: &OutputSpace) -> Safety {
    let drt = OpenDrt::new(cfg);
    let n = 17;
    let mut finite_in_range = true;
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                let v = |i: usize| -0.05 + 1.1 * i as f32 / (n - 1) as f32;
                let o = drt.apply([v(r), v(g), v(b)]);
                finite_in_range &= o.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x));
            }
        }
    }
    let stock_roughness = (0..LOOK_PRESET_LABELS.len())
        .map(|l| roughness(&preset_config(cfg, l, 0, 2, cfg.grey_nits), ring_space, 33))
        .fold(0.0, f64::max);
    Safety {
        finite_in_range,
        reversals: exposure_reversals(cfg, ring_space, output),
        roughness: roughness(cfg, ring_space, 33),
        stock_roughness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lut::FnTransform;
    use crate::spaces::DisplayTransfer;

    fn rec709() -> OutputSpace {
        OutputSpace::new(DisplayGamut::Rec709, DisplayTransfer::Gamma { gamma: 2.4 })
    }

    /// Stock looks pass the exposure-sweep check (so the check is not too strict).
    #[test]
    fn stock_looks_have_no_reversals() {
        let space = InputSpace::new(CameraSpaceId::ArriLogC3Awg3);
        for (look, name) in LOOK_PRESET_LABELS.iter().enumerate() {
            let cfg = Config::from_presets(&Presets { look, ..Presets::default() }).unwrap();
            let r = exposure_reversals(&cfg, &space, &rec709());
            assert!(r.is_empty(), "{name}: {r:?}");
        }
    }

    /// A "LUT" that *is* an OpenDRT preset (Sylvan look, Umbra tonescale, 12 nits grey)
    /// fed DWG/DI: the search must find it and the fit must reproduce it.
    #[test]
    fn recovers_an_opendrt_preset() {
        let mut want = Config::from_presets(&Presets { look: 2, tonescale: 9, ..Presets::default() }).unwrap();
        want.grey_nits = 12.0;
        let drt = OpenDrt::new(&want);
        let lut = FnTransform(move |c: &Vec3| {
            let o = drt.apply([c.x as f32, c.y as f32, c.z as f32]);
            Vec3::new(o[0] as f64, o[1] as f64, o[2] as f64)
        });
        let input = InputSpace::new(CameraSpaceId::DaVinciIntermediateDwg);
        let s = MatchSettings { max_iterations: 20, starts: 1, ..MatchSettings::default() };
        let m = match_opendrt(&lut, None, input, rec709(), &s).unwrap();
        assert_eq!((m.start.look, m.start.tonescale, m.start.creative_white), (2, 9, 2), "{}", m.start.description);
        assert!(m.best_preset.mean < 0.05, "preset itself: {:?}", m.best_preset);
        assert!(m.fitted.mean <= m.best_preset.mean + 1e-6, "{:?}", m.fitted);
        assert!((m.config.grey_nits - 12.0).abs() < 0.05, "grey {}", m.config.grey_nits);
        assert!(m.safety.ok(), "{:?}", m.safety);
    }

    /// The same preset with a display tint that grows with level (a film-print-like
    /// cast): Neutralize Gray (every level) gives back the untinted preset, so the
    /// search finds it again and the fit is far closer than matching the cast.
    #[test]
    fn neutral_gray_removes_a_level_dependent_cast() {
        let want = Config::from_presets(&Presets { look: 2, tonescale: 9, ..Presets::default() }).unwrap();
        let drt = OpenDrt::new(&want);
        let out = rec709();
        let lut = FnTransform(move |c: &Vec3| {
            let o = drt.apply([c.x as f32, c.y as f32, c.z as f32]);
            let lin = out.decode(&Vec3::new(o[0] as f64, o[1] as f64, o[2] as f64)) / out.peak_nits();
            let t = 1.0 + 0.06 * lin.y.sqrt();
            out.encode(&(Vec3::new(lin.x * t, lin.y * 1.01, lin.z / t).map(|v| v.min(1.0)) * out.peak_nits()))
        });
        let input = InputSpace::new(CameraSpaceId::DaVinciIntermediateDwg);
        let base = MatchSettings { max_iterations: 20, starts: 1, ..MatchSettings::default() };
        let off = match_opendrt(&lut, None, input, rec709(), &base).unwrap();
        let on = MatchSettings { neutral_gray: NeutralGray::Exposure, ..base };
        let m = match_opendrt(&lut, None, input, rec709(), &on).unwrap();
        assert!(off.notes.iter().any(|n| n.contains("not neutral")), "{:?}", off.notes);
        assert!(m.notes.iter().any(|n| n.contains("removed")), "{:?}", m.notes);
        assert_eq!((m.start.look, m.start.tonescale, m.start.creative_white), (2, 9, 2), "{}", m.start.description);
        assert!(m.best_preset.mean < 0.6, "{:?}", m.best_preset);
        assert!(m.fitted.mean < 0.5 * off.fitted.mean, "on {:?} off {:?}", m.fitted, off.fitted);
        assert_eq!(m.neutral_gray, NeutralGray::Exposure);
        assert!(m.gray_axis.iter().any(|p| p.duv > VISIBLE_DUV));
    }
}
