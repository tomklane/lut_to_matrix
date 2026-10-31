//! TPS3D: a per-channel tone curve through the grays, then a thin-plate spline, no
//! matrix (DECISIONS.md D44). Built like the user's Python matrix app's "TPS3D Fast"
//! (`rbf_solver.py` / `mf-solve`), with the curve first (the order that predicted
//! colours between the patches better and keeps clipped white white):
//!
//! - Tone curve: per channel, monotone cubic (Fritsch–Carlson) through the usable gray
//!   patches, camera DI → target DI, continued linearly beyond the end grays. It does the
//!   white balance, exposure and contrast of the grays, so they reach the neutral axis
//!   before the spline.
//! - Space: DaVinci Intermediate–encoded DWG, in and out.
//! - Kernel: the 3D biharmonic φ(r) = r with an affine part, per output channel
//!   `f(d) = a0 + a·d + Σ w_i |d − c_i|`, stored here as the residual `f(d) − d`;
//!   smoothing 0 passes through every control point exactly.
//! - Control points: the chart's colour patches (all 18, or the 6 primaries) and its
//!   usable grays, after the curve → target; synthetic neutrals `(L,L,L) → (L,L,L)` at
//!   each patch's mean and at DI 0.01, 0.02, DI(1.0) − 0.01 and DI(1.0); the eight
//!   corners of the DI cube mapped to themselves.
//! - Gray correction (bias subtraction), applied wherever the spline is evaluated:
//!   `out = f(d) − (f(L,L,L) − (L,L,L))`, L = mean(d). The spline never moves the neutral
//!   axis, so equal R, G, B (clipped white) stays exactly as it came in.

use nalgebra::{DMatrix, DVector};
use serde::{Deserialize, Serialize};

use crate::math::Vec3;
use crate::spaces::curves::{di_decode, di_encode};
use crate::spaces::davinci;

/// Per-channel monotone cubic through the grays (DI in → DI out), linear beyond the ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrayCurve {
    /// Per channel: knot positions (increasing), values, slopes.
    pub x: [Vec<f64>; 3],
    pub y: [Vec<f64>; 3],
    pub m: [Vec<f64>; 3],
}

impl GrayCurve {
    /// Through `(camera, target)` gray pairs (DI). Per channel, points that don't
    /// increase in both are dropped; None with fewer than 2 left in any channel.
    pub fn fit(pairs: &[(Vec3, Vec3)]) -> Option<GrayCurve> {
        let mut x: [Vec<f64>; 3] = Default::default();
        let mut y: [Vec<f64>; 3] = Default::default();
        let mut m: [Vec<f64>; 3] = Default::default();
        for k in 0..3 {
            let mut p: Vec<(f64, f64)> = pairs.iter().map(|(a, b)| (a[k], b[k])).filter(|(a, b)| a.is_finite() && b.is_finite()).collect();
            p.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut q: Vec<(f64, f64)> = Vec::new();
            for (a, b) in p {
                if q.last().is_none_or(|l| a > l.0 + 1e-6 && b > l.1) {
                    q.push((a, b));
                }
            }
            if q.len() < 2 {
                return None;
            }
            let n = q.len();
            let d: Vec<f64> = q.windows(2).map(|w| (w[1].1 - w[0].1) / (w[1].0 - w[0].0)).collect();
            let mut s: Vec<f64> = (0..n)
                .map(|i| match i {
                    0 => d[0],
                    i if i == n - 1 => d[n - 2],
                    i => (d[i - 1] + d[i]) / 2.0,
                })
                .collect();
            // Fritsch–Carlson: keep each segment monotone.
            for i in 0..n - 1 {
                let (a, b) = (s[i] / d[i], s[i + 1] / d[i]);
                let r = a * a + b * b;
                if r > 9.0 {
                    let t = 3.0 / r.sqrt();
                    s[i] = t * a * d[i];
                    s[i + 1] = t * b * d[i];
                }
            }
            x[k] = q.iter().map(|v| v.0).collect();
            y[k] = q.iter().map(|v| v.1).collect();
            m[k] = s;
        }
        Some(GrayCurve { x, y, m })
    }

    pub fn eval(&self, k: usize, v: f64) -> f64 {
        let (x, y, m) = (&self.x[k], &self.y[k], &self.m[k]);
        let n = x.len();
        if v <= x[0] {
            return y[0] + m[0] * (v - x[0]);
        }
        if v >= x[n - 1] {
            return y[n - 1] + m[n - 1] * (v - x[n - 1]);
        }
        let i = (0..n - 1).find(|&i| v < x[i + 1]).unwrap_or(n - 2);
        let h = x[i + 1] - x[i];
        let t = (v - x[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * y[i] + (t3 - 2.0 * t2 + t) * h * m[i] + (-2.0 * t3 + 3.0 * t2) * y[i + 1] + (t3 - t2) * h * m[i + 1]
    }

    pub fn apply(&self, d: &Vec3) -> Vec3 {
        Vec3::new(self.eval(0, d.x), self.eval(1, d.y), self.eval(2, d.z))
    }
}

/// The fitted correction: the gray curve, then the spline
/// residual(d) = affine · [1, d] + Σ weight_i · |d − centre_i|, in DI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tps {
    pub curve: Option<GrayCurve>,
    pub centres: Vec<[f64; 3]>,
    pub weights: Vec<[f64; 3]>,
    /// Per output channel: constant, then the coefficients of d.x, d.y, d.z.
    pub affine: [[f64; 4]; 3],
    pub smoothing: f64,
}

/// One control point: where it is and where it should go (both DI-encoded DWG), and
/// its weight (only used with smoothing).
#[derive(Debug, Clone, Copy)]
pub struct TpsPoint {
    pub from: Vec3,
    pub to: Vec3,
    pub weight: f64,
}

/// Weight of the identity anchors relative to a patch (with smoothing).
const ANCHOR_WEIGHT: f64 = 50.0;

impl Tps {
    /// `f(d) − d` without the gray correction.
    pub fn residual(&self, d: &Vec3) -> Vec3 {
        let mut r = Vec3::from_fn(|k, _| self.affine[k][0] + self.affine[k][1] * d.x + self.affine[k][2] * d.y + self.affine[k][3] * d.z);
        for (c, w) in self.centres.iter().zip(&self.weights) {
            let dist = ((d.x - c[0]).powi(2) + (d.y - c[1]).powi(2) + (d.z - c[2]).powi(2)).sqrt();
            r += Vec3::from(*w) * dist;
        }
        r
    }

    /// The spline with the gray correction, DI in and out (after the curve):
    /// `f(d) − (f(L,L,L) − (L,L,L))`.
    pub fn spline_di(&self, d: &Vec3) -> Vec3 {
        let l = Vec3::repeat((d.x + d.y + d.z) / 3.0);
        d + self.residual(d) - self.residual(&l)
    }

    /// The whole correction, DI in and out: the gray curve, then the spline.
    pub fn apply_di(&self, d: &Vec3) -> Vec3 {
        match &self.curve {
            Some(c) => self.spline_di(&c.apply(d)),
            None => self.spline_di(d),
        }
    }

    /// The spline applied to a colour (CIE XYZ D65 in and out).
    pub fn apply_xyz(&self, xyz: &Vec3) -> Vec3 {
        let d = (davinci::xyz_to_dwg() * xyz).map(di_encode);
        davinci::dwg_to_xyz() * self.apply_di(&d).map(di_decode)
    }
}

/// The identity anchors for patches at DI positions `colours`: synthetic neutrals at
/// each one's mean (no two within 1e-4) and at four endpoints, then the eight DI cube
/// corners.
pub fn identity_anchors(colours: &[Vec3]) -> Vec<TpsPoint> {
    // The level itself, de-duplicated at 4 decimals (as TPS3D Fast does).
    let mut lums: Vec<f64> = Vec::new();
    let mut keys: Vec<i64> = Vec::new();
    let mut push = |l: f64| {
        let q = (l * 1e4).round() as i64;
        if !keys.contains(&q) {
            keys.push(q);
            lums.push(l);
        }
    };
    for c in colours {
        push((c.x + c.y + c.z) / 3.0);
    }
    let clip = di_encode(1.0);
    for l in [0.01, 0.02, clip - 0.01, clip] {
        push(l);
    }
    let mut pts: Vec<TpsPoint> = lums.iter().map(|&l| Vec3::repeat(l)).map(|d| TpsPoint { from: d, to: d, weight: ANCHOR_WEIGHT }).collect();
    for k in 0..8 {
        let d = Vec3::new((k & 1) as f64, ((k >> 1) & 1) as f64, ((k >> 2) & 1) as f64);
        pts.push(TpsPoint { from: d, to: d, weight: ANCHOR_WEIGHT });
    }
    pts
}

/// Drop points that sit (almost) on an earlier one: two control points in the same
/// place make the system singular.
pub fn dedupe(points: Vec<TpsPoint>) -> Vec<TpsPoint> {
    let mut out: Vec<TpsPoint> = Vec::with_capacity(points.len());
    for p in points {
        if !out.iter().any(|q| (q.from - p.from).norm() < 1e-6) {
            out.push(p);
        }
    }
    out
}

/// Fit the spline. None if the system is singular (e.g. the points lie on a plane).
pub fn fit_tps(points: &[TpsPoint], smoothing: f64) -> Option<Tps> {
    let n = points.len();
    if n < 5 {
        return None;
    }
    let lambda = smoothing.max(0.0);
    let m = n + 4;
    let mut a = DMatrix::<f64>::zeros(m, m);
    for i in 0..n {
        for j in 0..n {
            a[(i, j)] = (points[i].from - points[j].from).norm();
        }
        // φ(r) = r is conditionally *negative* definite (the energy kernel is −r), so the
        // smoothing term goes in with a minus sign.
        a[(i, i)] -= lambda / points[i].weight.max(1e-6);
        let c = points[i].from;
        for (k, v) in [1.0, c.x, c.y, c.z].into_iter().enumerate() {
            a[(i, n + k)] = v;
            a[(n + k, i)] = v;
        }
    }
    let lu = a.lu();
    let mut weights = vec![[0.0; 3]; n];
    let mut affine = [[0.0; 4]; 3];
    for ch in 0..3 {
        let b = DVector::from_fn(m, |i, _| if i < n { points[i].to[ch] - points[i].from[ch] } else { 0.0 });
        let x = lu.solve(&b)?;
        if !x.iter().all(|v| v.is_finite()) {
            return None;
        }
        for (i, w) in weights.iter_mut().enumerate() {
            w[ch] = x[i];
        }
        for k in 0..4 {
            affine[ch][k] = x[n + k];
        }
    }
    Some(Tps { curve: None, centres: points.iter().map(|p| p.from.into()).collect(), weights, affine, smoothing: lambda })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts() -> Vec<TpsPoint> {
        // A curved distortion of a lattice (off the neutral axis).
        let mut v = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                for k in 0..3 {
                    if i == j && j == k {
                        continue;
                    }
                    let from = Vec3::new(0.15 + 0.15 * i as f64, 0.15 + 0.15 * j as f64, 0.15 + 0.15 * k as f64);
                    let to = from + Vec3::new(0.03 * (from.y * 6.0).sin(), -0.02 * from.x * from.z * 10.0, 0.01);
                    v.push(TpsPoint { from, to, weight: 1.0 });
                }
            }
        }
        v
    }

    #[test]
    fn interpolates_exactly_without_smoothing_and_less_with_it() {
        let p = pts();
        let t = fit_tps(&p, 0.0).unwrap();
        for q in &p {
            let got = q.from + t.residual(&q.from);
            assert!((got - q.to).norm() < 1e-9, "{got} vs {}", q.to);
        }
        let s = fit_tps(&p, 0.5).unwrap();
        let err = |t: &Tps| p.iter().map(|q| (q.from + t.residual(&q.from) - q.to).norm()).sum::<f64>();
        assert!(err(&s) > err(&t) + 1e-6);
    }

    #[test]
    fn gray_correction_leaves_the_neutral_axis_alone() {
        let mut p = pts();
        let colours: Vec<Vec3> = p.iter().map(|q| q.from).collect();
        p.extend(identity_anchors(&colours));
        let p = dedupe(p);
        let t = fit_tps(&p, 0.0).unwrap();
        // Anywhere on the axis, even far outside: exactly unchanged.
        for l in [-0.1, 0.0, 0.3, 0.55, 0.8, 1.4] {
            let d = Vec3::repeat(l);
            assert!((t.spline_di(&d) - d).norm() < 1e-12, "{l}");
        }
        // With the anchors the bias at the colour patches is ~0, so they still land.
        for q in p.iter().take(colours.len()) {
            assert!((t.spline_di(&q.from) - q.to).norm() < 1e-9);
        }
        // Corners and endpoint anchors present.
        assert!(p.iter().any(|q| q.from == Vec3::new(1.0, 0.0, 1.0)));
        assert!(p.iter().any(|q| (q.from - Vec3::repeat(0.02)).norm() < 1e-12));
    }

    #[test]
    fn gray_curve_goes_through_the_grays_and_stays_monotonic() {
        let pairs: Vec<(Vec3, Vec3)> = [0.1, 0.25, 0.38, 0.47, 0.55, 0.62]
            .iter()
            .map(|&v: &f64| (Vec3::new(v * 0.8, v, v * 1.1), Vec3::new(v.powf(0.9), v + 0.02, v * 0.95)))
            .collect();
        let c = GrayCurve::fit(&pairs).unwrap();
        for (a, b) in &pairs {
            assert!((c.apply(a) - b).norm() < 1e-12);
        }
        for k in 0..3 {
            let mut prev = f64::NEG_INFINITY;
            for i in -100..300 {
                let v = c.eval(k, i as f64 * 0.005);
                assert!(v > prev, "channel {k} at {i}");
                prev = v;
            }
        }
    }
}
