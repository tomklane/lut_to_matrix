//! Matrix fitting in linear XYZ (all f64).
//!
//! - [`least_squares`]: unconstrained, minimises Σ‖M·xᵢ − yᵢ‖².
//! - [`least_squares_constrained`]: the same subject to M·w = t (neutral preserved).
//! - [`refine`]: Levenberg–Marquardt on per-sample perceptual error, optionally
//!   keeping M·w = t exactly by optimising in the constraint's null space.

use nalgebra::{DMatrix, DVector, Matrix4, Vector4};

use crate::math::{Mat3, Vec3};

/// Weighted least squares: M = argmin Σ wᵢ‖M xᵢ − yᵢ‖² (`wt` = None → all 1).
pub fn least_squares(x: &[Vec3], y: &[Vec3], wt: Option<&[f64]>) -> Option<Mat3> {
    let mut xtx = Mat3::zeros();
    let mut ytx = Mat3::zeros();
    for (i, (xi, yi)) in x.iter().zip(y).enumerate() {
        let w = wt.map_or(1.0, |w| w[i]);
        xtx += xi * xi.transpose() * w;
        ytx += yi * xi.transpose() * w;
    }
    Some(ytx * xtx.try_inverse()?)
}

/// Weighted least squares subject to the exact linear constraint M·w = t (solved row
/// by row with a Lagrange multiplier).
pub fn least_squares_constrained(
    x: &[Vec3],
    y: &[Vec3],
    wt: Option<&[f64]>,
    w: &Vec3,
    t: &Vec3,
) -> Option<Mat3> {
    let mut xtx = Mat3::zeros();
    for (i, xi) in x.iter().enumerate() {
        xtx += xi * xi.transpose() * wt.map_or(1.0, |w| w[i]);
    }
    let mut m = Mat3::zeros();
    for row in 0..3 {
        let mut xty = Vec3::zeros();
        for (i, (xi, yi)) in x.iter().zip(y).enumerate() {
            xty += xi * yi[row] * wt.map_or(1.0, |w| w[i]);
        }
        // [XᵀX  w] [r]   [Xᵀy]
        // [wᵀ   0] [λ] = [t  ]
        let mut a = Matrix4::zeros();
        a.fixed_view_mut::<3, 3>(0, 0).copy_from(&xtx);
        a.fixed_view_mut::<3, 1>(0, 3).copy_from(w);
        a.fixed_view_mut::<1, 3>(3, 0).copy_from(&w.transpose());
        let b = Vector4::new(xty.x, xty.y, xty.z, t[row]);
        let sol = a.lu().solve(&b)?;
        m.set_row(row, &sol.fixed_rows::<3>(0).transpose());
    }
    Some(m)
}

/// Orthonormal basis (3×2) of the plane orthogonal to `w`.
fn null_basis(w: &Vec3) -> nalgebra::Matrix3x2<f64> {
    let n = w.normalize();
    let a = if n.x.abs() < 0.9 { Vec3::x() } else { Vec3::y() };
    let u = (a - n * n.dot(&a)).normalize();
    let v = n.cross(&u);
    nalgebra::Matrix3x2::from_columns(&[u, v])
}

/// Matrix parametrisation for the optimiser.
pub enum Param {
    /// All nine elements free.
    Free,
    /// Rows constrained so M·w = t: row k = base_k + N·z_k (6 parameters).
    Neutral { w: Vec3 },
}

impl Param {
    fn n(&self) -> usize {
        match self {
            Param::Free => 9,
            Param::Neutral { .. } => 6,
        }
    }

    fn to_matrix(&self, base: &Mat3, p: &[f64]) -> Mat3 {
        match self {
            Param::Free => Mat3::from_row_slice(p),
            Param::Neutral { w } => {
                let nb = null_basis(w);
                let mut m = *base;
                for row in 0..3 {
                    let dz = nalgebra::Vector2::new(p[2 * row], p[2 * row + 1]);
                    let r = base.row(row).transpose() + nb * dz;
                    m.set_row(row, &r.transpose());
                }
                m
            }
        }
    }

    fn initial(&self, base: &Mat3) -> Vec<f64> {
        match self {
            Param::Free => base.transpose().as_slice().to_vec(), // row-major
            Param::Neutral { .. } => vec![0.0; 6],
        }
    }
}

/// Minimise Σ rᵢ(M)² with Levenberg–Marquardt (Marquardt's diagonal scaling,
/// central-difference Jacobian). `residuals` returns one residual per sample.
pub fn refine(start: &Mat3, param: &Param, residuals: &dyn Fn(&Mat3) -> Vec<f64>) -> Mat3 {
    let base = *start;
    let n = param.n();
    let mut p = param.initial(&base);
    let eval = |p: &[f64]| residuals(&param.to_matrix(&base, p));
    let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();

    let mut r = eval(&p);
    let mut c = cost(&r);
    let mut lambda = 1e-3;
    for _iter in 0..200 {
        let m = r.len();
        let mut j = DMatrix::<f64>::zeros(m, n);
        for k in 0..n {
            let h = 1e-6 * (1.0 + p[k].abs());
            let mut pp = p.clone();
            pp[k] += h;
            let rp = eval(&pp);
            pp[k] -= 2.0 * h;
            let rm = eval(&pp);
            for i in 0..m {
                j[(i, k)] = (rp[i] - rm[i]) / (2.0 * h);
            }
        }
        let rv = DVector::from_vec(r.clone());
        let jtj = j.transpose() * &j;
        let g = j.transpose() * &rv;
        if g.amax() < 1e-12 {
            break;
        }
        let mut improved = false;
        for _ in 0..30 {
            let mut a = jtj.clone();
            for d in 0..n {
                a[(d, d)] += lambda * jtj[(d, d)].max(1e-12);
            }
            let Some(step) = a.lu().solve(&(-&g)) else {
                lambda *= 10.0;
                continue;
            };
            let pn: Vec<f64> = p.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
            let rn = eval(&pn);
            let cn = cost(&rn);
            if cn < c {
                let rel = (c - cn) / c.max(1e-300);
                p = pn;
                r = rn;
                c = cn;
                lambda = (lambda * 0.3).max(1e-12);
                improved = true;
                if rel < 1e-12 {
                    return param.to_matrix(&base, &p);
                }
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    param.to_matrix(&base, &p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(m: &Mat3) -> (Vec<Vec3>, Vec<Vec3>) {
        let mut x = Vec::new();
        for i in 0..40 {
            let f = i as f64;
            x.push(Vec3::new(0.1 + (f * 0.37).fract(), 0.1 + (f * 0.61).fract(), 0.1 + (f * 0.83).fract()));
        }
        let y = x.iter().map(|v| m * v).collect();
        (x, y)
    }

    #[test]
    fn ls_recovers_exact_matrix() {
        let m = Mat3::new(1.1, -0.05, 0.02, 0.03, 0.95, 0.01, -0.02, 0.04, 1.2);
        let (x, y) = data(&m);
        assert!((least_squares(&x, &y, None).unwrap() - m).abs().max() < 1e-12);
    }

    #[test]
    fn constrained_ls_honours_constraint() {
        let m = Mat3::new(1.1, -0.05, 0.02, 0.03, 0.95, 0.01, -0.02, 0.04, 1.2);
        let (x, y) = data(&m);
        let w = Vec3::new(0.95, 1.0, 1.09);
        let fit = least_squares_constrained(&x, &y, None, &w, &w).unwrap();
        assert!((fit * w - w).abs().max() < 1e-12);
    }

    #[test]
    fn refine_converges_and_keeps_constraint() {
        let m = Mat3::new(1.1, -0.05, 0.02, 0.03, 0.95, 0.01, -0.02, 0.04, 1.2);
        let (x, y) = data(&m);
        let res = |mm: &Mat3| -> Vec<f64> {
            x.iter().zip(&y).map(|(xi, yi)| (mm * xi - yi).norm()).collect()
        };
        let got = refine(&Mat3::identity(), &Param::Free, &res);
        assert!((got - m).abs().max() < 1e-6, "{got}");
        let w = Vec3::new(0.95, 1.0, 1.09);
        let t = m * w;
        let start = least_squares_constrained(&x, &y, None, &w, &t).unwrap();
        let got = refine(&start, &Param::Neutral { w }, &res);
        assert!((got * w - t).abs().max() < 1e-12);
        assert!((got - m).abs().max() < 1e-6);
    }
}
