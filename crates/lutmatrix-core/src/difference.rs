//! Perceptual colour difference: CIELAB / CIEDE2000 (SDR) and ICtCp / ΔE ITP (HDR).

use serde::{Deserialize, Serialize};

use crate::math::{Mat3, Vec3};
use crate::spaces::display::pq_inverse_eotf;
use crate::spaces::gamuts::REC2020;

/// CIE 1976 L*a*b* from XYZ, relative to white `wn` (same units as `xyz`).
/// CIE 015:2018 §8.2.1.
pub fn xyz_to_lab(xyz: &Vec3, wn: &Vec3) -> Vec3 {
    const E: f64 = 216.0 / 24389.0;
    const K: f64 = 24389.0 / 27.0;
    let f = |t: f64| if t > E { t.cbrt() } else { (K * t + 16.0) / 116.0 };
    let fx = f(xyz.x / wn.x);
    let fy = f(xyz.y / wn.y);
    let fz = f(xyz.z / wn.z);
    Vec3::new(116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

/// Inverse of [`xyz_to_lab`].
pub fn lab_to_xyz(lab: &Vec3, wn: &Vec3) -> Vec3 {
    const E: f64 = 216.0 / 24389.0;
    const K: f64 = 24389.0 / 27.0;
    let fy = (lab.x + 16.0) / 116.0;
    let fx = fy + lab.y / 500.0;
    let fz = fy - lab.z / 200.0;
    let inv = |f: f64| {
        let f3 = f * f * f;
        if f3 > E {
            f3
        } else {
            (116.0 * f - 16.0) / K
        }
    };
    let yr = if lab.x > K * E { fy * fy * fy } else { lab.x / K };
    Vec3::new(inv(fx) * wn.x, yr * wn.y, inv(fz) * wn.z)
}

/// CIEDE2000 (CIE 142-2001), following Sharma, Wu & Dalal (2005) including their
/// notes on hue-angle handling; kL = kC = kH = 1.
pub fn delta_e_2000(lab1: &Vec3, lab2: &Vec3) -> f64 {
    let (l1, a1, b1) = (lab1.x, lab1.y, lab1.z);
    let (l2, a2, b2) = (lab2.x, lab2.y, lab2.z);
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cbar = (c1 + c2) / 2.0;
    let cbar7 = cbar.powi(7);
    let g = 0.5 * (1.0 - (cbar7 / (cbar7 + 25f64.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = a1p.hypot(b1);
    let c2p = a2p.hypot(b2);
    let hp = |b: f64, a: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            let h = b.atan2(a).to_degrees();
            if h < 0.0 {
                h + 360.0
            } else {
                h
            }
        }
    };
    let h1p = hp(b1, a1p);
    let h2p = hp(b2, a2p);

    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let d_hp = 2.0 * (c1p * c2p).sqrt() * (dhp.to_radians() / 2.0).sin();

    let lbarp = (l1 + l2) / 2.0;
    let cbarp = (c1p + c2p) / 2.0;
    let hbarp = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hbarp - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hbarp).to_radians().cos()
        + 0.32 * (3.0 * hbarp + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hbarp - 63.0).to_radians().cos();
    let d_theta = 30.0 * (-((hbarp - 275.0) / 25.0).powi(2)).exp();
    let cbarp7 = cbarp.powi(7);
    let rc = 2.0 * (cbarp7 / (cbarp7 + 25f64.powi(7))).sqrt();
    let l50 = (lbarp - 50.0).powi(2);
    let sl = 1.0 + 0.015 * l50 / (20.0 + l50).sqrt();
    let sc = 1.0 + 0.045 * cbarp;
    let sh = 1.0 + 0.015 * cbarp * t;
    let rt = -(2.0 * d_theta).to_radians().sin() * rc;
    let (x, y, z) = (dlp / sl, dcp / sc, d_hp / sh);
    (x * x + y * y + z * z + rt * y * z).sqrt()
}

// ITU-R BT.2100-2 Table 7 (ICtCp, PQ).
fn lms_from_rec2020() -> Mat3 {
    Mat3::new(1688.0, 2146.0, 262.0, 683.0, 2951.0, 462.0, 99.0, 309.0, 3688.0) / 4096.0
}
fn ictcp_from_lms_pq() -> Mat3 {
    Mat3::new(2048.0, 2048.0, 0.0, 6610.0, -13613.0, 7003.0, 17933.0, -17390.0, -543.0) / 4096.0
}

/// ICtCp (PQ) from absolute CIE XYZ in cd/m², via linear BT.2020 RGB.
/// Negative RGB (outside BT.2020) is clamped to 0 before PQ.
pub fn xyz_nits_to_ictcp(xyz: &Vec3) -> Vec3 {
    let rgb = REC2020.xyz_to_rgb() * xyz;
    let lms = lms_from_rec2020() * rgb;
    let lms_pq = lms.map(|v| pq_inverse_eotf(v.max(0.0)));
    ictcp_from_lms_pq() * lms_pq
}

/// ΔE ITP, ITU-R BT.2124-0 (2019) eq. 1: 720·√(ΔI² + ΔT² + ΔP²), T = 0.5·Ct, P = Cp.
/// One unit ≈ one just-noticeable difference.
pub fn delta_e_itp(ictcp1: &Vec3, ictcp2: &Vec3) -> f64 {
    let di = ictcp1.x - ictcp2.x;
    let dt = 0.5 * (ictcp1.y - ictcp2.y);
    let dp = ictcp1.z - ictcp2.z;
    720.0 * (di * di + dt * dt + dp * dp).sqrt()
}

/// Which metric scores the fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// ΔE2000 for gamma (SDR) outputs, ΔE ITP for PQ outputs.
    #[default]
    Auto,
    DeltaE2000,
    DeltaEItp,
}

impl Metric {
    pub fn resolve(self, hdr: bool) -> Metric {
        match self {
            Metric::Auto if hdr => Metric::DeltaEItp,
            Metric::Auto => Metric::DeltaE2000,
            m => m,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Metric::Auto => "auto",
            Metric::DeltaE2000 => "ΔE2000",
            Metric::DeltaEItp => "ΔE ITP",
        }
    }
}

/// A resolved metric bound to its reference white, comparing absolute XYZ (cd/m²).
#[derive(Debug, Clone, Copy)]
pub struct Scorer {
    pub metric: Metric,
    /// Lab reference white (cd/m²), used by ΔE2000.
    pub lab_white: Vec3,
}

impl Scorer {
    pub fn delta(&self, a: &Vec3, b: &Vec3) -> f64 {
        match self.metric {
            Metric::DeltaEItp => delta_e_itp(&xyz_nits_to_ictcp(a), &xyz_nits_to_ictcp(b)),
            _ => delta_e_2000(&xyz_to_lab(a, &self.lab_white), &xyz_to_lab(b, &self.lab_white)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sharma, Wu & Dalal (2005) Table 1, selected pairs (with expected ΔE00).
    #[test]
    fn sharma_test_data() {
        let pairs = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 3.1571, -77.2803], [50.0, 0.0, -82.7485], 2.8615),
            ([50.0, -1.3802, -84.2814], [50.0, 0.0, -82.7485], 1.0000),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.49, -0.001], [50.0, -2.49, 0.0011], 7.2195),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
            ([22.7233, 20.0904, -46.6940], [23.0331, 14.9730, -42.5619], 2.0373),
            ([2.0776, 0.0795, -1.1350], [0.9033, -0.0636, -0.5514], 0.9082),
        ];
        for (a, b, want) in pairs {
            let d = delta_e_2000(&Vec3::from(a), &Vec3::from(b));
            assert!((d - want).abs() < 5e-5, "{a:?} {b:?}: {d} vs {want}");
            let d2 = delta_e_2000(&Vec3::from(b), &Vec3::from(a));
            assert!((d - d2).abs() < 1e-9, "symmetry");
        }
    }

    #[test]
    fn lab_round_trip() {
        let wn = crate::math::whites::D65.to_xyz() * 100.0;
        for xyz in [[20.0, 18.0, 30.0], [0.1, 0.12, 0.05], [95.0, 100.0, 108.0]] {
            let v = Vec3::from(xyz);
            assert!((lab_to_xyz(&xyz_to_lab(&v, &wn), &wn) - v).abs().max() < 1e-10);
        }
    }
}
