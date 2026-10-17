//! End-to-end analysis: measure → fit (LS + perceptual) → report + diagnostics.

use serde::Serialize;

use crate::difference::{Metric, Scorer};
use crate::fit::{least_squares, least_squares_constrained, refine, Param};
use crate::lut::{Lut, Transform};
use crate::math::{whites, Mat3, Vec3, Xy};
use crate::sample::{
    measure, AnalysisError, Measurement, Pipeline, SampleKind, SampleStatus, Settings, WhiteMode,
};
use crate::spaces::display::BT2408_REF_WHITE_NITS;
use crate::spaces::gamuts::REC709;
use crate::spaces::{InputSpace, OutputSpace};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HueError {
    pub hue_deg: f64,
    pub name: &'static str,
    pub mean: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FitStats {
    pub metric: Metric,
    pub n: usize,
    pub mean: f64,
    pub rms: f64,
    pub p95: f64,
    pub max: f64,
    /// Mean error over the neutral samples.
    pub neutral: f64,
    /// Chromaticity error of the neutral axis, Δu′v′ between M·D65 and the target white.
    pub neutral_duv: f64,
    /// Hues with the largest mean error, worst first.
    pub worst_hues: Vec<HueError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SampleReport {
    pub kind: SampleKind,
    pub stops: f64,
    pub status: SampleStatus,
    pub error_ls: Option<f64>,
    pub error_refined: Option<f64>,
    /// sRGB hex swatches, each normalised so its own gray sits at 18%:
    /// the scene colour (no colour change), the LUT output, the fit target
    /// (= LUT output, white-balanced in D65 mode) and the matrix prediction.
    pub swatch_input: String,
    pub swatch_lut: String,
    pub swatch_target: String,
    pub swatch_matrix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Analysis {
    pub version: &'static str,
    pub input: InputSpace,
    pub input_name: String,
    pub output: OutputSpace,
    pub output_name: String,
    pub settings: Settings,
    pub metric: Metric,

    /// The matrix to export: refined if `settings.refine`, else least squares.
    pub m_xyz: Mat3,
    pub m_xyz_ls: Mat3,
    pub m_xyz_refined: Option<Mat3>,
    pub stats_ls: FitStats,
    pub stats_refined: Option<FitStats>,

    /// CAT applied to the LUT output in D65 mode (measured LUT white → D65).
    pub white_adaptation: Option<Mat3>,
    /// Chromaticity of the LUT's rendering of scene neutral gray.
    pub lut_white_xy: Xy,
    pub lut_white_duv_from_d65: f64,

    pub anchor_scene_y: f64,
    pub anchor_stops: f64,
    pub display_middle_gray_nits: f64,
    pub display_middle_gray_source: String,
    pub gray_out_nits: f64,
    pub stops_from_display_middle_gray: f64,
    /// d log2(Y_out) / d log2(Y_in) at the anchor (scene → display contrast).
    pub contrast_at_gray: f64,

    pub samples_used: usize,
    pub samples_skipped: Vec<(SampleStatus, usize)>,
    pub samples: Vec<SampleReport>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn uv_prime(xyz: &Vec3) -> (f64, f64) {
    let d = xyz.x + 15.0 * xyz.y + 3.0 * xyz.z;
    (4.0 * xyz.x / d, 9.0 * xyz.y / d)
}

fn duv(a: &Vec3, b: &Vec3) -> f64 {
    let (ua, va) = uv_prime(a);
    let (ub, vb) = uv_prime(b);
    (ua - ub).hypot(va - vb)
}

/// Approximate colour name for a CIELAB hue angle.
pub fn hue_name(h: f64) -> &'static str {
    let h = h.rem_euclid(360.0);
    match h {
        h if h < 15.0 => "magenta-red",
        h if h < 45.0 => "red",
        h if h < 70.0 => "orange",
        h if h < 100.0 => "yellow",
        h if h < 125.0 => "yellow-green",
        h if h < 170.0 => "green",
        h if h < 215.0 => "cyan",
        h if h < 250.0 => "azure",
        h if h < 290.0 => "blue",
        h if h < 325.0 => "purple",
        _ => "magenta",
    }
}

/// XYZ (any scale) → sRGB hex, scaled so `gray_y` maps to 18%.
fn swatch(xyz: &Vec3, gray_y: f64) -> String {
    let rgb = REC709.xyz_to_rgb() * (xyz * (0.18 / gray_y));
    let enc = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        let e = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (e * 255.0).round() as u8
    };
    format!("#{:02x}{:02x}{:02x}", enc(rgb.x), enc(rgb.y), enc(rgb.z))
}

/// Run the full analysis for one LUT and one input/output pairing.
pub fn analyze(
    lut: &Lut,
    input: InputSpace,
    output: OutputSpace,
    settings: &Settings,
    middle_gray_hint: Option<f64>,
) -> Result<Analysis, AnalysisError> {
    if lut.cube.is_none() && lut.shaper.is_none() {
        return Err(AnalysisError::EmptyLut);
    }
    analyze_transform(lut, input, output, settings, middle_gray_hint)
}

/// As [`analyze`], for any [`Transform`].
pub fn analyze_transform(
    lut: &dyn Transform,
    input: InputSpace,
    output: OutputSpace,
    settings: &Settings,
    middle_gray_hint: Option<f64>,
) -> Result<Analysis, AnalysisError> {
    let m = measure(lut, input, output, settings, middle_gray_hint)?;
    let d65 = whites::D65.to_xyz();
    let lvl0 = m.level_index(0.0);
    let k0 = m.k[lvl0];

    // ---- white handling ----------------------------------------------------
    // Normalised LUT white (what scene gray maps to, in fit units).
    let lut_white = m.gray_out_xyz * k0;
    let white_adaptation = match settings.white_mode {
        WhiteMode::PreserveLut => None,
        WhiteMode::D65 => Some(settings.cat.matrix_xyz(&lut_white, &(d65 * lut_white.y))),
    };
    let adapt = white_adaptation.unwrap_or_else(Mat3::identity);

    // Fit data (usable samples only), in normalised units.
    let used: Vec<usize> = (0..m.samples.len()).filter(|&i| m.samples[i].usable()).collect();
    let k_of = |i: usize| m.k[m.level_index(m.samples[i].stops)];
    let xs: Vec<Vec3> = used.iter().map(|&i| m.samples[i].xyz_in).collect();
    let ys: Vec<Vec3> = used.iter().map(|&i| adapt * (m.samples[i].probe.xyz_out * k_of(i))).collect();

    // Neutral target per unit input white.
    let neutral_gain = lut_white.y / m.anchor_scene_y;
    let neutral_target = match settings.white_mode {
        WhiteMode::PreserveLut => lut_white / m.anchor_scene_y,
        WhiteMode::D65 => d65 * neutral_gain,
    };

    let nw = settings
        .neutral_weight
        .unwrap_or((settings.hues * settings.saturations.len()) as f64)
        .max(0.0);
    let wts: Vec<f64> = used
        .iter()
        .map(|&i| if matches!(m.samples[i].kind, SampleKind::Neutral) { nw } else { 1.0 })
        .collect();
    let sqrt_w: Vec<f64> = wts.iter().map(|w| w.sqrt()).collect();

    let constrained = settings.white_mode == WhiteMode::D65 || settings.lock_neutral;
    let m_ls = if constrained {
        least_squares_constrained(&xs, &ys, Some(&wts), &d65, &neutral_target)
    } else {
        least_squares(&xs, &ys, Some(&wts))
    }
    .ok_or(AnalysisError::TooFewSamples(xs.len()))?;

    // ---- perceptual scoring -------------------------------------------------
    let metric = settings.metric.resolve(output.transfer.is_hdr());
    let y_ref = if output.transfer.is_hdr() { BT2408_REF_WHITE_NITS } else { output.sdr_peak_nits };
    let scorer = Scorer { metric, lab_white: d65 * y_ref };
    let ks: Vec<f64> = used.iter().map(|&i| k_of(i)).collect();
    // Unweighted per-sample errors (for reporting) …
    let errors = |mm: &Mat3| -> Vec<f64> {
        xs.iter()
            .zip(&ys)
            .zip(&ks)
            .map(|((x, y), k)| scorer.delta(&(mm * x / *k), &(y / *k)))
            .collect()
    };
    // … and weighted residuals (for the optimiser).
    let residuals = |mm: &Mat3| -> Vec<f64> {
        errors(mm).into_iter().zip(&sqrt_w).map(|(e, w)| e * w).collect()
    };

    let m_refined = settings.refine.then(|| {
        let param = if constrained { Param::Neutral { w: d65 } } else { Param::Free };
        refine(&m_ls, &param, &residuals)
    });

    let stats_for = |mm: &Mat3| -> (FitStats, Vec<f64>) {
        let errs = errors(mm);
        let stats = summarise(&m, &used, &errs, metric, &(mm * d65), &neutral_target);
        (stats, errs)
    };
    let (stats_ls, errs_ls) = stats_for(&m_ls);
    let refined = m_refined.as_ref().map(stats_for);
    let m_xyz = m_refined.unwrap_or(m_ls);

    // ---- per-sample report --------------------------------------------------
    let mut err_ls = vec![None; m.samples.len()];
    let mut err_rf = vec![None; m.samples.len()];
    for (j, &i) in used.iter().enumerate() {
        err_ls[i] = Some(errs_ls[j]);
        if let Some((_, e)) = &refined {
            err_rf[i] = Some(e[j]);
        }
    }
    let samples: Vec<SampleReport> = m
        .samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let li = m.level_index(s.stops);
            let k = m.k[li];
            let gray_scene = m.anchor_scene_y * 2f64.powf(s.stops);
            let gray_disp = m.gray_out_nits[li];
            let target = adapt * s.probe.xyz_out;
            SampleReport {
                kind: s.kind,
                stops: s.stops,
                status: s.probe.status,
                error_ls: err_ls[i],
                error_refined: err_rf[i],
                swatch_input: swatch(&s.xyz_in, gray_scene),
                swatch_lut: swatch(&s.probe.xyz_out, gray_disp),
                swatch_target: swatch(&target, gray_disp),
                swatch_matrix: swatch(&(m_xyz * s.xyz_in / k), gray_disp),
            }
        })
        .collect();

    let mut skipped: Vec<(SampleStatus, usize)> = Vec::new();
    for st in [SampleStatus::OutsideCameraGamut, SampleStatus::OutsideLutDomain, SampleStatus::ClippedOutput] {
        let n = m.samples.iter().filter(|s| s.probe.status == st).count();
        if n > 0 {
            skipped.push((st, n));
        }
    }

    // ---- diagnostics --------------------------------------------------------
    let p = Pipeline::new(lut, input, output, settings);
    let contrast = {
        let h = 0.25;
        let a = p.gray_nits(m.anchor_scene_y * 2f64.powf(-h)).max(1e-9);
        let b = p.gray_nits(m.anchor_scene_y * 2f64.powf(h)).max(1e-9);
        (b / a).log2() / (2.0 * h)
    };
    let lut_white_xy = Xy::from_xyz(&m.gray_out_xyz);
    let lut_white_duv = duv(&m.gray_out_xyz, &d65);
    let final_stats = refined.as_ref().map(|r| &r.0).unwrap_or(&stats_ls);
    let diagnostics = diagnose(&DiagInput {
        p: &p,
        m: &m,
        settings,
        input: &input,
        output: &output,
        contrast,
        lut_white_duv,
        lut_white_xy,
        stats: final_stats,
        used: used.len(),
        total: m.samples.len(),
    });

    Ok(Analysis {
        version: crate::VERSION,
        input,
        input_name: input.name(),
        output,
        output_name: output.name(),
        settings: settings.clone(),
        metric,
        m_xyz,
        m_xyz_ls: m_ls,
        m_xyz_refined: m_refined,
        stats_ls,
        stats_refined: refined.map(|r| r.0),
        white_adaptation,
        lut_white_xy,
        lut_white_duv_from_d65: lut_white_duv,
        anchor_scene_y: m.anchor_scene_y,
        anchor_stops: m.anchor_stops,
        display_middle_gray_nits: m.display_middle_gray_nits,
        display_middle_gray_source: m.display_middle_gray_source.clone(),
        gray_out_nits: m.gray_out_nits[lvl0],
        stops_from_display_middle_gray: m.stops_from_display_middle_gray,
        contrast_at_gray: contrast,
        samples_used: used.len(),
        samples_skipped: skipped,
        samples,
        diagnostics,
    })
}

fn summarise(
    m: &Measurement,
    used: &[usize],
    errs: &[f64],
    metric: Metric,
    neutral_pred: &Vec3,
    neutral_target: &Vec3,
) -> FitStats {
    let n = errs.len();
    let mean = errs.iter().sum::<f64>() / n as f64;
    let rms = (errs.iter().map(|e| e * e).sum::<f64>() / n as f64).sqrt();
    let mut sorted = errs.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = sorted[((n as f64 * 0.95).ceil() as usize).clamp(1, n) - 1];
    let max = *sorted.last().unwrap();

    let mut neutral = Vec::new();
    let mut by_hue: Vec<(f64, Vec<f64>)> = Vec::new();
    for (j, &i) in used.iter().enumerate() {
        match m.samples[i].kind {
            SampleKind::Neutral => neutral.push(errs[j]),
            SampleKind::Hue { hue_deg, .. } => match by_hue.iter_mut().find(|(h, _)| *h == hue_deg) {
                Some((_, v)) => v.push(errs[j]),
                None => by_hue.push((hue_deg, vec![errs[j]])),
            },
        }
    }
    let mut worst: Vec<HueError> = by_hue
        .into_iter()
        .map(|(h, v)| HueError {
            hue_deg: h,
            name: hue_name(h),
            mean: v.iter().sum::<f64>() / v.len() as f64,
            max: v.iter().cloned().fold(0.0, f64::max),
        })
        .collect();
    worst.sort_by(|a, b| b.mean.partial_cmp(&a.mean).unwrap());
    worst.truncate(5);

    FitStats {
        metric,
        n,
        mean,
        rms,
        p95,
        max,
        neutral: if neutral.is_empty() { 0.0 } else { neutral.iter().sum::<f64>() / neutral.len() as f64 },
        neutral_duv: duv(neutral_pred, neutral_target),
        worst_hues: worst,
    }
}

struct DiagInput<'a> {
    p: &'a Pipeline<'a>,
    m: &'a Measurement,
    settings: &'a Settings,
    input: &'a InputSpace,
    output: &'a OutputSpace,
    contrast: f64,
    lut_white_duv: f64,
    lut_white_xy: Xy,
    stats: &'a FitStats,
    used: usize,
    total: usize,
}

/// Output chroma (u′v′ distance from that exposure's gray) and hue angle, for one
/// scene colour at one exposure. None if it clips.
fn uv_polar(p: &Pipeline, xyz: &Vec3, gray: &Vec3) -> Option<(f64, f64)> {
    let pr = p.run(xyz);
    let pg = p.run(gray);
    if pr.status != SampleStatus::Ok || pg.status != SampleStatus::Ok {
        return None;
    }
    let (u, v) = uv_prime(&pr.xyz_out);
    let (ug, vg) = uv_prime(&pg.xyz_out);
    Some(((u - ug).hypot(v - vg), (v - vg).atan2(u - ug).to_degrees()))
}

fn diagnose(d: &DiagInput) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut info = |code, message: String| out.push(Diagnostic { severity: Severity::Info, code, message });
    let metric = d.stats.metric.label();

    // Exposure placement.
    info(
        "exposure",
        format!(
            "Scene middle gray ({:.3}) renders at {:.2} cd/m², {:+.2} stops from display middle gray ({:.2} cd/m², {}).",
            d.settings.scene_middle_gray,
            d.p.gray_nits(d.settings.scene_middle_gray),
            d.m.stops_from_display_middle_gray,
            d.m.display_middle_gray_nits,
            d.m.display_middle_gray_source
        ),
    );
    if d.m.anchor_stops != 0.0 {
        info(
            "anchor",
            format!(
                "Sampling at {:+.2} stops (scene Y = {:.4}), where the LUT outputs display middle gray.",
                d.m.anchor_stops, d.m.anchor_scene_y
            ),
        );
    }
    info(
        "contrast",
        format!("Tone-curve contrast at middle gray: {:.2} (log–log slope, scene → display).", d.contrast),
    );
    info(
        "lut_white",
        format!(
            "The LUT renders neutral gray at x {:.4}, y {:.4} (Δu′v′ {:.4} from D65){}.",
            d.lut_white_xy.x,
            d.lut_white_xy.y,
            d.lut_white_duv,
            match d.settings.white_mode {
                WhiteMode::PreserveLut => "; preserved in the matrix",
                WhiteMode::D65 => "; removed (D65 white mode)",
            }
        ),
    );

    let mut warn = Vec::new();
    // Exposure behaviour of colour: probe ±1 stop around the anchor.
    let ring = crate::sample::hue_ring(
        &Settings { hues: 12, saturations: vec![0.5], ..d.settings.clone() },
        d.input,
        d.output,
        d.m.anchor_scene_y,
    );
    let gray = whites::D65.to_xyz() * d.m.anchor_scene_y;
    let (mut sat_changes, mut hue_shifts) = (Vec::new(), Vec::new());
    for (_, xyz) in &ring {
        let lo = uv_polar(d.p, &(xyz * 0.5), &(gray * 0.5));
        let mid = uv_polar(d.p, xyz, &gray);
        let hi = uv_polar(d.p, &(xyz * 2.0), &(gray * 2.0));
        if let (Some(lo), Some(mid), Some(hi)) = (lo, mid, hi) {
            if mid.0 > 1e-4 {
                sat_changes.push((hi.0 - lo.0) / 2.0 / mid.0);
                let dh = ((hi.1 - lo.1 + 540.0) % 360.0) - 180.0;
                hue_shifts.push(dh.abs() / 2.0);
            }
        }
    }
    if !sat_changes.is_empty() {
        let s = sat_changes.iter().sum::<f64>() / sat_changes.len() as f64;
        let h = hue_shifts.iter().sum::<f64>() / hue_shifts.len() as f64;
        let msg = format!(
            "Across ±1 stop, LUT saturation changes {:+.0}% per stop and hues rotate {:.1}° per stop on average.",
            s * 100.0,
            h
        );
        if s.abs() > 0.10 || h > 3.0 {
            warn.push((
                "exposure_dependent_colour",
                format!(
                    "{msg} The tone curve changes colour with exposure (typical of per-channel curves), so a single matrix only matches around the sampled exposure."
                ),
            ));
        } else {
            out.push(Diagnostic { severity: Severity::Info, code: "exposure_dependent_colour", message: msg });
        }
    }
    // Neutral tint vs exposure.
    let g0 = d.p.run(&gray).xyz_out;
    let tint = [0.5, 2.0]
        .iter()
        .map(|f| duv(&d.p.run(&(gray * *f)).xyz_out, &g0))
        .fold(0.0, f64::max);
    if tint > 0.003 {
        warn.push((
            "neutral_drift",
            format!("The LUT's neutral tint changes with exposure (Δu′v′ up to {tint:.4} at ±1 stop); the matrix captures the tint at the sampled exposure only."),
        ));
    }
    if d.contrast > 2.0 || d.contrast < 0.5 {
        warn.push((
            "extreme_contrast",
            format!("Tone-curve contrast at middle gray is {:.2}; colour measured here may not represent other exposures.", d.contrast),
        ));
    }
    // ΔE ITP reads ~5× ΔE2000 around middle gray on real LUTs (DECISIONS.md D19).
    let limit = if d.stats.metric == Metric::DeltaEItp { 15.0 } else { 3.0 };
    if d.stats.mean > limit {
        warn.push((
            "poor_fit",
            format!(
                "Mean {metric} {:.2} is high: the LUT's colour transform is not well approximated by a 3×3 matrix (worst: {}).",
                d.stats.mean,
                d.stats.worst_hues.first().map(|h| h.name).unwrap_or("-")
            ),
        ));
    }
    let skipped = d.total - d.used;
    if skipped * 4 > d.total {
        warn.push((
            "many_skipped",
            format!("{skipped} of {} samples were skipped (clipping or out of range); reduce saturation levels or check the spaces.", d.total),
        ));
    }
    if !d.settings.normalize_exposure && d.m.stops_from_display_middle_gray.abs() > 0.1 {
        warn.push((
            "gain_in_matrix",
            format!(
                "Exposure normalisation is off, so the matrix includes a {:+.2}-stop gain.",
                d.m.stops_from_display_middle_gray
            ),
        ));
    }
    for (code, message) in warn {
        out.push(Diagnostic { severity: Severity::Warning, code, message });
    }
    out
}
