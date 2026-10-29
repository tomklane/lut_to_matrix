//! What a chart still is encoded in: a transfer curve and a gamut, detected or chosen.
//!
//! A chart cannot reveal its gamut: any 3 × 3 matrix the fit finds absorbs a gamut
//! difference. It can show how the code values relate to light, though: in the right
//! decoding the neutral row is proportional to the patches' reflectance, so log2(decoded)
//! against log2(reflectance) has slope 1. Too steep means the curve decodes too
//! contrasty (e.g. a log file read as linear gives slopes of ~0.2…0.4; linear read as a
//! log curve, 2…6). Log curves differ from each other by much less than that, so the chart
//! tells linear from log reliably but not which log curve; that comes from the file name,
//! the EXR header or the user (DECISIONS.md D38).

use serde::{Deserialize, Serialize};

use super::image::ImageInfo;
use super::reference::NEUTRALS;
use crate::detect::Confidence;
use crate::math::{whites, Cat, Mat3, Primaries, Vec3};
use crate::spaces::curves::LogCurve;
use crate::spaces::gamuts;

/// Gamuts a still can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StillGamut {
    /// CIE XYZ itself (RGB = XYZ, white D65 = (0.9505, 1, 1.089), no adaptation), as
    /// Resolve's "CIE XYZ" colour space.
    Xyz,
    DaVinciWideGamut,
    AcesAp0,
    AcesAp1,
    ArriWideGamut3,
    ArriWideGamut4,
    SGamut3,
    SGamut3Cine,
    VGamut,
    RedWideGamut,
    CanonCinemaGamut,
    BmdWideGamutGen5,
    Rec2020,
    P3D65,
    Rec709,
}

impl StillGamut {
    pub const ALL: [StillGamut; 15] = [
        StillGamut::DaVinciWideGamut,
        StillGamut::Xyz,
        StillGamut::AcesAp0,
        StillGamut::AcesAp1,
        StillGamut::ArriWideGamut3,
        StillGamut::ArriWideGamut4,
        StillGamut::SGamut3,
        StillGamut::SGamut3Cine,
        StillGamut::VGamut,
        StillGamut::RedWideGamut,
        StillGamut::CanonCinemaGamut,
        StillGamut::BmdWideGamutGen5,
        StillGamut::Rec2020,
        StillGamut::P3D65,
        StillGamut::Rec709,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StillGamut::Xyz => "CIE XYZ",
            StillGamut::DaVinciWideGamut => "DaVinci Wide Gamut",
            StillGamut::AcesAp0 => "ACES AP0 (ACES2065-1)",
            StillGamut::AcesAp1 => "ACES AP1 (ACEScg / ACEScct)",
            StillGamut::ArriWideGamut3 => "ARRI Wide Gamut 3",
            StillGamut::ArriWideGamut4 => "ARRI Wide Gamut 4",
            StillGamut::SGamut3 => "S-Gamut3",
            StillGamut::SGamut3Cine => "S-Gamut3.Cine",
            StillGamut::VGamut => "V-Gamut",
            StillGamut::RedWideGamut => "REDWideGamutRGB",
            StillGamut::CanonCinemaGamut => "Canon Cinema Gamut",
            StillGamut::BmdWideGamutGen5 => "Blackmagic Wide Gamut Gen 5",
            StillGamut::Rec2020 => "Rec.2020",
            StillGamut::P3D65 => "P3-D65",
            StillGamut::Rec709 => "Rec.709",
        }
    }

    pub fn primaries(self) -> Option<Primaries> {
        Some(match self {
            StillGamut::Xyz => return None,
            StillGamut::DaVinciWideGamut => gamuts::DWG,
            StillGamut::AcesAp0 => gamuts::AP0,
            StillGamut::AcesAp1 => gamuts::AP1,
            StillGamut::ArriWideGamut3 => gamuts::AWG3,
            StillGamut::ArriWideGamut4 => gamuts::AWG4,
            StillGamut::SGamut3 => gamuts::S_GAMUT3,
            StillGamut::SGamut3Cine => gamuts::S_GAMUT3_CINE,
            StillGamut::VGamut => gamuts::V_GAMUT,
            StillGamut::RedWideGamut => gamuts::RWG,
            StillGamut::CanonCinemaGamut => gamuts::CINEMA_GAMUT,
            StillGamut::BmdWideGamutGen5 => gamuts::BMD_WG_GEN5,
            StillGamut::Rec2020 => gamuts::REC2020,
            StillGamut::P3D65 => gamuts::P3_D65,
            StillGamut::Rec709 => gamuts::REC709,
        })
    }

    /// Linear RGB → CIE XYZ, adapted to D65 (Bradford) when the gamut's white differs
    /// (ACES, BMD), as for LUT inputs (DECISIONS.md D6).
    pub fn rgb_to_xyz_d65(self) -> Mat3 {
        match self.primaries() {
            None => Mat3::identity(),
            Some(p) if p.white == whites::D65 => p.rgb_to_xyz(),
            Some(p) => Cat::Bradford.matrix(p.white, whites::D65) * p.rgb_to_xyz(),
        }
    }

    /// The gamut whose primaries match `p` (an EXR chromaticities attribute) to 0.002.
    pub fn from_primaries(p: &Primaries) -> Option<StillGamut> {
        let close = |a: &Primaries, b: &Primaries| {
            [(a.r, b.r), (a.g, b.g), (a.b, b.b)].iter().all(|(u, v)| (u.x - v.x).abs() < 2e-3 && (u.y - v.y).abs() < 2e-3)
        };
        // XYZ as an EXR writes it: primaries (1,0), (0,1), (0,0).
        if (p.r.x - 1.0).abs() < 2e-3 && p.r.y.abs() < 2e-3 && p.g.x.abs() < 2e-3 && (p.g.y - 1.0).abs() < 2e-3 {
            return Some(StillGamut::Xyz);
        }
        StillGamut::ALL.into_iter().find(|g| g.primaries().is_some_and(|q| close(&q, p)))
    }
}

/// Curves a still can be encoded with (ARRI LogC3 at EI 800 by default, like LUTs).
pub fn still_curves() -> Vec<LogCurve> {
    vec![
        LogCurve::DaVinciIntermediate,
        LogCurve::Linear,
        LogCurve::AcesCct,
        LogCurve::ArriLogC3 { ei: 800 },
        LogCurve::ArriLogC4,
        LogCurve::SLog3,
        LogCurve::VLog,
        LogCurve::Log3G10,
        LogCurve::CanonLog2,
        LogCurve::CanonLog3,
        LogCurve::BmdFilmGen5,
        LogCurve::FLog,
        LogCurve::FLog2,
    ]
}

/// Short ids for the curves (CLI, settings files): di, linear, acescct, logc3, logc4,
/// slog3, vlog, log3g10, clog2, clog3, gen5, flog, flog2.
pub fn curve_id(c: LogCurve) -> &'static str {
    match c {
        LogCurve::DaVinciIntermediate => "di",
        LogCurve::Linear => "linear",
        LogCurve::AcesCct => "acescct",
        LogCurve::ArriLogC3 { .. } => "logc3",
        LogCurve::ArriLogC4 => "logc4",
        LogCurve::SLog3 => "slog3",
        LogCurve::VLog => "vlog",
        LogCurve::Log3G10 => "log3g10",
        LogCurve::CanonLog2 => "clog2",
        LogCurve::CanonLog3 => "clog3",
        LogCurve::BmdFilmGen5 => "gen5",
        LogCurve::FLog => "flog",
        LogCurve::FLog2 => "flog2",
    }
}

pub fn curve_from_id(id: &str) -> Option<LogCurve> {
    still_curves().into_iter().find(|c| curve_id(*c) == id.to_lowercase())
}

/// The usual gamut for a curve (what a camera or Resolve pairs it with).
pub fn paired_gamut(c: LogCurve) -> Option<StillGamut> {
    Some(match c {
        LogCurve::DaVinciIntermediate => StillGamut::DaVinciWideGamut,
        LogCurve::AcesCct => StillGamut::AcesAp1,
        LogCurve::ArriLogC3 { .. } => StillGamut::ArriWideGamut3,
        LogCurve::ArriLogC4 => StillGamut::ArriWideGamut4,
        LogCurve::SLog3 => StillGamut::SGamut3Cine,
        LogCurve::VLog => StillGamut::VGamut,
        LogCurve::Log3G10 => StillGamut::RedWideGamut,
        LogCurve::CanonLog2 | LogCurve::CanonLog3 => StillGamut::CanonCinemaGamut,
        LogCurve::BmdFilmGen5 => StillGamut::BmdWideGamutGen5,
        LogCurve::FLog | LogCurve::FLog2 => StillGamut::Rec2020,
        LogCurve::Linear => return None,
    })
}

/// A still's encoding.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StillSpace {
    pub curve: LogCurve,
    pub gamut: StillGamut,
}

impl StillSpace {
    pub const DWG_DI: StillSpace = StillSpace { curve: LogCurve::DaVinciIntermediate, gamut: StillGamut::DaVinciWideGamut };

    pub fn name(&self) -> String {
        format!("{} / {}", self.gamut.label(), self.curve.name())
    }

    /// Stored code values → CIE XYZ (D65).
    pub fn to_xyz(&self, code: &[f64; 3]) -> Vec3 {
        let c = self.curve;
        self.gamut.rgb_to_xyz_d65() * Vec3::new(c.decode(code[0]), c.decode(code[1]), c.decode(code[2]))
    }
}

/// How well one curve explains the neutral row.
#[derive(Debug, Clone, Serialize)]
pub struct CurveFit {
    pub curve: LogCurve,
    /// Slope of log2(decoded) against log2(reflectance), per channel (NaN when a channel
    /// had fewer than 3 usable patches). 1 = proportional to light.
    pub slopes: [f64; 3],
    /// RMS deviation from that straight line, stops.
    pub rms_stops: f64,
    /// |log2(median slope)| + rms: lower is better.
    pub score: f64,
}

/// Rank curves by how linear-in-light they make the neutral row (white … neutral 3.5;
/// black is left out, flare lifts it). `usable[i][k]`: patch i channel k is not clipped.
pub fn rank_curves(codes: &[[f64; 3]; 24], usable: &[[bool; 3]; 24], ref_y: &[f64; 24]) -> Vec<CurveFit> {
    let mut out: Vec<CurveFit> = still_curves()
        .into_iter()
        .map(|curve| {
            let mut slopes = [f64::NAN; 3];
            let mut rss = 0.0;
            let mut n = 0usize;
            for k in 0..3 {
                let pts: Vec<(f64, f64)> = NEUTRALS[..5]
                    .iter()
                    .filter(|&&i| usable[i][k])
                    .filter_map(|&i| {
                        let l = curve.decode(codes[i][k]);
                        (l > 0.0 && l.is_finite()).then(|| (ref_y[i].log2(), l.log2()))
                    })
                    .collect();
                if pts.len() < 3 {
                    continue;
                }
                let m = pts.len() as f64;
                let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / m, pts.iter().map(|p| p.1).sum::<f64>() / m);
                let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
                let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
                let b = sxy / sxx;
                slopes[k] = b;
                for p in &pts {
                    rss += (my + b * (p.0 - mx) - p.1).powi(2);
                }
                n += pts.len();
            }
            let mut s: Vec<f64> = slopes.iter().copied().filter(|v| v.is_finite()).collect();
            s.sort_by(f64::total_cmp);
            let rms = if n > 0 { (rss / n as f64).sqrt() } else { f64::NAN };
            let score = match s.len() {
                0 => f64::INFINITY,
                _ if s[s.len() / 2] <= 0.0 => f64::INFINITY,
                _ => s[s.len() / 2].log2().abs() + rms,
            };
            CurveFit { curve, slopes, rms_stops: rms, score }
        })
        .collect();
    out.sort_by(|a, b| a.score.total_cmp(&b.score));
    out
}

/// A detected still encoding with reasons.
#[derive(Debug, Clone, Serialize)]
pub struct SpaceGuess {
    pub space: StillSpace,
    pub curve_confidence: Confidence,
    pub gamut_confidence: Confidence,
    pub reasons: Vec<String>,
    /// Every curve, best first.
    pub curves: Vec<CurveFit>,
}

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Keywords in a file name, as (curve, gamut) evidence.
fn name_hints(name: &str) -> (Option<LogCurve>, Option<StillGamut>) {
    let n = norm(name);
    let has = |k: &str| n.contains(k);
    let curve = if has("davinciintermediate") || has("dwgdi") || has("dwgintermediate") {
        Some(LogCurve::DaVinciIntermediate)
    } else if has("acescct") {
        Some(LogCurve::AcesCct)
    } else if has("logc4") {
        Some(LogCurve::ArriLogC4)
    } else if has("logc3") || has("logc") {
        Some(LogCurve::ArriLogC3 { ei: 800 })
    } else if has("slog3") {
        Some(LogCurve::SLog3)
    } else if has("vlog") {
        Some(LogCurve::VLog)
    } else if has("log3g10") {
        Some(LogCurve::Log3G10)
    } else if has("clog3") || has("canonlog3") {
        Some(LogCurve::CanonLog3)
    } else if has("clog2") || has("canonlog2") {
        Some(LogCurve::CanonLog2)
    } else if has("flog2") {
        Some(LogCurve::FLog2)
    } else if has("flog") {
        Some(LogCurve::FLog)
    } else if has("gen5") || has("bmdfilm") {
        Some(LogCurve::BmdFilmGen5)
    } else if has("linear") || has("acescg") || has("aces2065") || has("scenelinear") || has("xyzlin") {
        Some(LogCurve::Linear)
    } else {
        None
    };
    let gamut = if has("xyz") {
        Some(StillGamut::Xyz)
    } else if has("dwg") || has("davinciwidegamut") {
        Some(StillGamut::DaVinciWideGamut)
    } else if has("aces2065") || has("ap0") {
        Some(StillGamut::AcesAp0)
    } else if has("acescg") || has("acescct") || has("ap1") {
        Some(StillGamut::AcesAp1)
    } else if has("awg4") {
        Some(StillGamut::ArriWideGamut4)
    } else if has("awg3") || has("awg") {
        Some(StillGamut::ArriWideGamut3)
    } else if has("sgamut3cine") {
        Some(StillGamut::SGamut3Cine)
    } else if has("sgamut3") {
        Some(StillGamut::SGamut3)
    } else if has("vgamut") {
        Some(StillGamut::VGamut)
    } else if has("rwg") || has("redwidegamut") {
        Some(StillGamut::RedWideGamut)
    } else if has("rec2020") || has("bt2020") {
        Some(StillGamut::Rec2020)
    } else if has("rec709") {
        Some(StillGamut::Rec709)
    } else {
        None
    };
    (curve, gamut)
}

/// Guess the still's encoding from its neutral row, file name and header.
pub fn guess_space(
    file_name: &str,
    info: &ImageInfo,
    codes: &[[f64; 3]; 24],
    usable: &[[bool; 3]; 24],
    ref_y: &[f64; 24],
) -> SpaceGuess {
    let ranked = rank_curves(codes, usable, ref_y);
    let mut reasons = Vec::new();
    let (name_curve, name_gamut) = name_hints(file_name);
    let header_gamut = info.chromaticities.as_ref().and_then(StillGamut::from_primaries);
    let best = ranked.first();
    let fmt_slopes = |c: &CurveFit| {
        c.slopes.iter().map(|s| if s.is_finite() { format!("{s:.2}") } else { "–".into() }).collect::<Vec<_>>().join(" / ")
    };
    // Linear vs log from the chart: the best curve's family, if it is clearly better
    // than the best of the other family.
    let is_lin = |c: &LogCurve| *c == LogCurve::Linear;
    let best_lin = ranked.iter().find(|c| is_lin(&c.curve));
    let best_log = ranked.iter().find(|c| !is_lin(&c.curve));
    let chart_says_linear = match (best_lin, best_log) {
        (Some(l), Some(g)) if l.score.is_finite() && g.score.is_finite() => Some((l.score < g.score, (l.score - g.score).abs())),
        _ => None,
    };
    let (curve, curve_conf) = match (name_curve, chart_says_linear) {
        (Some(c), Some((lin, margin))) if is_lin(&c) == lin => {
            reasons.push(format!("file name says {}; the neutral row agrees ({} family)", c.name(), if lin { "linear" } else { "log" }));
            (c, if margin > 0.3 { Confidence::High } else { Confidence::Medium })
        }
        (Some(c), Some((lin, margin))) if margin > 0.5 => {
            let other = if lin { LogCurve::Linear } else { best_log.map(|g| g.curve).unwrap_or(LogCurve::DaVinciIntermediate) };
            reasons.push(format!(
                "file name says {}, but the neutral row is clearly {} (slopes {} with {}); using {}",
                c.name(),
                if lin { "linear" } else { "log-encoded" },
                fmt_slopes(if lin { best_lin.unwrap() } else { best_log.unwrap() }),
                other.name(),
                if lin { "Linear" } else { "DaVinci Intermediate" }
            ));
            (if lin { LogCurve::Linear } else { LogCurve::DaVinciIntermediate }, Confidence::Low)
        }
        (Some(c), _) => {
            reasons.push(format!("file name says {}", c.name()));
            (c, Confidence::Medium)
        }
        (None, Some((true, margin))) => {
            let l = best_lin.unwrap();
            reasons.push(format!(
                "the neutral row is proportional to light when read as linear (slopes {} R/G/B, 1 = exact)",
                fmt_slopes(l)
            ));
            (LogCurve::Linear, if margin > 0.5 { Confidence::High } else { Confidence::Medium })
        }
        (None, Some((false, margin))) => {
            // A log curve; which one the chart can't tell reliably (they differ far less
            // than real stills deviate from them), so DaVinci Intermediate, the DCTL's own
            // space, unless the name or header says otherwise.
            let g = best_log.unwrap();
            let pick = ranked.iter().find(|c| c.curve == LogCurve::DaVinciIntermediate).unwrap();
            reasons.push(format!(
                "the neutral row is log-encoded (as linear its slopes would be {}); which log curve can't be \
                 told from a chart, so {} is assumed (slopes {} with it{})",
                best_lin.map(fmt_slopes).unwrap_or_default(),
                pick.curve.name(),
                fmt_slopes(pick),
                if g.curve != pick.curve { format!("; closest: {}, {}", g.curve.name(), fmt_slopes(g)) } else { String::new() }
            ));
            (pick.curve, if margin > 0.5 { Confidence::Low } else { Confidence::None })
        }
        (None, None) => {
            reasons.push("too few unclipped neutral patches to judge the curve; assuming DaVinci Intermediate".into());
            (LogCurve::DaVinciIntermediate, Confidence::None)
        }
    };
    if let Some(b) = best {
        if b.score > 0.6 && b.score.is_finite() {
            reasons.push(format!(
                "no curve makes the neutral row proportional to light (best: {}, slopes {}): the still's tone \
                 was changed before export (tone mapping, a camera picture profile, a grade), or the patches \
                 are lit unevenly",
                b.curve.name(),
                fmt_slopes(b)
            ));
        }
    }
    let (gamut, gamut_conf) = if let Some(g) = header_gamut {
        reasons.push(format!("EXR header chromaticities are {}", g.label()));
        (g, Confidence::High)
    } else if let Some(g) = name_gamut {
        reasons.push(format!("file name says {}", g.label()));
        (g, Confidence::Medium)
    } else if let Some(g) = paired_gamut(curve) {
        reasons.push(format!("{} assumed, the gamut that goes with {} (a chart can't show its gamut)", g.label(), curve.name()));
        (g, Confidence::Low)
    } else {
        reasons.push("linear still with no gamut in its name or header: CIE XYZ assumed; choose the gamut it was exported in".into());
        (StillGamut::Xyz, Confidence::None)
    };
    SpaceGuess { space: StillSpace { curve, gamut }, curve_confidence: curve_conf, gamut_confidence: gamut_conf, reasons, curves: ranked }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::reference::ChartEdition;

    fn ref_y() -> [f64; 24] {
        let x = ChartEdition::After2014.xyz_d65();
        std::array::from_fn(|i| x[i].y)
    }

    #[test]
    fn exr_xyz_and_dwg_chromaticities() {
        let xyz = Primaries {
            r: crate::math::Xy::new(1.0, 0.0),
            g: crate::math::Xy::new(0.0, 1.0),
            b: crate::math::Xy::new(0.0, 0.0),
            white: crate::math::Xy::new(1.0 / 3.0, 1.0 / 3.0),
        };
        assert_eq!(StillGamut::from_primaries(&xyz), Some(StillGamut::Xyz));
        assert_eq!(StillGamut::from_primaries(&gamuts::DWG), Some(StillGamut::DaVinciWideGamut));
        assert_eq!(StillGamut::from_primaries(&gamuts::AP0), Some(StillGamut::AcesAp0));
    }

    /// A neutral row encoded with each curve is ranked linear-in-light by that curve
    /// (slope 1), and linear vs log is never confused.
    #[test]
    fn curve_ranking_tells_linear_from_log() {
        let y = ref_y();
        let usable = [[true; 3]; 24];
        for enc in still_curves() {
            let codes: [[f64; 3]; 24] = std::array::from_fn(|i| [enc.encode(y[i] * 0.7); 3]);
            let r = rank_curves(&codes, &usable, &y);
            let own = r.iter().find(|c| c.curve == enc).unwrap();
            assert!(own.score < 1e-6, "{enc:?}: {own:?}");
            let lin = r.iter().find(|c| c.curve == LogCurve::Linear).unwrap();
            if enc != LogCurve::Linear {
                assert!(lin.score > 0.6, "{enc:?} read as linear: {lin:?}");
            } else {
                assert!(r.iter().filter(|c| c.curve != LogCurve::Linear).all(|c| c.score > 0.6));
            }
        }
    }
}
