//! Basic f64 linear algebra and colorimetry helpers.
//!
//! Everything here is double precision. Matrices are row-major in the usual
//! colour-science sense: `XYZ = M * RGB` with RGB as a column vector.

use nalgebra::{Matrix3, Vector3};
use serde::{Deserialize, Serialize};

pub type Mat3 = Matrix3<f64>;
pub type Vec3 = Vector3<f64>;

/// CIE 1931 xy chromaticity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Xy {
    pub x: f64,
    pub y: f64,
}

impl Xy {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// XYZ tristimulus with Y = 1.
    pub fn to_xyz(self) -> Vec3 {
        Vec3::new(self.x / self.y, 1.0, (1.0 - self.x - self.y) / self.y)
    }

    pub fn from_xyz(xyz: &Vec3) -> Self {
        let s = xyz.x + xyz.y + xyz.z;
        Self::new(xyz.x / s, xyz.y / s)
    }
}

/// Named white points.
pub mod whites {
    use super::Xy;
    /// CIE D65 as used by ITU-R BT.709 / BT.2020, SMPTE RP 431-2 (P3-D65) and every
    /// camera gamut in this crate (4-decimal form, x = 0.3127, y = 0.3290).
    pub const D65: Xy = Xy::new(0.3127, 0.3290);
    /// DCI white, SMPTE RP 431-2:2011 Table A.1 (x = 0.314, y = 0.351).
    pub const DCI: Xy = Xy::new(0.314, 0.351);
    /// ACES white, SMPTE ST 2065-1:2012 §4.1 / AMPAS TB-2014-004 (x = 0.32168, y = 0.33767).
    pub const ACES: Xy = Xy::new(0.32168, 0.33767);
    /// Blackmagic Wide Gamut white, "Blackmagic Generation 5 Color Science" (2021):
    /// x = 0.3127170, y = 0.3290312 (Blackmagic's own D65 approximation).
    pub const BMD_WG: Xy = Xy::new(0.312717, 0.3290312);
}

/// RGB primaries + white point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Primaries {
    pub r: Xy,
    pub g: Xy,
    pub b: Xy,
    pub white: Xy,
}

impl Primaries {
    /// Normalised primary matrix (RGB → XYZ, white RGB (1,1,1) → white XYZ with Y = 1),
    /// per SMPTE RP 177-1993 §3.3.
    pub fn rgb_to_xyz(&self) -> Mat3 {
        let p = Mat3::from_columns(&[self.r.to_xyz(), self.g.to_xyz(), self.b.to_xyz()]);
        let w = self.white.to_xyz();
        let s = p
            .try_inverse()
            .expect("degenerate primaries")
            * w;
        p * Mat3::from_diagonal(&s)
    }

    pub fn xyz_to_rgb(&self) -> Mat3 {
        self.rgb_to_xyz().try_inverse().expect("degenerate primaries")
    }
}

/// Chromatic adaptation transforms (cone-response matrices).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Cat {
    /// Bradford (Lam 1985), as used by ICC v4 and Resolve's CST. Matrix from
    /// Lindbloom / colour-science `CAT_BRADFORD`.
    #[default]
    Bradford,
    /// CIECAM02 CAT02 (CIE 159:2004).
    Cat02,
    /// No adaptation (XYZ scaling is *not* applied either): identity.
    None,
}

impl Cat {
    fn cone_matrix(self) -> Option<Mat3> {
        match self {
            Cat::Bradford => Some(Mat3::new(
                0.8951, 0.2664, -0.1614, //
                -0.7502, 1.7135, 0.0367, //
                0.0389, -0.0685, 1.0296,
            )),
            Cat::Cat02 => Some(Mat3::new(
                0.7328, 0.4296, -0.1624, //
                -0.7036, 1.6975, 0.0061, //
                0.0030, 0.0136, 0.9834,
            )),
            Cat::None => None,
        }
    }

    /// von Kries-style adaptation matrix mapping XYZ under `src` white to XYZ under
    /// `dst` white (full adaptation, D = 1).
    pub fn matrix(self, src: Xy, dst: Xy) -> Mat3 {
        self.matrix_xyz(&src.to_xyz(), &dst.to_xyz())
    }

    /// As [`Cat::matrix`] but with arbitrary (unnormalised) XYZ whites, which lets a
    /// measured neutral be adapted with its luminance preserved.
    pub fn matrix_xyz(self, src: &Vec3, dst: &Vec3) -> Mat3 {
        let Some(m) = self.cone_matrix() else {
            return Mat3::identity();
        };
        let mi = m.try_inverse().expect("cone matrix invertible");
        let s = m * src;
        let d = m * dst;
        let g = Mat3::from_diagonal(&Vec3::new(d.x / s.x, d.y / s.y, d.z / s.z));
        mi * g * m
    }
}

/// Max absolute element difference between two matrices.
pub fn max_abs_diff(a: &Mat3, b: &Mat3) -> f64 {
    (a - b).abs().max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_maps_white() {
        let p = Primaries {
            r: Xy::new(0.64, 0.33),
            g: Xy::new(0.30, 0.60),
            b: Xy::new(0.15, 0.06),
            white: whites::D65,
        };
        let w = p.rgb_to_xyz() * Vec3::new(1.0, 1.0, 1.0);
        assert!((w - whites::D65.to_xyz()).abs().max() < 1e-14);
    }

    #[test]
    fn cat_maps_white_to_white() {
        for cat in [Cat::Bradford, Cat::Cat02] {
            let m = cat.matrix(whites::ACES, whites::D65);
            let w = m * whites::ACES.to_xyz();
            assert!((w - whites::D65.to_xyz()).abs().max() < 1e-12);
        }
    }
}
