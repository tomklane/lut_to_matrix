//! 3D LUT interpolation.

use serde::{Deserialize, Serialize};

use super::Lut3D;
use crate::math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// Tetrahedral (Kasson et al. 1995; what Resolve uses by default). Preserves the
    /// neutral axis exactly on the grid diagonal.
    #[default]
    Tetrahedral,
    Trilinear,
}

/// Sample `lut` at normalised coordinates `t` ∈ [0,1]³.
pub(super) fn sample(lut: &Lut3D, t: &Vec3, interp: Interpolation) -> Vec3 {
    let n = lut.size;
    let s = (n - 1) as f64;
    let mut i = [0usize; 3];
    let mut f = [0f64; 3];
    for c in 0..3 {
        let x = t[c] * s;
        let k = (x.floor() as usize).min(n - 2);
        i[c] = k;
        f[c] = x - k as f64;
    }
    let (r, g, b) = (i[0], i[1], i[2]);
    let (fr, fg, fb) = (f[0], f[1], f[2]);
    let c = |dr: usize, dg: usize, db: usize| lut.at(r + dr, g + dg, b + db);

    match interp {
        Interpolation::Trilinear => {
            let c00 = c(0, 0, 0) * (1.0 - fr) + c(1, 0, 0) * fr;
            let c10 = c(0, 1, 0) * (1.0 - fr) + c(1, 1, 0) * fr;
            let c01 = c(0, 0, 1) * (1.0 - fr) + c(1, 0, 1) * fr;
            let c11 = c(0, 1, 1) * (1.0 - fr) + c(1, 1, 1) * fr;
            let c0 = c00 * (1.0 - fg) + c10 * fg;
            let c1 = c01 * (1.0 - fg) + c11 * fg;
            c0 * (1.0 - fb) + c1 * fb
        }
        Interpolation::Tetrahedral => {
            let c000 = c(0, 0, 0);
            let c111 = c(1, 1, 1);
            if fr > fg {
                if fg > fb {
                    // r > g > b
                    let c100 = c(1, 0, 0);
                    let c110 = c(1, 1, 0);
                    c000 + (c100 - c000) * fr + (c110 - c100) * fg + (c111 - c110) * fb
                } else if fr > fb {
                    // r > b ≥ g
                    let c100 = c(1, 0, 0);
                    let c101 = c(1, 0, 1);
                    c000 + (c100 - c000) * fr + (c101 - c100) * fb + (c111 - c101) * fg
                } else {
                    // b ≥ r > g
                    let c001 = c(0, 0, 1);
                    let c101 = c(1, 0, 1);
                    c000 + (c001 - c000) * fb + (c101 - c001) * fr + (c111 - c101) * fg
                }
            } else if fb > fg {
                // b > g ≥ r
                let c001 = c(0, 0, 1);
                let c011 = c(0, 1, 1);
                c000 + (c001 - c000) * fb + (c011 - c001) * fg + (c111 - c011) * fr
            } else if fb > fr {
                // g ≥ b > r
                let c010 = c(0, 1, 0);
                let c011 = c(0, 1, 1);
                c000 + (c010 - c000) * fg + (c011 - c010) * fb + (c111 - c011) * fr
            } else {
                // g ≥ r ≥ b
                let c010 = c(0, 1, 0);
                let c110 = c(1, 1, 0);
                c000 + (c010 - c000) * fg + (c110 - c010) * fr + (c111 - c110) * fb
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn affine(v: Vec3) -> Vec3 {
        // Any affine map is reproduced exactly by both interpolators.
        Vec3::new(
            0.2 + 0.5 * v.x + 0.1 * v.y - 0.05 * v.z,
            0.1 * v.x + 0.7 * v.y + 0.2 * v.z,
            -0.1 + 0.3 * v.x + 0.3 * v.z,
        )
    }

    #[test]
    fn affine_is_exact() {
        let lut = Lut3D::from_fn(9, affine);
        for interp in [Interpolation::Tetrahedral, Interpolation::Trilinear] {
            for k in 0..500 {
                let k = k as f64;
                let v = Vec3::new((k * 0.137).fract(), (k * 0.311).fract(), (k * 0.713).fract());
                let d = (lut.apply(&v, interp) - affine(v)).abs().max();
                assert!(d < 1e-12, "{interp:?} {v:?} {d}");
            }
        }
    }

    #[test]
    fn grid_points_exact_and_neutral_axis_tetra() {
        let f = |v: Vec3| Vec3::new(v.x * v.x, v.y.sqrt(), (v.z * 3.0).sin());
        let lut = Lut3D::from_fn(17, f);
        let p = Vec3::new(4.0 / 16.0, 9.0 / 16.0, 1.0);
        assert!((lut.apply(&p, Interpolation::Tetrahedral) - f(p)).abs().max() < 1e-12);
        // On the neutral diagonal tetrahedral only uses diagonal grid points.
        let g = |v: Vec3| Vec3::new(v.x.powf(0.45), v.y.powf(0.45), v.z.powf(0.45));
        let lut = Lut3D::from_fn(5, g);
        let v = Vec3::new(0.3, 0.3, 0.3);
        let o = lut.apply(&v, Interpolation::Tetrahedral);
        assert!((o.x - o.y).abs() < 1e-15 && (o.y - o.z).abs() < 1e-15);
    }
}
