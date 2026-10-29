//! Highlight protection for the chart DCTL (DECISIONS.md D42).
//!
//! A chart correction is fitted to real colours. Clipped pixels are not: where every
//! channel clipped the file holds R = G = B, which under strongly coloured light is far
//! redder than the camera's own white (under water the camera sees white as about
//! 0.34 : 1 : 1.22), so the correction turns it red or magenta; where only green clipped
//! the ratios are wrong in the same direction (a pink halo).
//!
//! The protection rebuilds clipped channels towards the camera's white: of the colours
//! with the camera's white balance, take the darkest that is at least as bright as the
//! pixel in every channel (`gray · max_k(x_k / gray_k)`), and raise each channel towards
//! it by that channel's own weight, which rises smoothly (smoothstep) from 0 at
//! `softness` stops below the clip to `strength` at the clip. A fully clipped pixel
//! becomes the camera's white, which the correction makes neutral; a channel that isn't
//! near the clip is never touched, so bright but unclipped colours keep theirs. With the
//! DCTL's white balance off (footage balanced in the raw decode) the camera's white is
//! equal R, G, B. The DCTL carries the same formulas with sliders for the strength, clip
//! level and softness.

use serde::{Deserialize, Serialize};

use super::detect::ClipLevels;
use super::encoding::StillSpace;
use super::fit::ChartFit;
use crate::math::Vec3;
use crate::spaces::davinci;

/// What the user asks for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HighlightSettings {
    /// Put the protection (and its sliders) in the DCTL.
    pub enabled: bool,
    /// Where the footage clips, as a DaVinci Intermediate code value. None = measured
    /// from the still.
    pub clip_di: Option<f64>,
    /// Width of the transition below the clip, stops.
    pub softness: f64,
    /// 0 = off … 1 = full.
    pub strength: f64,
}

impl Default for HighlightSettings {
    fn default() -> Self {
        HighlightSettings { enabled: false, clip_di: None, softness: 1.0, strength: 1.0 }
    }
}

/// Where the clip level came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipSource {
    /// Set by the user.
    Set,
    /// Measured: a channel of the still piles up at its largest value.
    Measured,
    /// Nothing in the still clips: its brightest value (the footage clips at or above it).
    Brightest,
    /// Nothing usable in the still.
    Default,
}

/// The protection as it goes into the DCTL.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Highlights {
    pub enabled: bool,
    pub clip_di: f64,
    pub source: ClipSource,
    /// The still's measured clip (or brightest value), DI, for the UI.
    pub measured_di: f64,
    pub measured_source: ClipSource,
    pub softness: f64,
    pub strength: f64,
}

/// DCTL slider range for the clip level.
pub const CLIP_DI_RANGE: (f64, f64) = (0.2, 1.0);
/// DCTL slider range for the softness, stops.
pub const SOFTNESS_RANGE: (f64, f64) = (0.1, 4.0);
/// Used when the still says nothing: a typical camera clip in DI (about 4.7 stops over
/// 18 % gray).
const DEFAULT_CLIP_DI: f64 = 0.68;

/// The still's clip level as a DI code value. A pixel at the clip of its earliest-clipping
/// channel, taken as neutral (equal codes), decoded with the still's encoding to linear
/// DWG; its brightest channel encoded as DI. For a DWG/DI still this is that channel's
/// clip code.
pub fn still_clip_di(clip: &ClipLevels, space: &StillSpace) -> (f64, ClipSource) {
    let to_di = |code: f64| -> Option<f64> {
        let xyz = space.to_xyz(&[code; 3]);
        let d = davinci::xyz_to_dwg() * xyz;
        let mx = d.x.max(d.y).max(d.z);
        (mx.is_finite() && mx > 0.0).then(|| davinci::di_encode(mx))
    };
    let clipped = (0..3).filter(|&k| clip.plateau[k] && clip.high[k].is_finite()).map(|k| clip.high[k]).fold(f64::INFINITY, f64::min);
    if let Some(v) = clipped.is_finite().then(|| to_di(clipped)).flatten() {
        return (v, ClipSource::Measured);
    }
    let brightest = clip.max.iter().copied().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
    if let Some(v) = brightest.is_finite().then(|| to_di(brightest)).flatten() {
        return (v, ClipSource::Brightest);
    }
    (DEFAULT_CLIP_DI, ClipSource::Default)
}

impl Highlights {
    pub fn resolve(s: &HighlightSettings, clip: &ClipLevels, space: &StillSpace) -> Highlights {
        let (measured, msrc) = still_clip_di(clip, space);
        let (lo, hi) = CLIP_DI_RANGE;
        let (clip_di, source) = match s.clip_di {
            Some(v) if v.is_finite() => (v.clamp(lo, hi), ClipSource::Set),
            _ => (measured.clamp(lo, hi), msrc),
        };
        Highlights {
            enabled: s.enabled,
            clip_di,
            source,
            measured_di: measured,
            measured_source: msrc,
            softness: if s.softness.is_finite() { s.softness.clamp(SOFTNESS_RANGE.0, SOFTNESS_RANGE.1) } else { 1.0 },
            strength: if s.strength.is_finite() { s.strength.clamp(0.0, 1.0) } else { 1.0 },
        }
    }

    /// Protection weight of one channel value (linear DWG): 0 = untouched … strength.
    pub fn weight(&self, v: f64) -> f64 {
        let lc = davinci::di_decode(self.clip_di);
        if self.strength <= 0.0 || v <= 0.0 || lc <= 0.0 {
            return 0.0;
        }
        let t = ((v / lc).log2() / self.softness.max(1e-3) + 1.0).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t) * self.strength
    }

    /// The camera's white the clipped channels are rebuilt towards (linear DWG, green =
    /// 1): the chart's gray with the white balance on, equal R, G, B without.
    pub fn camera_white(fit: &ChartFit, white_balance: bool) -> Vec3 {
        let g = Vec3::from(fit.gray_dwg);
        if white_balance && g.iter().all(|v| v.is_finite() && *v > 1e-6) {
            g
        } else {
            Vec3::repeat(1.0)
        }
    }

    /// A linear DWG pixel with its clipped channels rebuilt towards `white`.
    pub fn rebuild(&self, dwg: &Vec3, white: &Vec3) -> Vec3 {
        if !self.enabled {
            return *dwg;
        }
        let s = (0..3).map(|k| dwg[k] / white[k]).fold(f64::NEG_INFINITY, f64::max);
        Vec3::from_fn(|k, _| {
            let est = s * white[k];
            if est > dwg[k] {
                dwg[k] + self.weight(dwg[k]) * (est - dwg[k])
            } else {
                dwg[k]
            }
        })
    }

    /// The DCTL's result for a linear DWG pixel, CIE XYZ (D65) out (`Amount` = 1).
    pub fn apply(&self, fit: &ChartFit, dwg: &Vec3, white_balance: bool) -> Vec3 {
        let src = self.rebuild(dwg, &Self::camera_white(fit, white_balance));
        fit.apply_wb(&(davinci::dwg_to_xyz() * src), white_balance)
    }
}
