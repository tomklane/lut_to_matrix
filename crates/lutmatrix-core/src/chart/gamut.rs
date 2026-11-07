//! Gamut compression after the chart correction (DECISIONS.md D49).
//!
//! A strong correction (under water: red ×5 minus green) pushes colours the chart doesn't
//! have past the edge of the gamut: one channel goes below 0 and clips (black or coloured
//! specks in dark, noisy water, hard edges on saturated colours). This pulls them back
//! smoothly, the way the ACES Reference Gamut Compression does: per channel, the distance
//! from the pixel's largest channel, `d = (max − c) / max` (0 = neutral, 1 = at the edge,
//! above 1 = outside), is left alone up to a threshold and compressed above it so that
//! the limit lands on the edge.
//!
//! Source: Academy of Motion Picture Arts and Sciences, ACES Reference Gamut Compression
//! (`LMT.Academy.ReferenceGamutCompress.ctl`, ACES 1.3, 2021): distance from the
//! achromatic axis, the power curve `t + s·x / (1 + x^p)^(1/p)`, `x = (d − t) / s`, with
//! `s` such that the limit maps to 1; power 1.2.
//!
//! Here, in linear DaVinci Wide Gamut on the correction's output: the threshold is set
//! just above the chart's most saturated colours (so the chart match is unchanged) and
//! each channel's limit is how far the correction pushes that channel on the chart
//! still's own pixels (its 99.9th percentile), so the still's footage comes back inside.

use serde::{Deserialize, Serialize};

use crate::math::Vec3;

/// What the user asks for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GamutSettings {
    /// Put the compression in the DCTL (and the preview).
    pub enabled: bool,
    /// Where compression starts (distance from neutral, 0 … 1). None = just above the
    /// chart's most saturated colour.
    pub threshold: Option<f64>,
}

impl Default for GamutSettings {
    fn default() -> Self {
        GamutSettings { enabled: true, threshold: None }
    }
}

/// ACES RGC's power.
pub const POWER: f64 = 1.2;
/// DCTL slider range for the threshold.
pub const THRESHOLD_RANGE: (f64, f64) = (0.5, 0.98);
/// Margin above the chart's most saturated colour for the automatic threshold.
const THRESHOLD_MARGIN: f64 = 0.04;
/// The smallest limit: a channel the correction never pushes out still gets a gentle
/// curve (the limit must be above the edge).
const MIN_LIMIT: f64 = 1.05;
/// The largest limit (beyond it the compression would flatten everything above the
/// threshold onto the edge).
const MAX_LIMIT: f64 = 4.0;
/// Pixels darker than this (largest channel, linear DWG) don't count towards the limits.
const FLOOR: f64 = 0.002;

/// The compression as it goes into the DCTL.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Gamut {
    pub enabled: bool,
    pub threshold: f64,
    /// The threshold the chart asks for (shown when it is set by hand).
    pub auto_threshold: f64,
    /// How far the chart's most saturated target colour is from neutral.
    pub chart_reach: f64,
    /// Per channel (R, G, B): the distance that lands on the edge.
    pub limits: [f64; 3],
    /// Share of the still's pixels the correction pushes outside the gamut (any channel
    /// below 0), before compression.
    pub outside: f64,
    /// … and how many of those stay outside with it (beyond the limit).
    pub outside_after: f64,
}

impl Gamut {
    /// `targets`: the chart's target colours (linear DWG). `pixels`: the still's pixels
    /// after the correction (linear DWG).
    pub fn resolve(s: &GamutSettings, targets: &[Vec3], pixels: &[Vec3]) -> Gamut {
        let auto = targets
            .iter()
            .filter_map(distances)
            .flat_map(|d| d.into_iter())
            .fold(0.0, f64::max);
        let auto_threshold = (auto + THRESHOLD_MARGIN).clamp(0.7, THRESHOLD_RANGE.1);
        let threshold = match s.threshold {
            Some(t) if t.is_finite() => t.clamp(THRESHOLD_RANGE.0, THRESHOLD_RANGE.1),
            _ => auto_threshold,
        };
        let ds: Vec<[f64; 3]> = pixels.iter().filter(|p| p.max() > FLOOR).filter_map(distances).collect();
        let limits = std::array::from_fn(|k| {
            let mut v: Vec<f64> = ds.iter().map(|d| d[k]).collect();
            let p = percentile(&mut v, 0.999).unwrap_or(0.0);
            p.clamp(MIN_LIMIT.max(threshold + 0.05), MAX_LIMIT)
        });
        let n = ds.len().max(1) as f64;
        let outside = ds.iter().filter(|d| d.iter().any(|v| *v > 1.0)).count() as f64 / n;
        let mut g = Gamut { enabled: s.enabled, threshold, auto_threshold, chart_reach: auto, limits, outside, outside_after: 0.0 };
        g.outside_after = ds.iter().filter(|d| (0..3).any(|k| g.compress_distance(d[k], k) > 1.0 + 1e-9)).count() as f64 / n;
        g
    }

    /// One channel's distance, compressed (ACES RGC).
    pub fn compress_distance(&self, d: f64, k: usize) -> f64 {
        let (t, l) = (self.threshold, self.limits[k]);
        if d < t || l <= 1.0 || t >= 1.0 || l <= t {
            return d;
        }
        let s = (l - t) / (((1.0 - t) / (l - t)).powf(-POWER) - 1.0).powf(1.0 / POWER);
        let x = (d - t) / s;
        t + s * x / (1.0 + x.powf(POWER)).powf(1.0 / POWER)
    }

    /// A linear DWG colour, compressed (unchanged when off).
    pub fn apply(&self, dwg: &Vec3) -> Vec3 {
        if !self.enabled {
            return *dwg;
        }
        let a = dwg.max();
        if a.is_nan() || a <= 0.0 {
            return *dwg;
        }
        Vec3::from_fn(|k, _| a - self.compress_distance((a - dwg[k]) / a, k) * a)
    }
}

/// Distance of each channel from the largest, relative to it (None for a colour without a
/// positive channel).
fn distances(c: &Vec3) -> Option<[f64; 3]> {
    let a = c.max();
    (a > 0.0 && c.iter().all(|v| v.is_finite())).then(|| std::array::from_fn(|k| (a - c[k]) / a))
}

fn percentile(v: &mut [f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[((v.len() - 1) as f64 * p).round() as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gamut(t: f64, l: f64) -> Gamut {
        Gamut { enabled: true, threshold: t, auto_threshold: t, chart_reach: t, limits: [l; 3], outside: 0.0, outside_after: 0.0 }
    }

    #[test]
    fn leaves_the_inside_alone_and_lands_the_limit_on_the_edge() {
        let g = gamut(0.8, 1.6);
        assert_eq!(g.compress_distance(0.5, 0), 0.5);
        assert_eq!(g.compress_distance(0.8, 0), 0.8);
        assert!((g.compress_distance(1.6, 0) - 1.0).abs() < 1e-12);
        // Smooth and monotonic from the threshold on, never past the asymptote.
        let mut last = 0.8;
        for i in 1..400 {
            let d = 0.8 + i as f64 * 0.01;
            let c = g.compress_distance(d, 0);
            assert!(c > last && c < d, "{d}: {c}");
            last = c;
        }
        let slope = (g.compress_distance(0.8 + 1e-6, 0) - 0.8) / 1e-6;
        assert!((slope - 1.0).abs() < 1e-3, "{slope}");
    }

    #[test]
    fn pulls_negative_channels_back_and_keeps_neutrals_and_the_largest_channel() {
        let g = gamut(0.8, 1.6);
        // Red pushed below 0 (distance 1.4): back inside, green unchanged.
        let c = g.apply(&Vec3::new(-0.04, 0.10, 0.07));
        assert!(c.x > 0.0 && c.y == 0.10 && c.z == 0.07, "{c:?}");
        for v in [Vec3::repeat(0.3), Vec3::new(0.2, 0.15, 0.1), Vec3::zeros(), Vec3::repeat(-0.1)] {
            assert_eq!(g.apply(&v), v);
        }
        assert_eq!(Gamut { enabled: false, ..g }.apply(&Vec3::new(-0.04, 0.1, 0.07)), Vec3::new(-0.04, 0.1, 0.07));
    }

    #[test]
    fn resolve_sets_the_threshold_above_the_chart_and_limits_from_the_pixels() {
        let targets = [Vec3::new(0.4, 0.1, 0.05), Vec3::new(0.1, 0.2, 0.3), Vec3::repeat(0.18)];
        // The most saturated target: red patch, blue distance 0.875.
        let mut px: Vec<Vec3> = (0..2000).map(|i| Vec3::new(0.05 + 0.0001 * i as f64, 0.1, 0.1)).collect();
        px.extend((0..10).map(|i| Vec3::new(-0.02 - 0.01 * i as f64, 0.1, 0.1)));
        let g = Gamut::resolve(&GamutSettings::default(), &targets, &px);
        assert!((g.threshold - (0.875 + THRESHOLD_MARGIN)).abs() < 1e-12, "{g:?}");
        assert!(g.limits[0] > 1.5 && g.limits[0] < 2.2 && g.limits[1] == MIN_LIMIT, "{g:?}");
        assert!(g.outside > 0.004 && g.outside < 0.006 && g.outside_after < g.outside, "{g:?}");
        let h = Gamut::resolve(&GamutSettings { enabled: true, threshold: Some(0.6) }, &targets, &px);
        assert_eq!(h.threshold, 0.6);
        assert_eq!(h.auto_threshold, g.threshold);
    }
}
