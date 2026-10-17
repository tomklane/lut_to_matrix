//! LUT model, file I/O and interpolation.
//!
//! A [`Lut`] is an optional 1D shaper followed by an optional 3D table, which covers
//! plain 1D `.cube`, plain 3D `.cube`, Resolve-style shaper+3D `.cube`, and `.3dl`.

mod cube;
mod interp;
mod threedl;

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::math::Vec3;

pub use cube::{parse_cube, write_cube};
pub use interp::Interpolation;
pub use threedl::parse_3dl;

#[derive(Debug, Error)]
pub enum LutError {
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("line {line}: {msg}")]
    Parse { line: usize, msg: String },
    #[error("{0}")]
    Invalid(String),
    #[error("unsupported LUT format '{0}' (expected .cube or .3dl)")]
    UnsupportedFormat(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LutFormat {
    Cube,
    ThreeDl,
}

/// Per-channel 1D table. Entry `i` is the output for input
/// `min + (max - min) * i / (size - 1)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut1D {
    pub data: Vec<[f64; 3]>,
    pub domain_min: [f64; 3],
    pub domain_max: [f64; 3],
}

/// 3D table, red index fastest (`.cube` order): `data[r + g*N + b*N*N]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut3D {
    pub size: usize,
    pub data: Vec<[f64; 3]>,
    pub domain_min: [f64; 3],
    pub domain_max: [f64; 3],
}

impl Lut1D {
    pub fn size(&self) -> usize {
        self.data.len()
    }

    pub fn apply(&self, rgb: &Vec3) -> Vec3 {
        let n = self.data.len();
        let mut out = Vec3::zeros();
        for c in 0..3 {
            let t = normalise(rgb[c], self.domain_min[c], self.domain_max[c]);
            let x = t * (n - 1) as f64;
            let i = (x.floor() as usize).min(n - 2);
            let f = x - i as f64;
            out[c] = self.data[i][c] * (1.0 - f) + self.data[i + 1][c] * f;
        }
        out
    }
}

impl Lut3D {
    pub fn at(&self, r: usize, g: usize, b: usize) -> Vec3 {
        let n = self.size;
        Vec3::from(self.data[r + g * n + b * n * n])
    }

    /// Build a 3D LUT by sampling `f` on a uniform `size`³ grid over [0,1]³.
    pub fn from_fn(size: usize, f: impl Fn(Vec3) -> Vec3) -> Self {
        let mut data = Vec::with_capacity(size * size * size);
        let s = (size - 1) as f64;
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    let v = f(Vec3::new(r as f64 / s, g as f64 / s, b as f64 / s));
                    data.push([v.x, v.y, v.z]);
                }
            }
        }
        Self { size, data, domain_min: [0.0; 3], domain_max: [1.0; 3] }
    }

    pub fn apply(&self, rgb: &Vec3, interp: Interpolation) -> Vec3 {
        let t = Vec3::new(
            normalise(rgb.x, self.domain_min[0], self.domain_max[0]),
            normalise(rgb.y, self.domain_min[1], self.domain_max[1]),
            normalise(rgb.z, self.domain_min[2], self.domain_max[2]),
        );
        interp::sample(self, &t, interp)
    }
}

/// Map `v` from [min, max] to [0, 1], clamped (LUT inputs outside the domain are
/// clamped, as Resolve does).
fn normalise(v: f64, min: f64, max: f64) -> f64 {
    ((v - min) / (max - min)).clamp(0.0, 1.0)
}

/// Anything that maps LUT-input code values to output code values. Implemented by
/// [`Lut`]; tests also use exact analytic transforms.
pub trait Transform {
    fn apply_rgb(&self, rgb: &Vec3, interp: Interpolation) -> Vec3;
    /// True if `rgb` is inside the input domain.
    fn in_domain_rgb(&self, _rgb: &Vec3) -> bool {
        true
    }
}

impl Transform for Lut {
    fn apply_rgb(&self, rgb: &Vec3, interp: Interpolation) -> Vec3 {
        self.apply(rgb, interp)
    }
    fn in_domain_rgb(&self, rgb: &Vec3) -> bool {
        self.in_domain(rgb)
    }
}

/// An exact transform from a closure (used by tests and synthetic checks).
pub struct FnTransform<F: Fn(&Vec3) -> Vec3>(pub F);

impl<F: Fn(&Vec3) -> Vec3> Transform for FnTransform<F> {
    fn apply_rgb(&self, rgb: &Vec3, _interp: Interpolation) -> Vec3 {
        (self.0)(rgb)
    }
}

/// A LUT file: optional shaper, optional 3D table, plus metadata used for detection.
#[derive(Debug, Clone)]
pub struct Lut {
    pub format: LutFormat,
    /// File name (no directory), if loaded from disk.
    pub filename: Option<String>,
    pub title: Option<String>,
    pub comments: Vec<String>,
    pub shaper: Option<Lut1D>,
    pub cube: Option<Lut3D>,
    /// Non-fatal oddities found while parsing.
    pub warnings: Vec<String>,
}

impl Lut {
    /// Load a `.cube` or `.3dl` file. Works with Windows and macOS paths (including
    /// non-UTF-8 bytes in the file, which are decoded lossily).
    pub fn load(path: &Path) -> Result<Self, LutError> {
        let bytes = std::fs::read(path).map_err(|source| LutError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let text = String::from_utf8_lossy(&bytes);
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let mut lut = match ext.as_str() {
            "cube" => parse_cube(&text)?,
            "3dl" => parse_3dl(&text)?,
            other => {
                // Sniff: .cube files declare LUT_*_SIZE.
                if text.contains("LUT_3D_SIZE") || text.contains("LUT_1D_SIZE") {
                    parse_cube(&text)?
                } else {
                    return Err(LutError::UnsupportedFormat(other.to_string()));
                }
            }
        };
        lut.filename = path.file_name().map(|n| n.to_string_lossy().into_owned());
        Ok(lut)
    }

    pub fn is_3d(&self) -> bool {
        self.cube.is_some()
    }

    /// Input domain (min, max) per channel of the first stage.
    pub fn input_domain(&self) -> ([f64; 3], [f64; 3]) {
        if let Some(s) = &self.shaper {
            (s.domain_min, s.domain_max)
        } else if let Some(c) = &self.cube {
            (c.domain_min, c.domain_max)
        } else {
            ([0.0; 3], [1.0; 3])
        }
    }

    /// True if `rgb` lies inside the LUT's input domain (no clamping would occur).
    pub fn in_domain(&self, rgb: &Vec3) -> bool {
        let (lo, hi) = self.input_domain();
        (0..3).all(|c| rgb[c] >= lo[c] && rgb[c] <= hi[c])
    }

    pub fn apply(&self, rgb: &Vec3, interp: Interpolation) -> Vec3 {
        let mut v = *rgb;
        if let Some(s) = &self.shaper {
            v = s.apply(&v);
        }
        if let Some(c) = &self.cube {
            v = c.apply(&v, interp);
        }
        v
    }

    /// Smallest and largest output value over every table entry (cheap proxy for
    /// output range, used by detection and clip checks).
    pub fn output_extent(&self) -> (f64, f64) {
        let data = match (&self.cube, &self.shaper) {
            (Some(c), _) => &c.data,
            (None, Some(s)) => &s.data,
            _ => return (0.0, 1.0),
        };
        data.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
            (lo.min(v), hi.max(v))
        })
    }

    /// Short human description, e.g. "3D 33³ .cube".
    pub fn describe(&self) -> String {
        let fmt = match self.format {
            LutFormat::Cube => ".cube",
            LutFormat::ThreeDl => ".3dl",
        };
        match (&self.shaper, &self.cube) {
            (Some(s), Some(c)) => format!("{}-point shaper + 3D {}³ {fmt}", s.size(), c.size),
            (None, Some(c)) => format!("3D {}³ {fmt}", c.size),
            (Some(s), None) => format!("1D {}-point {fmt}", s.size()),
            (None, None) => format!("empty {fmt}"),
        }
    }
}
