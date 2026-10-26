//! A LUT's gray axis: how far the display colour of scene neutrals is from D65 at each
//! exposure, and a correction that makes it neutral (DECISIONS.md D33).
//!
//! OpenDRT renders scene neutrals as D65 at every level (with creative white D65), while
//! film-print LUTs often tint them, and differently in shadows, mid-tones and highlights.
//! An OpenDRT match against such a LUT bends other parameters to approach a cast it cannot
//! make. [`Corrector`] removes the cast from the LUT's output before matching: each output
//! colour is adapted (Bradford) from the LUT's own gray at the same luminance to D65. It
//! depends only on the LUT's output, so it is a real display-referred correction (a node
//! after the LUT would do the same), and it maps the gray axis exactly onto D65.

use serde::{Deserialize, Serialize};

use crate::difference::xyz_to_lab;
use crate::lut::{Interpolation, Transform};
use crate::math::{whites, Cat, Vec3, Xy};
use crate::spaces::{InputSpace, OutputSpace};

/// How the LUT's gray axis is treated when matching OpenDRT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NeutralGray {
    /// Match the LUT as it is, cast included.
    #[default]
    Off,
    /// One white balance for every colour: the LUT's 18% gray becomes D65.
    MiddleGray,
    /// White balance per luminance level: every gray on the axis becomes D65.
    Exposure,
}

impl NeutralGray {
    pub fn label(self) -> &'static str {
        match self {
            NeutralGray::Off => "off",
            NeutralGray::MiddleGray => "middle gray",
            NeutralGray::Exposure => "every level",
        }
    }
}

/// The LUT's output for scene gray at one exposure.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GrayPoint {
    /// Exposure, stops relative to 18% gray.
    pub stops: f64,
    /// Output luminance, cd/m².
    pub nits: f64,
    /// CIELAB of the output (white = D65 at the display peak).
    pub l: f64,
    pub a: f64,
    pub b: f64,
    /// u′v′ distance from D65 (0.004 is about a just-noticeable cast at mid-tones).
    pub duv: f64,
}

impl GrayPoint {
    /// The direction of the cast, in words ("yellow-green"); "neutral" below 0.0015 Δu′v′.
    pub fn cast(&self) -> &'static str {
        if self.duv < 0.0015 {
            return "neutral";
        }
        let h = self.b.atan2(self.a).to_degrees().rem_euclid(360.0);
        match h {
            h if (20.0..70.0).contains(&h) => "red-orange",
            h if (70.0..105.0).contains(&h) => "yellow",
            h if (105.0..150.0).contains(&h) => "yellow-green",
            h if (150.0..200.0).contains(&h) => "green",
            h if (200.0..250.0).contains(&h) => "cyan",
            h if (250.0..300.0).contains(&h) => "blue",
            h if (300.0..340.0).contains(&h) => "magenta",
            _ => "red-magenta",
        }
    }
}

/// Levels darker than this (L*) are not counted as a visible cast.
pub const VISIBLE_L: f64 = 10.0;
/// A cast this large (Δu′v′) at L* ≥ [`VISIBLE_L`] is reported.
pub const VISIBLE_DUV: f64 = 0.005;

fn uv(x: &Vec3) -> (f64, f64) {
    let d = x.x + 15.0 * x.y + 3.0 * x.z;
    (4.0 * x.x / d, 9.0 * x.y / d)
}

fn uv_to_xy(u: f64, v: f64) -> Xy {
    let d = 6.0 * u - 16.0 * v + 12.0;
    Xy::new(9.0 * u / d, 4.0 * v / d)
}

/// The measured gray axis of a LUT.
#[derive(Debug, Clone)]
pub struct GrayAxis {
    /// −10…+10 stops in ¼ stops, samples inside the LUT's input range only.
    pub points: Vec<GrayPoint>,
    /// (ln nits, u′, v′) along rising luminance, for interpolation.
    curve: Vec<(f64, f64, f64)>,
    /// u′v′ of the measured point closest to 18% gray.
    middle: (f64, f64),
}

impl GrayAxis {
    pub fn measure(lut: &dyn Transform, input: &InputSpace, output: &OutputSpace, interp: Interpolation) -> GrayAxis {
        let d65 = whites::D65.to_xyz();
        let (wu, wv) = uv(&d65);
        let lab_white = d65 * output.peak_nits();
        let to_cam = input.xyz_d65_to_rgb();
        let disp = output.rgb_to_xyz();
        let mut points = Vec::new();
        let mut curve: Vec<(f64, f64, f64)> = Vec::new();
        let mut middle = (wu, wv, f64::INFINITY);
        for k in 0..=80 {
            let stops = -10.0 + 0.25 * k as f64;
            let code = input.encode(&(to_cam * (d65 * (0.18 * 2f64.powf(stops)))));
            if !lut.in_domain_rgb(&code) {
                continue;
            }
            let xyz = disp * output.decode(&lut.apply_rgb(&code, interp));
            if !xyz.iter().all(|v| v.is_finite()) || xyz.y <= 1e-9 {
                continue;
            }
            let (u, v) = uv(&xyz);
            let lab = xyz_to_lab(&xyz, &lab_white);
            points.push(GrayPoint { stops, nits: xyz.y, l: lab.x, a: lab.y, b: lab.z, duv: (u - wu).hypot(v - wv) });
            if stops.abs() < middle.2 {
                middle = (u, v, stops.abs());
            }
            // Keep luminance strictly rising (skips a clipped top and any reversal).
            if curve.last().map_or(true, |c| xyz.y.ln() > c.0 + 1e-9) {
                curve.push((xyz.y.ln(), u, v));
            }
        }
        GrayAxis { points, curve, middle: (middle.0, middle.1) }
    }

    /// The largest visible cast (L* ≥ [`VISIBLE_L`]) at whole stops, as reported.
    pub fn worst(&self) -> Option<GrayPoint> {
        self.whole_stops().into_iter().filter(|p| p.l >= VISIBLE_L).max_by(|a, b| a.duv.total_cmp(&b.duv))
    }

    /// The points at whole stops (for reports).
    pub fn whole_stops(&self) -> Vec<GrayPoint> {
        self.points.iter().filter(|p| p.stops.fract() == 0.0).copied().collect()
    }

    /// u′v′ of the LUT's gray at luminance `nits` (ends held, linear in log luminance).
    fn white_uv(&self, nits: f64) -> (f64, f64) {
        let c = &self.curve;
        let ly = nits.max(1e-12).ln();
        if ly <= c[0].0 {
            return (c[0].1, c[0].2);
        }
        let last = c[c.len() - 1];
        if ly >= last.0 {
            return (last.1, last.2);
        }
        let i = c.partition_point(|p| p.0 <= ly);
        let (a, b) = (c[i - 1], c[i]);
        let t = (ly - a.0) / (b.0 - a.0);
        (a.1 + t * (b.1 - a.1), a.2 + t * (b.2 - a.2))
    }

    /// The correction for `mode` at strength `amount` (0…1); `None` when off, or when the
    /// axis could not be measured.
    pub fn corrector(&self, mode: NeutralGray, amount: f64, output: &OutputSpace) -> Option<Corrector> {
        let amount = amount.clamp(0.0, 1.0);
        if mode == NeutralGray::Off || amount == 0.0 || self.curve.is_empty() {
            return None;
        }
        let fixed = match mode {
            NeutralGray::MiddleGray => Some(self.middle),
            _ => None,
        };
        Some(Corrector { axis: self.clone(), fixed, amount, output: *output })
    }
}

/// Removes the LUT's gray-axis cast from its display output (absolute XYZ, cd/m²).
#[derive(Debug, Clone)]
pub struct Corrector {
    axis: GrayAxis,
    /// MiddleGray mode: the one white used for every colour.
    fixed: Option<(f64, f64)>,
    amount: f64,
    output: OutputSpace,
}

impl Corrector {
    pub fn apply(&self, xyz: &Vec3) -> Vec3 {
        let (su, sv) = self.fixed.unwrap_or_else(|| self.axis.white_uv(xyz.y));
        let (wu, wv) = uv(&whites::D65.to_xyz());
        let (du, dv) = (su + self.amount * (wu - su), sv + self.amount * (wv - sv));
        let m = Cat::Bradford.matrix(uv_to_xy(su, sv), uv_to_xy(du, dv));
        // Keep the result displayable: OpenDRT cannot exceed the display gamut or peak.
        let peak = self.output.peak_nits();
        let rgb = (self.output.xyz_to_rgb() * (m * xyz)).map(|c| c.clamp(0.0, peak));
        self.output.rgb_to_xyz() * rgb
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lut::FnTransform;
    use crate::spaces::{CameraSpaceId, DisplayGamut, DisplayTransfer};

    fn rec709() -> OutputSpace {
        OutputSpace::new(DisplayGamut::Rec709, DisplayTransfer::Gamma { gamma: 2.4 })
    }

    /// A LUT from DWG/DI: a neutral tone curve with a display-RGB tint that depends on
    /// level (`tint(y)` multiplies R and divides B).
    fn tinted(tint: impl Fn(f64) -> f64 + Sync) -> impl Transform {
        let input = InputSpace::new(CameraSpaceId::DaVinciIntermediateDwg);
        let out = rec709();
        let to_disp = out.xyz_to_rgb() * input.rgb_to_xyz_d65();
        FnTransform(move |c: &Vec3| {
            let lin = to_disp * input.decode(c);
            let y = lin.map(|v| (v.max(0.0) / (v.max(0.0) + 0.6)).min(1.0));
            let t = tint(y.y);
            out.encode(&(Vec3::new(y.x * t, y.y, y.z / t) * out.peak_nits()).map(|v| v.min(out.peak_nits())))
        })
    }

    fn input() -> InputSpace {
        InputSpace::new(CameraSpaceId::DaVinciIntermediateDwg)
    }

    #[test]
    fn neutral_lut_needs_no_correction() {
        let lut = tinted(|_| 1.0);
        let ax = GrayAxis::measure(&lut, &input(), &rec709(), Interpolation::Tetrahedral);
        assert!(ax.worst().unwrap().duv < 1e-6, "{:?}", ax.worst());
        let c = ax.corrector(NeutralGray::Exposure, 1.0, &rec709()).unwrap();
        let x = Vec3::new(20.0, 15.0, 9.0);
        assert!((c.apply(&x) - x).norm() < 1e-9);
    }

    /// Every level makes the whole gray axis D65 with its luminance kept; middle gray only
    /// fixes 18% gray; amount 0.5 halves the cast.
    #[test]
    fn exposure_mode_neutralises_every_level() {
        let lut = tinted(|y| 1.0 + 0.08 * y);
        let out = rec709();
        let ax = GrayAxis::measure(&lut, &input(), &out, Interpolation::Tetrahedral);
        assert!(ax.worst().unwrap().duv > VISIBLE_DUV, "{:?}", ax.worst());
        assert_eq!(ax.worst().unwrap().cast(), "yellow", "more red, less blue");
        let full = ax.corrector(NeutralGray::Exposure, 1.0, &out).unwrap();
        let mid = ax.corrector(NeutralGray::MiddleGray, 1.0, &out).unwrap();
        let half = ax.corrector(NeutralGray::Exposure, 0.5, &out).unwrap();
        let d65 = uv(&whites::D65.to_xyz());
        let dist = |x: &Vec3| {
            let (u, v) = uv(x);
            (u - d65.0).hypot(v - d65.1)
        };
        for p in &ax.points {
            if p.nits > 0.95 * out.peak_nits() {
                continue;
            }
            let code = input().encode(&(input().xyz_d65_to_rgb() * (whites::D65.to_xyz() * (0.18 * 2f64.powf(p.stops)))));
            let x = out.rgb_to_xyz() * out.decode(&lut.apply_rgb(&code, Interpolation::Tetrahedral));
            let f = full.apply(&x);
            assert!(dist(&f) < 2e-4, "{} stops: {}", p.stops, dist(&f));
            assert!((f.y - x.y).abs() < 1e-6 * x.y.max(1.0), "luminance kept");
            assert!((dist(&half.apply(&x)) - 0.5 * p.duv).abs() < 0.1 * p.duv + 2e-4, "{} stops", p.stops);
            if p.stops == 0.0 {
                assert!(dist(&mid.apply(&x)) < 2e-4);
            }
            if p.stops == 3.0 {
                assert!(dist(&mid.apply(&x)) > 0.002, "middle gray mode leaves other levels tinted");
            }
        }
        assert!(ax.corrector(NeutralGray::Off, 1.0, &out).is_none());
    }
}
