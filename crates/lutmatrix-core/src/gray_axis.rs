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
//!
//! [`NeutralGray::WhiteGain`] is the colorist's version (D34): one per-channel gain in
//! display-linear light after the LUT, green kept at 1, that makes the LUT's white D65.
//! Darker levels keep whatever the LUT does relative to that white. The gain is scaled
//! down, all channels together, whenever it would take any LUT output above 1 (D35).

use serde::{Deserialize, Serialize};

use crate::difference::xyz_to_lab;
use crate::lut::{Interpolation, Transform};
use crate::math::{whites, Cat, Vec3, Xy};
use crate::spaces::{DisplayTransfer, InputSpace, OutputSpace};

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
    /// A per-channel linear gain after the LUT (green 1) that makes the LUT's white D65,
    /// as a Gain node after the LUT would.
    WhiteGain,
}

fn one() -> f64 {
    1.0
}

/// Diffuse white: a 90% reflector, log2(0.90 / 0.18) stops over 18% gray.
pub const DIFFUSE_WHITE_STOPS: f64 = 2.321928094887362;

impl NeutralGray {
    pub fn label(self) -> &'static str {
        match self {
            NeutralGray::Off => "off",
            NeutralGray::MiddleGray => "middle gray",
            NeutralGray::Exposure => "every level",
            NeutralGray::WhiteGain => "white gain",
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
    fn new(stops: f64, xyz: &Vec3, output: &OutputSpace) -> GrayPoint {
        let d65 = whites::D65.to_xyz();
        let (wu, wv) = uv(&d65);
        let (u, v) = uv(xyz);
        let lab = xyz_to_lab(xyz, &(d65 * output.peak_nits()));
        GrayPoint { stops, nits: xyz.y, l: lab.x, a: lab.y, b: lab.z, duv: (u - wu).hypot(v - wv) }
    }

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
    /// (stops, display-linear RGB in cd/m²) of every measured point.
    rgb: Vec<(f64, Vec3)>,
    /// The LUT's largest output per channel, display-linear cd/m².
    max_rgb: Vec3,
}

/// The white used by [`NeutralGray::WhiteGain`] and the gain that balances it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct WhiteGain {
    /// Exposure of the white actually used (the requested one, or the brightest gray inside
    /// the LUT's input range if that is lower).
    pub stops: f64,
    /// The LUT's output there before the gain (u′v′ distance from D65).
    pub duv: f64,
    /// Gain on display-linear R, G, B (G = 1 unless scaled down for headroom).
    pub linear: [f64; 3],
    /// Factor applied to all three channels so that no LUT output exceeds signal 1
    /// (1 = no scaling needed).
    #[serde(default = "one")]
    pub headroom: f64,
    /// The same gain on the encoded signal for a pure power display (linear^(1/γ)), i.e.
    /// what a Gain control after the LUT should be set to; `None` for PQ.
    pub signal: Option<[f64; 3]>,
}

impl GrayAxis {
    pub fn measure(lut: &dyn Transform, input: &InputSpace, output: &OutputSpace, interp: Interpolation) -> GrayAxis {
        let d65 = whites::D65.to_xyz();
        let (wu, wv) = uv(&d65);
        let to_cam = input.xyz_d65_to_rgb();
        let disp = output.rgb_to_xyz();
        let mut points = Vec::new();
        let mut curve: Vec<(f64, f64, f64)> = Vec::new();
        let mut middle = (wu, wv, f64::INFINITY);
        let mut rgb = Vec::new();
        for k in 0..=80 {
            let stops = -10.0 + 0.25 * k as f64;
            let code = input.encode(&(to_cam * (d65 * (0.18 * 2f64.powf(stops)))));
            if !lut.in_domain_rgb(&code) {
                continue;
            }
            let lin = output.decode(&lut.apply_rgb(&code, interp));
            let xyz = disp * lin;
            if !xyz.iter().all(|v| v.is_finite()) || xyz.y <= 1e-9 {
                continue;
            }
            let (u, v) = uv(&xyz);
            points.push(GrayPoint::new(stops, &xyz, output));
            rgb.push((stops, lin));
            if stops.abs() < middle.2 {
                middle = (u, v, stops.abs());
            }
            // Keep luminance strictly rising (skips a clipped top and any reversal).
            if curve.last().is_none_or(|c| xyz.y.ln() > c.0 + 1e-9) {
                curve.push((xyz.y.ln(), u, v));
            }
        }
        // The LUT's largest output per channel: exact for a table, else sampled over 0–1.
        let max_code = lut.output_max().unwrap_or_else(|| {
            let n = 17;
            let v = |i: usize| i as f64 / (n - 1) as f64;
            let mut m = [f64::NEG_INFINITY; 3];
            for r in 0..n {
                for g in 0..n {
                    for b in 0..n {
                        let o = lut.apply_rgb(&Vec3::new(v(r), v(g), v(b)), interp);
                        m = [m[0].max(o.x), m[1].max(o.y), m[2].max(o.z)];
                    }
                }
            }
            m
        });
        let max_rgb = output.decode(&Vec3::from(max_code));
        GrayAxis { points, curve, middle: (middle.0, middle.1), rgb, max_rgb }
    }

    /// The largest visible cast (L* ≥ [`VISIBLE_L`]) at whole stops, as reported.
    pub fn worst(&self) -> Option<GrayPoint> {
        self.whole_stops().into_iter().filter(|p| p.l >= VISIBLE_L).max_by(|a, b| a.duv.total_cmp(&b.duv))
    }

    /// The whole-stop points as they come out of `corrector` (for reports).
    pub fn corrected_whole_stops(&self, corrector: &Corrector, output: &OutputSpace) -> Vec<GrayPoint> {
        self.rgb
            .iter()
            .filter(|(s, _)| s.fract() == 0.0)
            .map(|(s, rgb)| GrayPoint::new(*s, &corrector.apply(&(output.rgb_to_xyz() * rgb)), output))
            .collect()
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

    /// The per-channel display-linear gain (green 1) that moves the LUT's gray at
    /// `stops` (or the brightest measured gray below it) `amount` of the way to D65.
    pub fn white_gain(&self, stops: f64, amount: f64, output: &OutputSpace) -> Option<WhiteGain> {
        let &(at, w) = self.rgb.iter().rev().find(|(s, _)| *s <= stops + 1e-9)?;
        if w.min() <= 0.0 {
            return None;
        }
        let xyz = output.rgb_to_xyz() * w;
        let (su, sv) = uv(&xyz);
        let (du, dv) = uv(&whites::D65.to_xyz());
        let a = amount.clamp(0.0, 1.0);
        let t = output.xyz_to_rgb() * uv_to_xy(su + a * (du - su), sv + a * (dv - sv)).to_xyz();
        let g = Vec3::new(t.x / w.x, t.y / w.y, t.z / w.z) / (t.y / w.y);
        // Signal 1 in display-linear terms; scale everything down if any channel of any LUT
        // output would pass it.
        let limit = output.decode_channel(1.0);
        let headroom = (0..3)
            .filter(|&c| self.max_rgb[c] > 0.0)
            .map(|c| limit / (g[c] * self.max_rgb[c]))
            .fold(1.0f64, f64::min);
        let g = g * headroom;
        let signal = match output.transfer {
            DisplayTransfer::Gamma { gamma } => Some([g.x, g.y, g.z].map(|v| v.powf(1.0 / gamma))),
            _ => None,
        };
        Some(WhiteGain { stops: at, duv: (su - du).hypot(sv - dv), linear: [g.x, g.y, g.z], headroom, signal })
    }

    /// The correction for `mode` at strength `amount` (0…1); `None` when off, or when the
    /// axis could not be measured. `white_stops` is the white for [`NeutralGray::WhiteGain`].
    pub fn corrector(&self, mode: NeutralGray, amount: f64, white_stops: f64, output: &OutputSpace) -> Option<Corrector> {
        let amount = amount.clamp(0.0, 1.0);
        if mode == NeutralGray::Off || amount == 0.0 || self.curve.is_empty() {
            return None;
        }
        let kind = match mode {
            NeutralGray::MiddleGray => Kind::Adapt { axis: self.clone(), fixed: Some(self.middle) },
            NeutralGray::Exposure => Kind::Adapt { axis: self.clone(), fixed: None },
            NeutralGray::WhiteGain => Kind::Gain(self.white_gain(white_stops, amount, output)?),
            NeutralGray::Off => unreachable!(),
        };
        Some(Corrector { kind, amount, output: *output })
    }
}

#[derive(Debug, Clone)]
enum Kind {
    /// Bradford from the LUT's gray (at the colour's luminance, or `fixed`) to D65.
    Adapt { axis: GrayAxis, fixed: Option<(f64, f64)> },
    /// One display-linear gain.
    Gain(WhiteGain),
}

/// Removes the LUT's gray-axis cast from its display output (absolute XYZ, cd/m²).
#[derive(Debug, Clone)]
pub struct Corrector {
    kind: Kind,
    amount: f64,
    output: OutputSpace,
}

impl Corrector {
    pub fn apply(&self, xyz: &Vec3) -> Vec3 {
        let rgb = match &self.kind {
            Kind::Adapt { axis, fixed } => {
                let (su, sv) = fixed.unwrap_or_else(|| axis.white_uv(xyz.y));
                let (wu, wv) = uv(&whites::D65.to_xyz());
                let (du, dv) = (su + self.amount * (wu - su), sv + self.amount * (wv - sv));
                let m = Cat::Bradford.matrix(uv_to_xy(su, sv), uv_to_xy(du, dv));
                self.output.xyz_to_rgb() * (m * xyz)
            }
            // The amount is already in the gain.
            Kind::Gain(g) => (self.output.xyz_to_rgb() * xyz).component_mul(&Vec3::from(g.linear)),
        };
        // Keep the result displayable: OpenDRT cannot exceed the display gamut or peak.
        let peak = self.output.peak_nits();
        self.output.rgb_to_xyz() * rgb.map(|c| c.clamp(0.0, peak))
    }

    /// The gain, in [`NeutralGray::WhiteGain`] mode.
    pub fn white_gain(&self) -> Option<WhiteGain> {
        match self.kind {
            Kind::Gain(g) => Some(g),
            Kind::Adapt { .. } => None,
        }
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
        let c = ax.corrector(NeutralGray::Exposure, 1.0, DIFFUSE_WHITE_STOPS, &rec709()).unwrap();
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
        let full = ax.corrector(NeutralGray::Exposure, 1.0, DIFFUSE_WHITE_STOPS, &out).unwrap();
        let mid = ax.corrector(NeutralGray::MiddleGray, 1.0, DIFFUSE_WHITE_STOPS, &out).unwrap();
        let half = ax.corrector(NeutralGray::Exposure, 0.5, DIFFUSE_WHITE_STOPS, &out).unwrap();
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
        assert!(ax.corrector(NeutralGray::Off, 1.0, DIFFUSE_WHITE_STOPS, &out).is_none());
    }

    /// White gain: one display-linear gain, green 1, puts the LUT's diffuse white on D65;
    /// grays below keep their cast relative to it (here they turn cool, as the tint grows
    /// with level); the signal gain is the linear gain ^ (1/2.4); half amount halves it.
    #[test]
    fn white_gain_balances_the_white_only() {
        let lut = tinted(|y| 1.0 + 0.08 * y);
        let out = rec709();
        let ax = GrayAxis::measure(&lut, &input(), &out, Interpolation::Tetrahedral);
        let c = ax.corrector(NeutralGray::WhiteGain, 1.0, DIFFUSE_WHITE_STOPS, &out).unwrap();
        let g = c.white_gain().unwrap();
        assert_eq!(g.stops, 2.25, "nearest measured level at or below diffuse white");
        assert!((g.linear[1] - g.headroom).abs() < 1e-12, "green is 1 before headroom scaling");
        assert!(g.linear[0] < g.linear[1] && g.linear[2] > g.linear[1], "{:?}", g.linear);
        let sig = g.signal.unwrap();
        for (s, l) in sig.iter().zip(g.linear) {
            assert!((s - l.powf(1.0 / 2.4)).abs() < 1e-12);
        }
        let d65 = uv(&whites::D65.to_xyz());
        let at = |stops: f64| {
            let code = input().encode(&(input().xyz_d65_to_rgb() * (whites::D65.to_xyz() * (0.18 * 2f64.powf(stops)))));
            out.rgb_to_xyz() * out.decode(&lut.apply_rgb(&code, Interpolation::Tetrahedral))
        };
        let w = c.apply(&at(2.25));
        let (u, v) = uv(&w);
        assert!((u - d65.0).hypot(v - d65.1) < 1e-6, "white is D65");
        let m = c.apply(&at(0.0));
        let lab = xyz_to_lab(&m, &(whites::D65.to_xyz() * out.peak_nits()));
        assert!(lab.z < -0.5, "middle gray turns cool: b* {}", lab.z);
        let half = ax.white_gain(DIFFUSE_WHITE_STOPS, 0.5, &out).unwrap();
        assert!(half.linear[2] / half.linear[1] > 1.0 && half.linear[2] / half.linear[1] < g.linear[2] / g.linear[1]);
    }

    /// A LUT whose blue output already reaches 1 (signal): balancing a warm white needs
    /// blue gain > 1, so the whole gain is scaled down until blue × gain peaks at exactly 1.
    #[test]
    fn white_gain_never_exceeds_signal_one() {
        let out = rec709();
        // Warm (R > G > B) at every level, every channel reaching code 1 at the top.
        let to_disp = out.xyz_to_rgb() * input().rgb_to_xyz_d65();
        let lut = FnTransform(move |c: &Vec3| {
            let y = (to_disp * input().decode(c)).map(|v| (1.05 * v.max(0.0) / (v.max(0.0) + 0.6)).min(1.0));
            out.encode(&(Vec3::new((1.12 * y.x).min(1.0), (1.06 * y.y).min(1.0), y.z) * out.peak_nits()))
        });
        let ax = GrayAxis::measure(&lut, &input(), &out, Interpolation::Tetrahedral);
        let g = ax.white_gain(DIFFUSE_WHITE_STOPS, 1.0, &out).unwrap();
        assert!(g.headroom < 1.0, "{g:?}");
        let peak = (0..3).map(|c| g.linear[c] * ax.max_rgb[c]).fold(0.0, f64::max);
        assert!((peak - out.peak_nits()).abs() < 1e-9 * out.peak_nits(), "brightest channel lands on 1: {peak}");
        // A neutral LUT that never reaches 1 needs no scaling.
        let soft = FnTransform(|c: &Vec3| *c * 0.8);
        let ax = GrayAxis::measure(&soft, &input(), &out, Interpolation::Tetrahedral);
        assert_eq!(ax.white_gain(DIFFUSE_WHITE_STOPS, 1.0, &out).unwrap().headroom, 1.0);
    }
}
