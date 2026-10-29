//! Highlight protection for the chart DCTL (DECISIONS.md D42).
//!
//! A chart correction is fitted to real colours. Clipped pixels are not real colours:
//! where every channel is clipped the file holds R = G = B, which the correction treats
//! as "redder than this camera's white" under strongly coloured light and turns red; where
//! only one channel is clipped the ratios are wrong and the correction swings them further.
//! Near the clip level the protection therefore
//! 1. pulls the input towards its brightest channel (a clipped pixel becomes neutral at
//!    the clip, its most likely colour), and
//! 2. pulls the output towards a neutral of the same luminance as the corrected pixel,
//!    so clipped areas come out white at the brightness the correction gives them.
//!
//! The weight rises smoothly (smoothstep) from 0 at `softness` stops below the clip to
//! `strength` at the clip. It is measured on a level between the brightest channel
//! (`detect` = 0: any channel near the clip counts) and the darkest one (`detect` = 1:
//! only pixels with every channel near the clip), in linear DWG. Under strongly coloured
//! light one channel clips over large, otherwise fine areas (green around the sun under
//! water) that the correction handles acceptably, while whitening them looks wrong, so the
//! default is the darkest channel: only where everything clips (where the correction does
//! the most damage, e.g. a red or magenta sun). The DCTL carries the same formulas with sliders
//! for the strength, clip level, softness and detection.

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
    /// Which channel decides: 0 = the brightest (any channel clipping) … 1 = the darkest
    /// (only every channel clipping).
    pub detect: f64,
}

/// Default for [`HighlightSettings::detect`].
pub const DEFAULT_DETECT: f64 = 1.0;

impl Default for HighlightSettings {
    fn default() -> Self {
        HighlightSettings { enabled: false, clip_di: None, softness: 1.0, strength: 1.0, detect: DEFAULT_DETECT }
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
    pub detect: f64,
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
            detect: if s.detect.is_finite() { s.detect.clamp(0.0, 1.0) } else { DEFAULT_DETECT },
        }
    }

    /// Protection weight for a linear DWG pixel (0 = untouched … strength).
    pub fn weight(&self, dwg: &Vec3) -> f64 {
        let mx = dwg.x.max(dwg.y).max(dwg.z);
        let mn = dwg.x.min(dwg.y).min(dwg.z);
        let v = mx + (mn - mx) * self.detect;
        let lc = davinci::di_decode(self.clip_di);
        if self.strength <= 0.0 || v <= 0.0 || lc <= 0.0 {
            return 0.0;
        }
        let t = ((v / lc).log2() / self.softness.max(1e-3) + 1.0).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t) * self.strength
    }

    /// The protected correction of a linear DWG pixel, CIE XYZ (D65) out. Without
    /// protection this is `fit.apply(DWG_TO_XYZ · dwg)`.
    pub fn apply(&self, fit: &ChartFit, dwg: &Vec3) -> Vec3 {
        if !self.enabled {
            return fit.apply(&(davinci::dwg_to_xyz() * dwg));
        }
        let h = self.weight(dwg);
        let mx = dwg.x.max(dwg.y).max(dwg.z);
        let src = dwg + (Vec3::repeat(mx) - dwg) * h;
        let out = fit.apply(&(davinci::dwg_to_xyz() * src));
        let white = davinci::dwg_to_xyz() * Vec3::repeat(1.0);
        let neutral = white * (out.y.max(0.0) / white.y);
        out + (neutral - out) * h
    }
}
