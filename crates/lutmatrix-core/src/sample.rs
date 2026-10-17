//! Test-colour generation and measurement through the LUT.
//!
//! Scene side: CIE XYZ relative to D65 with Y = 1 for a perfect white, so the 18%
//! gray card has Y = 0.18. Display side: absolute CIE XYZ in cd/m².

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::difference::{lab_to_xyz, Metric};
use crate::lut::{Interpolation, Transform};
use crate::math::{whites, Cat, Mat3, Vec3};
use crate::spaces::display::default_display_middle_gray_nits;
use crate::spaces::{InputSpace, OutputSpace};

/// Where on the exposure scale to measure (DECISIONS.md D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GrayAnchor {
    /// A: sample at scene-linear middle gray (0.18 by default).
    #[default]
    SceneLinear,
    /// B: sample at the scene exposure the LUT maps to display middle gray.
    DisplayTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WhiteMode {
    /// Unconstrained fit: the LUT's neutral shift is baked into the matrix.
    #[default]
    PreserveLut,
    /// D65 neutral in → D65 neutral out (CAT on the LUT output + neutral constraint).
    D65,
}

/// Which gamut bounds the hue ring's chroma.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RingLimit {
    /// Inside both the camera gamut and the display gamut (default): measures colours
    /// the display can show, not the LUT's gamut compression.
    #[default]
    CameraAndDisplay,
    /// Inside the camera gamut only.
    Camera,
}

/// Every user-facing knob of the analysis. Serialisable so the app can remember it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub interpolation: Interpolation,
    pub anchor: GrayAnchor,
    /// Scene-linear middle gray (reflectance).
    pub scene_middle_gray: f64,
    /// Display middle gray in cd/m²; `None` = default for the output (BT.2408-based),
    /// or the LUT's "DW" hint when detection found one.
    pub display_middle_gray_nits: Option<f64>,
    pub hues: usize,
    /// Fractions of the per-hue maximum chroma.
    pub saturations: Vec<f64>,
    pub ring_limit: RingLimit,
    /// Exposure bracket ±stops around the anchor; 0 = single exposure.
    pub bracket_stops: f64,
    pub normalize_exposure: bool,
    pub white_mode: WhiteMode,
    pub cat: Cat,
    pub metric: Metric,
    /// Refine the least-squares matrix by minimising the perceptual metric.
    pub refine: bool,
    /// Weight of each neutral sample relative to a colour sample. `None` = the number
    /// of colours in one ring, so gray counts as much as all hues together.
    pub neutral_weight: Option<f64>,
    /// Preserve mode only: constrain the matrix to reproduce the LUT's measured white
    /// exactly (M·D65 = LUT white) instead of an unconstrained fit. D65 mode always
    /// constrains.
    pub lock_neutral: bool,
    /// Output code at or below this counts as clipped (black clip).
    pub clip_low_code: f64,
    /// Output within this of the top code (1.0, or PQ(peak)) counts as clipped.
    pub clip_high_margin: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            interpolation: Interpolation::Tetrahedral,
            anchor: GrayAnchor::SceneLinear,
            scene_middle_gray: 0.18,
            display_middle_gray_nits: None,
            hues: 36,
            saturations: vec![0.25, 0.5, 0.75, 0.95],
            ring_limit: RingLimit::CameraAndDisplay,
            bracket_stops: 0.0,
            normalize_exposure: true,
            white_mode: WhiteMode::PreserveLut,
            cat: Cat::Bradford,
            metric: Metric::Auto,
            refine: true,
            neutral_weight: None,
            lock_neutral: false,
            clip_low_code: 0.0005,
            clip_high_margin: 0.001,
        }
    }
}

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("the LUT has no data")]
    EmptyLut,
    #[error("middle gray does not survive the LUT ({0}); check the input space and middle-gray settings")]
    GrayUnusable(String),
    #[error("too few usable samples ({0}); the LUT clips or the spaces are wrong")]
    TooFewSamples(usize),
    #[error("invalid setting: {0}")]
    Setting(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleStatus {
    Ok,
    OutsideCameraGamut,
    OutsideLutDomain,
    ClippedOutput,
}

impl SampleStatus {
    pub fn label(self) -> &'static str {
        match self {
            SampleStatus::Ok => "ok",
            SampleStatus::OutsideCameraGamut => "outside camera gamut",
            SampleStatus::OutsideLutDomain => "outside LUT input range",
            SampleStatus::ClippedOutput => "clipped on output",
        }
    }
}

/// Result of pushing one scene colour through camera encoding → LUT → display decode.
#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub camera_rgb: Vec3,
    pub lut_in: Vec3,
    pub lut_out: Vec3,
    /// Display-linear absolute XYZ, cd/m².
    pub xyz_out: Vec3,
    pub status: SampleStatus,
}

/// The measurement chain for one LUT and one input/output space pairing.
pub struct Pipeline<'a> {
    pub lut: &'a dyn Transform,
    pub input: InputSpace,
    pub output: OutputSpace,
    pub interp: Interpolation,
    xyz_to_cam: Mat3,
    disp_rgb_to_xyz: Mat3,
    top_code: f64,
    clip_low: f64,
    clip_high: f64,
}

impl<'a> Pipeline<'a> {
    pub fn new(lut: &'a dyn Transform, input: InputSpace, output: OutputSpace, s: &Settings) -> Self {
        let top_code = output.encode_channel(output.peak_nits()).min(1.0);
        Self {
            lut,
            input,
            output,
            interp: s.interpolation,
            xyz_to_cam: input.xyz_d65_to_rgb(),
            disp_rgb_to_xyz: output.rgb_to_xyz(),
            top_code,
            clip_low: s.clip_low_code,
            clip_high: top_code - s.clip_high_margin,
        }
    }

    pub fn top_code(&self) -> f64 {
        self.top_code
    }

    /// Scene XYZ (D65-relative) → display XYZ (cd/m²).
    pub fn run(&self, xyz_in: &Vec3) -> Probe {
        let cam = self.xyz_to_cam * xyz_in;
        let lut_in = self.input.encode(&cam);
        let lut_out = self.lut.apply_rgb(&lut_in, self.interp);
        let xyz_out = self.disp_rgb_to_xyz * self.output.decode(&lut_out);
        let status = if cam.min() < -1e-9 {
            SampleStatus::OutsideCameraGamut
        } else if !self.lut.in_domain_rgb(&lut_in) {
            SampleStatus::OutsideLutDomain
        } else if lut_out.max() >= self.clip_high || lut_out.min() <= self.clip_low {
            SampleStatus::ClippedOutput
        } else {
            SampleStatus::Ok
        };
        Probe { camera_rgb: cam, lut_in, lut_out, xyz_out, status }
    }

    /// Display luminance (cd/m²) of scene neutral gray at scene luminance `y`.
    pub fn gray_nits(&self, y: f64) -> f64 {
        self.run(&(whites::D65.to_xyz() * y)).xyz_out.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SampleKind {
    Neutral,
    Hue { hue_deg: f64, saturation: f64 },
}

#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub kind: SampleKind,
    /// Exposure offset (stops) from the anchor exposure.
    pub stops: f64,
    /// Scene XYZ, D65-relative (Y = 0.18 at middle gray).
    pub xyz_in: Vec3,
    pub probe: Probe,
}

impl Sample {
    pub fn usable(&self) -> bool {
        self.probe.status == SampleStatus::Ok
    }
}

/// All measurements for one analysis run.
#[derive(Debug, Clone, Serialize)]
pub struct Measurement {
    pub samples: Vec<Sample>,
    /// Bracket levels in stops, e.g. [-0.5, 0, 0.5].
    pub levels: Vec<f64>,
    /// Scene luminance (Y) of the anchor exposure's gray (0.18 in mode A).
    pub anchor_scene_y: f64,
    /// Stops of the anchor relative to scene middle gray (0 in mode A).
    pub anchor_stops: f64,
    /// Display middle gray used (cd/m²) and where that value came from.
    pub display_middle_gray_nits: f64,
    pub display_middle_gray_source: String,
    /// Display luminance of scene gray at each level (cd/m²).
    pub gray_out_nits: Vec<f64>,
    /// Display XYZ of scene gray at the anchor (cd/m²): the LUT's rendered white point.
    pub gray_out_xyz: Vec3,
    /// Exposure normalisation factor per level: target = k · display XYZ.
    pub k: Vec<f64>,
    /// Where the scene-middle-gray lands vs display middle gray (stops).
    pub stops_from_display_middle_gray: f64,
}

/// Resolve the display middle gray: explicit setting, then a detection hint, then default.
pub fn resolve_display_middle_gray(
    s: &Settings,
    output: &OutputSpace,
    hint: Option<f64>,
) -> (f64, String) {
    if let Some(v) = s.display_middle_gray_nits {
        (v, "set by user".into())
    } else if let Some(v) = hint {
        (v, "from the LUT's diffuse-white level in its name".into())
    } else if output.transfer.is_hdr() {
        (default_display_middle_gray_nits(output), "BT.2408-8 18% grey (26 cd/m²)".into())
    } else {
        (
            default_display_middle_gray_nits(output),
            "26/203 of SDR peak (BT.2408 gray/white ratio)".into(),
        )
    }
}

/// Maximum CIELAB chroma at hue `h` (degrees) and luminance `y` that stays inside the
/// limiting gamut(s).
fn max_chroma(h: f64, y: f64, inside: &dyn Fn(&Vec3) -> bool) -> f64 {
    let wn = whites::D65.to_xyz();
    let l = crate::difference::xyz_to_lab(&(wn * y), &wn).x;
    let (s, c) = h.to_radians().sin_cos();
    let at = |chroma: f64| lab_to_xyz(&Vec3::new(l, chroma * c, chroma * s), &wn);
    // March outwards, then bisect the first crossing.
    let mut lo = 0.0;
    let mut hi = 0.0;
    let step = 1.0;
    while hi < 400.0 {
        hi += step;
        if !inside(&at(hi)) {
            break;
        }
        lo = hi;
    }
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if inside(&at(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Generate the hue-ring colours at scene luminance `y` (XYZ, D65-relative).
pub fn hue_ring(
    s: &Settings,
    input: &InputSpace,
    output: &OutputSpace,
    y: f64,
) -> Vec<(SampleKind, Vec3)> {
    let wn = whites::D65.to_xyz();
    let l = crate::difference::xyz_to_lab(&(wn * y), &wn).x;
    let to_cam = input.xyz_d65_to_rgb();
    let to_disp = output.xyz_to_rgb();
    let limit = s.ring_limit;
    let inside = move |xyz: &Vec3| {
        (to_cam * xyz).min() >= 0.0 && (limit == RingLimit::Camera || (to_disp * xyz).min() >= 0.0)
    };
    let mut out = Vec::new();
    for i in 0..s.hues {
        let h = 360.0 * i as f64 / s.hues as f64;
        let cmax = max_chroma(h, y, &inside);
        let (sn, cs) = h.to_radians().sin_cos();
        for &sat in &s.saturations {
            let c = cmax * sat;
            let xyz = lab_to_xyz(&Vec3::new(l, c * cs, c * sn), &wn);
            out.push((SampleKind::Hue { hue_deg: h, saturation: sat }, xyz));
        }
    }
    out
}

/// Find the scene exposure (stops relative to `scene_gray`) whose gray the LUT maps to
/// `target_nits`. Bisection on a monotonic tone curve.
pub fn solve_anchor_exposure(p: &Pipeline, scene_gray: f64, target_nits: f64) -> Option<f64> {
    let f = |st: f64| (p.gray_nits(scene_gray * 2f64.powf(st)).max(1e-9) / target_nits).log2();
    let (mut lo, mut hi) = (-8.0, 8.0);
    let (flo, fhi) = (f(lo), f(hi));
    if flo > 0.0 || fhi < 0.0 {
        return None;
    }
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if f(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Generate samples and push them through the LUT.
pub fn measure(
    lut: &dyn Transform,
    input: InputSpace,
    output: OutputSpace,
    s: &Settings,
    middle_gray_hint: Option<f64>,
) -> Result<Measurement, AnalysisError> {
    if s.hues < 3 || s.saturations.is_empty() {
        return Err(AnalysisError::Setting("need at least 3 hues and one saturation level".into()));
    }
    if s.scene_middle_gray.is_nan() || s.scene_middle_gray <= 0.0 {
        return Err(AnalysisError::Setting("scene middle gray must be positive".into()));
    }
    let p = Pipeline::new(lut, input, output, s);
    let (disp_mid, disp_mid_src) = resolve_display_middle_gray(s, &output, middle_gray_hint);

    let mid_out = p.gray_nits(s.scene_middle_gray);
    let stops_from_display = (mid_out.max(1e-9) / disp_mid).log2();

    let anchor_stops = match s.anchor {
        GrayAnchor::SceneLinear => 0.0,
        GrayAnchor::DisplayTarget => solve_anchor_exposure(&p, s.scene_middle_gray, disp_mid).ok_or_else(|| {
            AnalysisError::GrayUnusable(format!(
                "no exposure within ±8 stops maps gray to {disp_mid:.1} cd/m²"
            ))
        })?,
    };
    let anchor_y = s.scene_middle_gray * 2f64.powf(anchor_stops);

    let levels: Vec<f64> = if s.bracket_stops > 0.0 {
        vec![-s.bracket_stops, 0.0, s.bracket_stops]
    } else {
        vec![0.0]
    };

    let ring = hue_ring(s, &input, &output, anchor_y);
    let d65 = whites::D65.to_xyz();
    let mut samples = Vec::with_capacity(levels.len() * (ring.len() + 1));
    let mut gray_out_nits = Vec::new();
    let mut k = Vec::new();
    let mut gray_out_xyz = Vec3::zeros();
    for &lv in &levels {
        let g = 2f64.powf(lv);
        let gray_in = d65 * anchor_y * g;
        let gp = p.run(&gray_in);
        if gp.status != SampleStatus::Ok {
            return Err(AnalysisError::GrayUnusable(format!(
                "gray at {lv:+} stops is {}",
                gp.status.label()
            )));
        }
        if lv == 0.0 {
            gray_out_xyz = gp.xyz_out;
        }
        gray_out_nits.push(gp.xyz_out.y);
        k.push(if s.normalize_exposure {
            gray_in.y / gp.xyz_out.y
        } else {
            s.scene_middle_gray / disp_mid
        });
        samples.push(Sample { kind: SampleKind::Neutral, stops: lv, xyz_in: gray_in, probe: gp });
        for (kind, xyz) in &ring {
            let xin = xyz * g;
            samples.push(Sample { kind: *kind, stops: lv, xyz_in: xin, probe: p.run(&xin) });
        }
    }

    let usable = samples.iter().filter(|s| s.usable()).count();
    if usable < 6 {
        return Err(AnalysisError::TooFewSamples(usable));
    }

    Ok(Measurement {
        samples,
        levels,
        anchor_scene_y: anchor_y,
        anchor_stops,
        display_middle_gray_nits: disp_mid,
        display_middle_gray_source: disp_mid_src,
        gray_out_nits,
        gray_out_xyz,
        k,
        stops_from_display_middle_gray: stops_from_display,
    })
}

impl Measurement {
    pub fn level_index(&self, stops: f64) -> usize {
        self.levels.iter().position(|&l| l == stops).unwrap_or(0)
    }
}
