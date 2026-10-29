//! An OpenDRT look preset fitted to a chart.
//!
//! The target is what stock OpenDRT (a chosen look, tonescale and display) shows for the
//! target chart: the published patches, or another camera's chart. The preset is
//! OpenDRT's look parameters, refined (from that look) so that OpenDRT shows the camera's
//! chart the same way. Optionally the chart is first corrected by the fitted matrix, for
//! "matrix DCTL → OpenDRT preset" in that order.
//!
//! A chart has 24 patches, OpenDRT ~50 look parameters: the fit is pulled towards the
//! starting look (regularisation) and leaves alone what a chart can't constrain (the
//! highlight shoulder and contrast modules, creative white). With "fit tone" the
//! tonescale's contrast, toe and offset are fitted too, so a camera's different tone is
//! matched as well. Display Grey Luminance carries the exposure when the exposure is
//! matched. The usual safety checks run on the result (DECISIONS.md D40).

use std::time::Instant;

use lutmatrix_opendrt::tables::{LOOK_PRESET_LABELS, TONESCALE_PRESET_LABELS};
use lutmatrix_opendrt::{Config, OpenDrt, Presets};
use serde::{Deserialize, Serialize};

use super::fit::Exposure;
use super::{srgb8, ChartAnalysis};
use crate::difference::{Metric, Scorer};
use crate::math::{whites, Vec3};
use crate::opendrt_match::{bounded_lm, safety_checks, threads, to_di, Safety, Vars, NEUTRAL};
use crate::spaces::{CameraSpaceId, DisplayGamut, DisplayTransfer, InputSpace, OutputSpace};

/// Display encodings offered for chart presets: (OpenDRT Display Encoding Preset index,
/// label, gamut, gamma).
pub const DISPLAYS: [(usize, &str, DisplayGamut, f64); 4] = [
    (0, "Rec.1886 - 2.4 Power / Rec.709", DisplayGamut::Rec709, 2.4),
    (1, "sRGB Display - 2.2 Power / Rec.709", DisplayGamut::Rec709, 2.2),
    (2, "Display P3 - 2.2 Power / P3-D65", DisplayGamut::P3D65, 2.2),
    (9, "DCI - 2.6 Power / P3-D65", DisplayGamut::P3D65, 2.6),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OdrtChartSettings {
    /// Look Preset to start from and to match (0 = Standard).
    pub look: usize,
    /// Tonescale Preset of the target (0 = the look's own).
    pub tonescale: usize,
    /// Display Encoding Preset (one of [`DISPLAYS`]).
    pub display_encoding: usize,
    /// Fit tonescale contrast, toe and offset as well.
    pub fit_tone: bool,
    /// Run the chart through the fitted matrix first (matrix DCTL before OpenDRT).
    pub after_matrix: bool,
    /// Pull towards the starting look, per parameter in slider-range units.
    pub regularization: f64,
    pub max_iterations: usize,
    pub threads: usize,
}

impl Default for OdrtChartSettings {
    fn default() -> Self {
        OdrtChartSettings {
            look: 0,
            tonescale: 0,
            display_encoding: 0,
            fit_tone: true,
            after_matrix: true,
            regularization: 6.0,
            max_iterations: 80,
            threads: 0,
        }
    }
}

/// Per-patch and summary errors (ΔE2000, display referred).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdrtScore {
    pub mean: f64,
    pub max: f64,
    pub neutral: f64,
    pub per_patch: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdrtChartFit {
    pub source: String,
    pub target: String,
    /// The starting / target look, as text.
    pub base: String,
    pub settings: OdrtChartSettings,
    /// The fitted OpenDRT settings (look parameters + Display Grey Luminance at 100 nits).
    pub config: Config,
    /// Stock look on the camera's chart vs the target.
    pub before: OdrtScore,
    pub after: OdrtScore,
    pub used: Vec<bool>,
    /// sRGB display swatches: target, stock look on the camera chart, preset.
    pub swatch_target: Vec<[u8; 3]>,
    pub swatch_before: Vec<[u8; 3]>,
    pub swatch_after: Vec<[u8; 3]>,
    pub frozen: Vec<String>,
    pub safety: Safety,
    pub notes: Vec<String>,
    pub seconds: f64,
}

/// Tonescale/shoulder modules a chart can't constrain (always frozen), and the tonescale
/// parameters frozen unless `fit_tone`.
/// The toe is frozen too: the chart's black patch is dominated by flare.
const ALWAYS_FROZEN: [&str; 5] = ["tn_sh", "tn_toe", "tn_hcon", "tn_lcon", "cwp_lm"];
const TONE: [&str; 2] = ["tn_con", "tn_off"];

pub fn fit_opendrt_chart(a: &ChartAnalysis, s: &OdrtChartSettings) -> Result<OdrtChartFit, String> {
    let t0 = Instant::now();
    let fit = a.fit.as_ref();
    if s.after_matrix && fit.is_none() {
        return Err("no matrix fit to put in front of OpenDRT".into());
    }
    let &(enc, enc_label, gamut, gamma) =
        DISPLAYS.iter().find(|d| d.0 == s.display_encoding).ok_or("unsupported display encoding")?;
    let output = OutputSpace::new(gamut, DisplayTransfer::Gamma { gamma });
    let presets = Presets { look: s.look, tonescale: s.tonescale, display_encoding: enc, ..Presets::default() };
    let base = Config::from_presets(&presets).ok_or("unknown look or tonescale preset")?;
    let disp = output.rgb_to_xyz();
    let scorer = Scorer { metric: Metric::DeltaE2000, lab_white: whites::D65.to_xyz() * output.peak_nits() };
    let to_nits = |drt: &OpenDrt, xyz: &Vec3| -> Vec3 {
        let o = drt.apply(to_di(xyz));
        disp * output.decode(&Vec3::new(o[0] as f64, o[1] as f64, o[2] as f64))
    };
    let ratio = fit.map_or(1.0, |f| f.exposure_ratio);
    let keep = a.fit.as_ref().map_or(Exposure::Match, |f| f.settings.exposure) == Exposure::Keep;
    // Inputs: the camera chart (optionally through the matrix); targets: stock OpenDRT of
    // the target chart, at the still's exposure when keeping it.
    let input: Vec<Vec3> = a
        .patches
        .iter()
        .map(|p| {
            let x = Vec3::from(p.xyz);
            match (s.after_matrix, fit) {
                (true, Some(f)) => f.apply(&x),
                _ => x,
            }
        })
        .collect();
    let base_drt = OpenDrt::new(&base);
    let k = if keep { ratio } else { 1.0 };
    let target: Vec<Vec3> = a.patches.iter().map(|p| to_nits(&base_drt, &(Vec3::from(p.target) * k))).collect();
    let used: Vec<bool> = a.patches.iter().map(|p| p.used).collect();
    let idx: Vec<usize> = (0..24).filter(|&i| used[i]).collect();
    if idx.len() < 6 {
        return Err(format!("only {} usable patches", idx.len()));
    }
    let weight = |i: usize| -> f64 { if i >= 18 { 2.0 } else { 1.0 } };

    let score = |cfg: &Config| -> OdrtScore {
        let drt = OpenDrt::new(cfg);
        let per: Vec<f64> = (0..24).map(|i| scorer.delta(&to_nits(&drt, &input[i]), &target[i])).collect();
        let mean = idx.iter().map(|&i| per[i]).sum::<f64>() / idx.len() as f64;
        let max = idx.iter().map(|&i| per[i]).fold(0.0, f64::max);
        let n: Vec<f64> = idx.iter().filter(|&&i| i >= 18).map(|&i| per[i]).collect();
        OdrtScore { mean, max, neutral: n.iter().sum::<f64>() / n.len().max(1) as f64, per_patch: per }
    };

    // Exposure: when matching it, Display Grey Luminance absorbs the still's exposure.
    // Start from the grey that best matches the neutrals (golden-section search).
    let mut cfg0 = base;
    let fit_grey = !keep || s.fit_tone;
    if fit_grey {
        let neutral_err = |g: f64| {
            let mut c = base;
            c.grey_nits = g as f32;
            let drt = OpenDrt::new(&c);
            idx.iter().filter(|&&i| i >= 18).map(|&i| scorer.delta(&to_nits(&drt, &input[i]), &target[i]).powi(2)).sum::<f64>()
        };
        let (mut lo, mut hi) = (3.0f64, 25.0f64);
        let phi = 0.618_033_988_75;
        for _ in 0..40 {
            let (a1, b1) = (hi - phi * (hi - lo), lo + phi * (hi - lo));
            if neutral_err(a1) < neutral_err(b1) {
                hi = b1;
            } else {
                lo = a1;
            }
        }
        cfg0.grey_nits = ((lo + hi) / 2.0) as f32;
    }
    for (en, strengths) in NEUTRAL {
        if cfg0.look.get(en) == Some(0.0) {
            cfg0.look.set(en, 1.0);
            for st in strengths {
                cfg0.look.set(st, 0.0);
            }
        }
    }
    let mut frozen: Vec<String> = ALWAYS_FROZEN.iter().map(|s| s.to_string()).collect();
    if !s.fit_tone {
        frozen.extend(TONE.iter().map(|s| s.to_string()));
    }
    if !fit_grey {
        frozen.push("grey".into());
    }
    let nthreads = threads(s.threads);
    let ring_space = InputSpace::new(CameraSpaceId::ArriLogC3Awg3);
    let run = |frozen: &[String]| -> Config {
        let vars = Vars::new(frozen);
        let u0 = vars.to_unit(&cfg0);
        let reg = s.regularization;
        let resid = |u: &[f64]| -> Vec<f64> {
            let cfg = vars.apply(&cfg0, u);
            let drt = OpenDrt::new(&cfg);
            let mut r = Vec::with_capacity(idx.len() * 3 + u.len());
            for &i in &idx {
                let c = scorer.components(&to_nits(&drt, &input[i]), &target[i]);
                let w = weight(i);
                r.extend(c.iter().map(|v| if v.is_finite() { v * w.sqrt() } else { 1e3 }));
            }
            r.extend(u.iter().zip(&u0).map(|(a, b)| reg * (a - b)));
            r
        };
        let u = bounded_lm(u0.clone(), &resid, s.max_iterations, nthreads);
        vars.apply(&cfg0, &u)
    };
    let mut notes = Vec::new();
    let mut cfg = run(&frozen);
    let mut safety = safety_checks(&cfg, &ring_space, &output);
    if !safety.reversals.is_empty() && !frozen.iter().any(|f| f == "brl") {
        notes.push(format!("luminance fell with rising exposure for {} colour(s); refitted with brilliance frozen", safety.reversals.len()));
        frozen.push("brl".into());
        cfg = run(&frozen);
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
            "sharper transitions than the stock looks (max second difference {:.3} vs {:.3}); raise the regularisation",
            safety.roughness, safety.stock_roughness
        ));
    }
    let mut before_cfg = base;
    before_cfg.grey_nits = cfg0.grey_nits;
    let before = score(&before_cfg);
    let after = score(&cfg);
    let peak = output.peak_nits();
    let sw = |c: &Config| -> Vec<[u8; 3]> {
        let drt = OpenDrt::new(c);
        input.iter().map(|x| srgb8(&(to_nits(&drt, x) / peak))).collect()
    };
    let base_desc = format!(
        "{} look, {} tonescale, {}",
        LOOK_PRESET_LABELS.get(s.look).copied().unwrap_or("?"),
        if s.tonescale == 0 { "its own".to_string() } else { TONESCALE_PRESET_LABELS.get(s.tonescale).copied().unwrap_or("?").to_string() },
        enc_label
    );
    Ok(OdrtChartFit {
        source: a.file_name.clone(),
        target: a.target_name.clone(),
        base: base_desc,
        settings: s.clone(),
        config: cfg,
        before,
        after,
        used,
        swatch_target: target.iter().map(|t| srgb8(&(t / peak))).collect(),
        swatch_before: sw(&before_cfg),
        swatch_after: sw(&cfg),
        frozen,
        safety,
        notes,
        seconds: t0.elapsed().as_secs_f64(),
    })
}
