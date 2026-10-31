//! A thin-plate spline (TPS) refinement after the chart matrix (DECISIONS.md D44).
//!
//! The matrix leaves errors a 3 × 3 can't remove (spectral differences: under water
//! about 6 ΔE). A 3D thin-plate spline bends colour space so each patch, as the matrix
//! leaves it, moves towards its target, and everything in between follows smoothly.
//!
//! - Space: DaVinci Intermediate–encoded DWG after the matrix, so the bending is about
//!   evenly perceptible from shadows to highlights, and the DCTL needs no other space.
//! - Kernel: the 3D biharmonic φ(r) = r, with an affine part, solved as the residual
//!   `target − matrix output`; `smoothing` (λ) trades exactness at the patches for
//!   smoothness (λ = 0 interpolates every patch exactly, which follows their noise).
//! - Anchors: neutrals from black to 32 × the chart's white are pinned with zero residual,
//!   so grays, clipped white and everything far brighter than the chart stay as the
//!   matrix makes them.
//! - Far from the chart the kernels' sum levels off (the side conditions remove its
//!   growth), so out-of-chart colours move about as much as their nearest patches do.

use nalgebra::{DMatrix, DVector};
use serde::{Deserialize, Serialize};

use crate::math::Vec3;
use crate::spaces::curves::{di_decode, di_encode};
use crate::spaces::davinci;

/// The fitted spline: residual(d) = affine · [1, d] + Σ weight_i · |d − centre_i|, in DI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tps {
    pub centres: Vec<[f64; 3]>,
    pub weights: Vec<[f64; 3]>,
    /// Per output channel: constant, then the coefficients of d.x, d.y, d.z.
    pub affine: [[f64; 4]; 3],
    pub smoothing: f64,
}

/// One point the spline is fitted to: where the matrix puts it and where it should be
/// (both DI-encoded DWG), and its weight.
#[derive(Debug, Clone, Copy)]
pub struct TpsPoint {
    pub from: Vec3,
    pub to: Vec3,
    pub weight: f64,
}

/// Weight of the neutral anchors relative to a patch.
const ANCHOR_WEIGHT: f64 = 50.0;

impl Tps {
    pub fn residual(&self, d: &Vec3) -> Vec3 {
        let mut r = Vec3::from_fn(|k, _| self.affine[k][0] + self.affine[k][1] * d.x + self.affine[k][2] * d.y + self.affine[k][3] * d.z);
        for (c, w) in self.centres.iter().zip(&self.weights) {
            let dist = ((d.x - c[0]).powi(2) + (d.y - c[1]).powi(2) + (d.z - c[2]).powi(2)).sqrt();
            r += Vec3::from(*w) * dist;
        }
        r
    }

    /// The spline applied to a colour (CIE XYZ D65 in and out).
    pub fn apply_xyz(&self, xyz: &Vec3) -> Vec3 {
        let d = (davinci::xyz_to_dwg() * xyz).map(di_encode);
        let o = d + self.residual(&d);
        davinci::dwg_to_xyz() * o.map(di_decode)
    }
}

/// The neutral anchors for a chart whose white patch is at linear `white` (target scale).
pub fn neutral_anchors(white: f64) -> Vec<TpsPoint> {
    let white = if white.is_finite() && white > 0.0 { white } else { 0.9 };
    std::iter::once(0.0)
        .chain((1..=5).map(|k| white * f64::powi(2.0, k)))
        .map(|l| {
            let d = Vec3::repeat(di_encode(l));
            TpsPoint { from: d, to: d, weight: ANCHOR_WEIGHT }
        })
        .collect()
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
    Some(Tps { centres: points.iter().map(|p| p.from.into()).collect(), weights, affine, smoothing: lambda })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts() -> Vec<TpsPoint> {
        // A curved distortion of a lattice.
        let mut v = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                for k in 0..3 {
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
    fn reproduces_an_affine_map_and_keeps_anchors() {
        // A pure affine residual is reproduced everywhere (no kernel needed).
        let f = |d: Vec3| Vec3::new(0.01 + 0.1 * d.x, -0.02 * d.y, 0.05 * d.z - 0.01);
        let p: Vec<TpsPoint> = pts().into_iter().map(|q| TpsPoint { to: q.from + f(q.from), ..q }).collect();
        let t = fit_tps(&p, 0.0).unwrap();
        let x = Vec3::new(0.9, 0.05, 0.6);
        assert!((t.residual(&x) - f(x)).norm() < 1e-9);
        // Anchors: zero residual on the neutral axis where they sit.
        let mut p = pts();
        p.extend(neutral_anchors(0.9));
        let t = fit_tps(&p, 1e-3).unwrap();
        for a in neutral_anchors(0.9).iter().skip(2) {
            assert!(t.residual(&a.from).norm() < 5e-3, "{}", t.residual(&a.from));
        }
    }
}
