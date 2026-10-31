//! Colour chart matching: find a ColorChecker in a still, sample it, work out how the
//! still is encoded, and fit a correction that takes the camera's chart to the reference
//! chart (or to another camera's chart), exported as a DWG/DI DCTL or an OpenDRT preset.
//!
//! - [`image`]: reading TIFF / OpenEXR / PNG / JPEG / DPX as stored values.
//! - [`detect`]: chart location, orientation, patch sampling, clip levels.
//! - [`reference`]: the published patch values.
//! - [`encoding`]: the still's curve and gamut (detected or chosen).
//! - [`fit`]: matrix (and tone) correction; [`dctl`]: its DCTL; [`highlights`]: the
//!   DCTL's highlight protection.
//! - [`odrt`]: an OpenDRT look preset fitted to the chart.

pub mod dctl;
pub mod detect;
pub mod encoding;
pub mod fit;
pub mod highlights;
pub mod image;
pub mod odrt;
pub mod reference;
pub mod tps;

use serde::{Deserialize, Serialize};

use self::detect::{clip_levels, find_lattice, placements, sample_patch, ClipLevels, Lattice, Orientation, PatchSample, Placement};
use self::encoding::{guess_space, SpaceGuess, StillSpace};
use self::fit::{fit_chart, ChartFit, FitPatch, FitSettings};
use self::highlights::{HighlightSettings, Highlights};
use self::image::{ChartImage, ImageInfo};
use self::reference::{ChartEdition, NEUTRALS, PATCH_NAMES};
use crate::difference::xyz_to_lab;
use crate::fit::least_squares;
use crate::math::{whites, Vec3};
use crate::spaces::display::DisplayGamut;

/// What the camera's chart should be corrected to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Target {
    /// The published values of the chart.
    #[default]
    Reference,
    /// Another still's chart, as measured (a "hero" camera): `xyz` in chart order, at
    /// that still's exposure; `used` marks its usable patches.
    Chart { name: String, xyz: Vec<[f64; 3]>, used: Vec<bool> },
}

/// Everything the analysis can be told.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartSettings {
    pub edition: ChartEdition,
    /// The still's encoding; None = detected.
    pub space: Option<StillSpace>,
    /// Forced orientation; None = the placement that fits best.
    pub orientation: Option<Orientation>,
    /// Manual placement: centres of dark skin (1), bluish green (6), black (24) and white
    /// (19), image pixels. Replaces detection.
    pub corners: Option<[[f64; 2]; 4]>,
    /// Width of the sampled square, fraction of the patch pitch.
    pub sample_size: f64,
    /// Patches (0-based chart index) the user left out.
    pub exclude: Vec<usize>,
    /// A channel counts as clipped when more than this fraction of its pixels are.
    pub clip_fraction: f64,
    /// Consider mirrored placements in detection.
    pub mirrors: bool,
    pub fit: FitSettings,
    pub target: Target,
    /// Highlight protection in the DCTL.
    pub highlights: HighlightSettings,
    /// The DCTL's white balance checkbox starts on (off: footage white-balanced in the
    /// raw decode).
    pub white_balance: bool,
}

impl Default for ChartSettings {
    fn default() -> Self {
        ChartSettings {
            edition: ChartEdition::After2014,
            space: None,
            orientation: None,
            corners: None,
            sample_size: 0.4,
            exclude: Vec::new(),
            clip_fraction: 0.02,
            mirrors: false,
            fit: FitSettings::default(),
            target: Target::Reference,
            highlights: HighlightSettings::default(),
            white_balance: true,
        }
    }
}

/// One patch as reported.
#[derive(Debug, Clone, Serialize)]
pub struct PatchReport {
    pub index: usize,
    pub name: &'static str,
    /// Centre and the corners of the sampled square, image pixels.
    pub centre: [f64; 2],
    pub sample_box: [[f64; 2]; 4],
    pub sample: PatchSample,
    pub clipped_low: [bool; 3],
    pub clipped_high: [bool; 3],
    /// Brighter neutral not brighter than the next darker one (highlight roll-off).
    pub rolled_off: [bool; 3],
    pub excluded: bool,
    pub used: bool,
    /// Why it is not used.
    pub reason: Option<String>,
    /// Measured, CIE XYZ D65.
    pub xyz: [f64; 3],
    /// Target, CIE XYZ D65 at the target's exposure.
    pub target: [f64; 3],
    pub corrected: [f64; 3],
    pub de_before: f64,
    pub de_after: f64,
    /// sRGB display swatches (8-bit), all at the target's exposure.
    pub swatch_measured: [u8; 3],
    pub swatch_target: [u8; 3],
    pub swatch_corrected: [u8; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct PlacementScore {
    pub orientation: Orientation,
    /// Mean ΔE76 of a quick least-squares matrix fit (lower = more chart-like).
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChartAnalysis {
    pub file_name: String,
    pub image: ImageInfo,
    pub placement: Placement,
    /// "detected" or "manual".
    pub placement_source: String,
    /// Patches found by detection (0 for manual).
    pub detected_patches: usize,
    pub pitch_px: f64,
    /// Every orientation considered, best first.
    pub alternatives: Vec<PlacementScore>,
    pub clip: ClipLevels,
    pub space: StillSpace,
    pub space_detected: bool,
    pub guess: SpaceGuess,
    pub edition: ChartEdition,
    pub target_name: String,
    pub patches: Vec<PatchReport>,
    pub fit: Option<ChartFit>,
    pub fit_error: Option<String>,
    /// Highlight protection for the DCTL, with the clip level resolved.
    pub highlights: Highlights,
    /// The DCTL's white balance checkbox starts on.
    pub white_balance: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChartError {
    #[error("no colour chart found; place it by clicking the corner patches")]
    NotFound,
    #[error("no placement of the chart in that orientation fits the patches found")]
    NoPlacement,
    #[error("the four corner points don't form a chart (are two of them the same?)")]
    BadCorners,
}

struct Sampled {
    samples: Vec<PatchSample>,
    codes: [[f64; 3]; 24],
    usable: [[bool; 3]; 24],
    low: [[bool; 3]; 24],
    high: [[bool; 3]; 24],
    rolled: [[bool; 3]; 24],
}

fn sample_all(img: &ChartImage, p: &Placement, s: &ChartSettings, clip: &ClipLevels) -> Sampled {
    let samples: Vec<PatchSample> = (0..24).map(|i| sample_patch(img, p, i % 6, i / 6, s.sample_size, clip)).collect();
    let codes: [[f64; 3]; 24] = std::array::from_fn(|i| samples[i].mean);
    let low: [[bool; 3]; 24] = std::array::from_fn(|i| std::array::from_fn(|k| samples[i].n == 0 || samples[i].low_clip[k] > s.clip_fraction));
    let high: [[bool; 3]; 24] = std::array::from_fn(|i| std::array::from_fn(|k| samples[i].high_clip[k] > s.clip_fraction));
    let mut rolled = [[false; 3]; 24];
    // Neutral row must get darker from white to black in every channel; among unclipped
    // neutrals, a brighter one that isn't brighter is compressed (upper half: the
    // brighter patch; lower half: the darker one).
    for k in 0..3 {
        let ok: Vec<usize> = NEUTRALS.iter().copied().filter(|&i| !low[i][k] && !high[i][k] && codes[i][k].is_finite()).collect();
        for w in ok.windows(2) {
            let (a, b) = (w[0], w[1]);
            if codes[a][k] <= codes[b][k] {
                let pos = NEUTRALS.iter().position(|&n| n == a).unwrap_or(0);
                if pos < 3 {
                    rolled[a][k] = true;
                } else {
                    rolled[b][k] = true;
                }
            }
        }
    }
    let usable = std::array::from_fn(|i| std::array::from_fn(|k| !low[i][k] && !high[i][k] && !rolled[i][k] && codes[i][k].is_finite()));
    Sampled { samples, codes, usable, low, high, rolled }
}

/// Quick plausibility of a placement: mean ΔE76 after a least-squares matrix in XYZ
/// (weighted 1/Y) with exposure matched, plus a penalty for each brightness inversion
/// along the neutral row.
fn placement_score(img: &ChartImage, p: &Placement, s: &ChartSettings, clip: &ClipLevels, refs: &[Vec3; 24]) -> f64 {
    let sm = sample_all(img, p, s, clip);
    let ref_y: [f64; 24] = std::array::from_fn(|i| refs[i].y);
    let space = s.space.unwrap_or_else(|| guess_space(&img.file_name, &img.info, &sm.codes, &sm.usable, &ref_y).space);
    let used: Vec<usize> = (0..24).filter(|&i| sm.usable[i].iter().all(|u| *u)).collect();
    if used.len() < 6 {
        return 1e3 - used.len() as f64;
    }
    let x: Vec<Vec3> = used.iter().map(|&i| space.to_xyz(&sm.codes[i])).collect();
    let y: Vec<Vec3> = used.iter().map(|&i| refs[i]).collect();
    let wt: Vec<f64> = used.iter().map(|&i| 1.0 / refs[i].y.max(1e-3)).collect();
    let Some(m) = least_squares(&x, &y, Some(&wt)) else { return 1e3 };
    let wn = whites::D65.to_xyz();
    let mean = x
        .iter()
        .zip(&y)
        .map(|(a, b)| (xyz_to_lab(&(m * a).map(|v| v.max(0.0)), &wn) - xyz_to_lab(b, &wn)).norm())
        .sum::<f64>()
        / x.len() as f64;
    let lum = |i: usize| sm.codes[i].iter().sum::<f64>();
    let inversions = NEUTRALS.windows(2).filter(|w| lum(w[0]) <= lum(w[1])).count();
    mean + 5.0 * inversions as f64
}

/// sRGB 8-bit for display of an XYZ (D65) reflectance-scale colour.
pub fn srgb8(xyz: &Vec3) -> [u8; 3] {
    let rgb = DisplayGamut::Rec709.primaries().xyz_to_rgb() * xyz;
    let enc = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        let e = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (e * 255.0).round() as u8
    };
    [enc(rgb.x), enc(rgb.y), enc(rgb.z)]
}

/// Analyse a still. `lattice` is the detection result for this image (cache it: it only
/// depends on the pixels); None runs detection.
pub fn analyze(img: &ChartImage, lattice: Option<&Lattice>, s: &ChartSettings) -> Result<ChartAnalysis, ChartError> {
    let clip = clip_levels(img);
    let refs = s.edition.xyz_d65();
    let mut notes = Vec::new();
    let (placement, source, detected, alternatives) = if let Some(c) = s.corners {
        let p = Placement::from_corners(c).ok_or(ChartError::BadCorners)?;
        if !p.h.iter().all(|v| v.is_finite()) || p.pitch() < 3.0 {
            return Err(ChartError::BadCorners);
        }
        (p, "manual", 0, Vec::new())
    } else {
        let owned;
        let lat = match lattice {
            Some(l) => l,
            None => {
                owned = find_lattice(img).ok_or(ChartError::NotFound)?;
                &owned
            }
        };
        let cands = placements(lat, s.mirrors || s.orientation.is_some_and(|o| o.mirrored()));
        let cands: Vec<Placement> = cands.into_iter().filter(|p| s.orientation.is_none_or(|o| o == p.orientation)).collect();
        if cands.is_empty() {
            return Err(ChartError::NoPlacement);
        }
        let mut scored: Vec<(f64, Placement)> =
            cands.into_iter().map(|p| (placement_score(img, &p, s, &clip, &refs) * if p.orientation.mirrored() { 1.15 } else { 1.0 }, p)).collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut alts: Vec<PlacementScore> = Vec::new();
        for (sc, p) in &scored {
            if !alts.iter().any(|a| a.orientation == p.orientation) {
                alts.push(PlacementScore { orientation: p.orientation, score: *sc });
            }
        }
        if alts.len() > 1 && alts[1].score < alts[0].score * 1.25 && s.orientation.is_none() {
            notes.push(format!(
                "orientation is uncertain ({} fits almost as well); check the numbers on the patches",
                alts[1].orientation.label()
            ));
        }
        if lat.members.len() < 12 {
            notes.push(format!("only {} patches were found directly; check the placement", lat.members.len()));
        }
        (scored[0].1, "detected", lat.members.len(), alts)
    };

    let sm = sample_all(img, &placement, s, &clip);
    let ref_y: [f64; 24] = std::array::from_fn(|i| refs[i].y);
    let guess = guess_space(&img.file_name, &img.info, &sm.codes, &sm.usable, &ref_y);
    let space = s.space.unwrap_or(guess.space);

    // Targets.
    let (target_xyz, target_used, target_name): (Vec<Vec3>, Vec<bool>, String) = match &s.target {
        Target::Reference => (refs.to_vec(), vec![true; 24], s.edition.label().to_string()),
        Target::Chart { name, xyz, used } if xyz.len() == 24 && used.len() == 24 => {
            (xyz.iter().map(|v| Vec3::from(*v)).collect(), used.clone(), format!("chart in {name}"))
        }
        Target::Chart { .. } => (refs.to_vec(), vec![true; 24], s.edition.label().to_string()),
    };

    let xyz: Vec<Vec3> = sm.codes.iter().map(|c| space.to_xyz(c)).collect();
    let mut reasons: Vec<Option<String>> = vec![None; 24];
    let chans = |f: &[bool; 3]| ["R", "G", "B"].iter().zip(f).filter(|(_, b)| **b).map(|(c, _)| *c).collect::<Vec<_>>().join("");
    for i in 0..24 {
        reasons[i] = if s.exclude.contains(&i) {
            Some("left out".into())
        } else if sm.samples[i].n == 0 {
            Some("outside the image".into())
        } else if sm.low[i].iter().any(|b| *b) {
            Some(format!("{} clipped at the bottom", chans(&sm.low[i])))
        } else if sm.high[i].iter().any(|b| *b) {
            Some(format!("{} clipped at the top", chans(&sm.high[i])))
        } else if sm.rolled[i].iter().any(|b| *b) {
            Some(format!("{} not in order with the other grays (rolled off)", chans(&sm.rolled[i])))
        } else if !target_used[i] {
            Some("not usable in the target chart".into())
        } else if !xyz[i].iter().all(|v| v.is_finite()) {
            Some("decodes to a non-finite value".into())
        } else {
            None
        };
    }
    let patches: Vec<FitPatch> = (0..24)
        .map(|i| FitPatch { xyz: xyz[i], target: target_xyz[i], neutral: i >= 18, used: reasons[i].is_none() })
        .collect();
    let clipped = (0..24).filter(|&i| sm.low[i].iter().chain(&sm.high[i]).any(|b| *b)).count();
    if clipped > 0 {
        notes.push(format!(
            "{clipped} patch{} clipped in the file and left out{}",
            if clipped == 1 { " is" } else { "es are" },
            if !img.info.float && (0..24).any(|i| sm.low[i].iter().any(|b| *b)) {
                "; an integer file can't store values below 0: export a half-float EXR to keep them"
            } else {
                ""
            }
        ));
    }
    let (fit, fit_error) = match fit_chart(&patches, &s.fit) {
        Ok(f) => (Some(f), None),
        Err(e) => (None, Some(e)),
    };
    if let Some(f) = &fit {
        notes.extend(f.notes.iter().cloned());
    }

    // Display: everything at the target's exposure.
    let ratio = fit.as_ref().map_or(1.0, |f| f.exposure_ratio);
    let keep = s.fit.exposure == fit::Exposure::Keep;
    let reports = (0..24)
        .map(|i| {
            let c = placement.project((i % 6) as f64, (i / 6) as f64);
            let h = s.sample_size / 2.0;
            let (cc, rr) = ((i % 6) as f64, (i / 6) as f64);
            let corr = fit.as_ref().map_or(xyz[i], |f| f.apply(&xyz[i]));
            let corr_disp = if keep { corr / ratio } else { corr };
            PatchReport {
                index: i,
                name: PATCH_NAMES[i],
                centre: c,
                sample_box: [
                    placement.project(cc - h, rr - h),
                    placement.project(cc + h, rr - h),
                    placement.project(cc + h, rr + h),
                    placement.project(cc - h, rr + h),
                ],
                sample: sm.samples[i],
                clipped_low: sm.low[i],
                clipped_high: sm.high[i],
                rolled_off: sm.rolled[i],
                excluded: s.exclude.contains(&i),
                used: reasons[i].is_none(),
                reason: reasons[i].clone(),
                xyz: xyz[i].into(),
                target: target_xyz[i].into(),
                corrected: corr.into(),
                de_before: fit.as_ref().map_or(f64::NAN, |f| f.de_before[i]),
                de_after: fit.as_ref().map_or(f64::NAN, |f| f.de_after[i]),
                swatch_measured: srgb8(&(xyz[i] / ratio)),
                swatch_target: srgb8(&target_xyz[i]),
                swatch_corrected: srgb8(&corr_disp),
            }
        })
        .collect();

    Ok(ChartAnalysis {
        file_name: img.file_name.clone(),
        image: img.info.clone(),
        pitch_px: placement.pitch(),
        placement,
        placement_source: source.into(),
        detected_patches: detected,
        alternatives,
        clip,
        space,
        space_detected: s.space.is_none(),
        guess,
        edition: s.edition,
        target_name,
        patches: reports,
        fit,
        fit_error,
        highlights: Highlights::resolve(&s.highlights, &clip, &space),
        white_balance: s.white_balance,
        notes,
    })
}

impl ChartAnalysis {
    /// This chart as a target for another still (its measured colours and exposure).
    pub fn as_target(&self) -> Target {
        Target::Chart {
            name: self.file_name.clone(),
            xyz: self.patches.iter().map(|p| p.xyz).collect(),
            used: self.patches.iter().map(|p| p.used).collect(),
        }
    }

    /// Header text for the DCTL.
    pub fn dctl_meta(&self, date: &str) -> Option<dctl::ChartMeta> {
        let f = self.fit.as_ref()?;
        Some(dctl::ChartMeta {
            source_image: self.file_name.clone(),
            image_space: self.space.name(),
            target: self.target_name.clone(),
            model: match f.settings.model {
                fit::Model::Matrix => "3x3 matrix in XYZ".to_string(),
                fit::Model::MatrixTone => "per-channel tone curve in linear DWG + 3x3 matrix in XYZ".to_string(),
                fit::Model::Tps => format!(
                    "TPS3D thin-plate spline in DI DWG, no matrix ({} patches, smoothing {}), gray-corrected",
                    match f.settings.tps_patches {
                        fit::TpsPatches::All => "colour patches and grays",
                        fit::TpsPatches::Primaries => "6 primaries and grays",
                    },
                    f.settings.tps_smoothing
                ),
            } + if f.settings.model == fit::Model::Tps {
                ""
            } else if f.settings.preserve_neutrals {
                ", neutrals preserved after the white balance (equal RGB stays equal)"
            } else if f.settings.lock_neutral {
                ", neutral locked"
            } else {
                ""
            },
            exposure: match f.settings.exposure {
                fit::Exposure::Match => format!(
                    "matched to the target (the still was {:+.2} stops from it; neutral gain x{:.3})",
                    f.exposure_ratio.log2(),
                    f.neutral_gain
                ),
                fit::Exposure::Keep => format!("kept (neutral gain x{:.3})", f.neutral_gain),
            },
            fit_error: format!(
                "dE2000 mean {:.2} (before {:.2}), max {:.2}, neutrals {:.2}; {} of 24 patches used",
                f.mean_after, f.mean_before, f.max_after, f.neutral_after, f.used
            ),
            version: crate::VERSION.into(),
            date: date.into(),
        })
    }

    pub fn dctl(&self, date: &str) -> Option<String> {
        Some(dctl::chart_dctl(self.fit.as_ref()?, &self.highlights, self.white_balance, &self.dctl_meta(date)?))
    }
}

/// How a preview is shown on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PreviewDisplay {
    /// Rec.709 / sRGB with a soft highlight roll-off.
    #[default]
    Simple,
    /// OpenDRT (DWG/DI in) with this look and tonescale preset, Rec.1886 out: how the
    /// footage looks after "chart DCTL → OpenDRT".
    OpenDrt { look: usize, tonescale: usize },
}

/// The correction a preview shows (the DCTL as exported, `Amount` = 1).
#[derive(Debug, Clone, Copy)]
pub struct PreviewCorrection<'a> {
    pub fit: &'a ChartFit,
    pub highlights: &'a Highlights,
    pub white_balance: bool,
}

/// A display preview of the still (JPEG bytes, width, height): decoded with `space`,
/// exposure scaled by 1/`exposure_ratio` so the chart's grays sit at their reflectance,
/// a soft highlight roll-off, sRGB. At most `max_side` pixels across.
pub fn preview_jpeg(img: &ChartImage, space: &StillSpace, exposure_ratio: f64, max_side: usize) -> (Vec<u8>, usize, usize) {
    preview_jpeg_with(img, space, exposure_ratio, None, PreviewDisplay::Simple, max_side)
}

/// [`preview_jpeg`] with the correction applied (if any) and shown on `display`. The
/// correction runs on the still's own values (so highlight protection sees the real
/// clip level); before and after are shown at the same exposure (the target's).
pub fn preview_jpeg_with(
    img: &ChartImage,
    space: &StillSpace,
    exposure_ratio: f64,
    correction: Option<PreviewCorrection>,
    display: PreviewDisplay,
    max_side: usize,
) -> (Vec<u8>, usize, usize) {
    use crate::spaces::davinci::{di_encode, xyz_to_dwg};
    let step = img.width.max(img.height).div_ceil(max_side).max(1);
    let (w, h) = (img.width / step, img.height / step);
    let to709 = DisplayGamut::Rec709.primaries().xyz_to_rgb();
    let ratio = exposure_ratio.max(1e-9);
    // Corrected values are at the target's exposure unless the fit keeps the still's.
    let scale = match correction {
        Some(c) if c.fit.settings.exposure == fit::Exposure::Match => 1.0,
        _ => 1.0 / ratio,
    };
    let drt = match display {
        PreviewDisplay::OpenDrt { look, tonescale } => {
            let p = lutmatrix_opendrt::Presets { look, tonescale, ..Default::default() };
            Some(lutmatrix_opendrt::OpenDrt::new(&lutmatrix_opendrt::Config::from_presets(&p).unwrap_or_default()))
        }
        PreviewDisplay::Simple => None,
    };
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let simple = |v: f64| {
        let v = v.max(0.0);
        let v = v * (1.0 + v / 4.0) / (1.0 + v);
        byte(if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 })
    };
    let pixel = |x: usize, y: usize| -> [u8; 3] {
        let p = img.pixel(x * step + step / 2, y * step + step / 2);
        let xyz = space.to_xyz(&[p[0] as f64, p[1] as f64, p[2] as f64]);
        let xyz = match correction {
            Some(c) => c.highlights.apply(c.fit, &(xyz_to_dwg() * xyz), c.white_balance),
            None => xyz,
        } * scale;
        if !xyz.iter().all(|v| v.is_finite()) {
            return [0; 3];
        }
        match &drt {
            Some(d) => {
                let di = (xyz_to_dwg() * xyz).map(di_encode);
                let o = d.apply([di.x as f32, di.y as f32, di.z as f32]);
                [byte(o[0] as f64), byte(o[1] as f64), byte(o[2] as f64)]
            }
            None => {
                let rgb = to709 * xyz;
                [simple(rgb.x), simple(rgb.y), simple(rgb.z)]
            }
        }
    };
    let mut out = vec![0u8; w * h * 3];
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    let rows = h.div_ceil(threads).max(1);
    std::thread::scope(|sc| {
        for (k, chunk) in out.chunks_mut(rows * w * 3).enumerate() {
            let pixel = &pixel;
            sc.spawn(move || {
                for (i, px) in chunk.chunks_mut(3).enumerate() {
                    let (x, y) = (i % w, k * rows + i / w);
                    px.copy_from_slice(&pixel(x, y));
                }
            });
        }
    });
    let mut buf = Vec::new();
    let _ = ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 88).encode(&out, w as u32, h as u32, ::image::ExtendedColorType::Rgb8);
    (buf, w, h)
}

/// Standard base64 (for data: URLs).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for ch in bytes.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if ch.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if ch.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
