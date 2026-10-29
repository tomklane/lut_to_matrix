//! ColorChecker Classic (24 patch) reference values.

use serde::{Deserialize, Serialize};

use crate::difference::lab_to_xyz;
use crate::math::{whites, Cat, Vec3, Xy};

/// Which published values to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChartEdition {
    /// Charts made from November 2014 (new pigments).
    #[default]
    After2014,
    /// Earlier charts.
    Before2014,
}

impl ChartEdition {
    pub const ALL: [ChartEdition; 2] = [ChartEdition::After2014, ChartEdition::Before2014];

    pub fn label(self) -> &'static str {
        match self {
            ChartEdition::After2014 => "ColorChecker Classic (Nov 2014 and later)",
            ChartEdition::Before2014 => "ColorChecker Classic (before Nov 2014)",
        }
    }

    fn lab(self) -> &'static [[f64; 3]; 24] {
        match self {
            ChartEdition::After2014 => &LAB_AFTER_2014,
            ChartEdition::Before2014 => &LAB_BEFORE_2014,
        }
    }

    /// The 24 patches as CIE XYZ under D65 (Y of a perfect white = 1), from the
    /// published D50 L*a*b* by Bradford adaptation (as Resolve's CST adapts).
    pub fn xyz_d65(self) -> [Vec3; 24] {
        let wn = D50.to_xyz();
        let cat = Cat::Bradford.matrix(D50, whites::D65);
        let lab = self.lab();
        std::array::from_fn(|i| cat * lab_to_xyz(&Vec3::from(lab[i]), &wn))
    }
}

/// CIE D50 (CIE 015:2018, 4-decimal xy), the white of the published L*a*b* values.
pub const D50: Xy = Xy::new(0.3457, 0.3585);

/// Patch names, chart order: row by row from dark skin (top left with the neutral row
/// at the bottom) to black.
pub const PATCH_NAMES: [&str; 24] = [
    "dark skin",
    "light skin",
    "blue sky",
    "foliage",
    "blue flower",
    "bluish green",
    "orange",
    "purplish blue",
    "moderate red",
    "purple",
    "yellow green",
    "orange yellow",
    "blue",
    "green",
    "red",
    "yellow",
    "magenta",
    "cyan",
    "white",
    "neutral 8",
    "neutral 6.5",
    "neutral 5",
    "neutral 3.5",
    "black",
];

/// Chart indices of the neutral row, white to black.
pub const NEUTRALS: [usize; 6] = [18, 19, 20, 21, 22, 23];

/// X-Rite, "ColorChecker Classic" L*a*b* (D50, 2°) for charts made after November
/// 2014. Same values as colour-science 0.4.x `CCS_COLOURCHECKERS["ColorChecker24 - After
/// November 2014"]` (checked by a test against xyY copied from it).
const LAB_AFTER_2014: [[f64; 3]; 24] = [
    [37.54, 14.37, 14.92],
    [64.66, 19.27, 17.50],
    [49.32, -3.82, -22.54],
    [43.46, -12.74, 22.72],
    [54.94, 9.61, -24.79],
    [70.48, -32.26, -0.37],
    [62.73, 35.83, 56.50],
    [39.43, 10.75, -45.17],
    [50.57, 48.64, 16.67],
    [30.10, 22.54, -20.87],
    [71.77, -24.13, 58.19],
    [71.51, 18.24, 67.37],
    [28.37, 15.42, -49.80],
    [54.38, -39.72, 32.27],
    [42.43, 51.05, 28.62],
    [81.80, 2.67, 80.41],
    [50.63, 51.28, -14.12],
    [49.57, -29.71, -28.32],
    [95.19, -1.03, 2.93],
    [81.29, -0.57, 0.44],
    [66.89, -0.75, -0.06],
    [50.76, -0.13, 0.14],
    [35.63, -0.46, -0.48],
    [20.64, 0.07, -0.46],
];

/// X-Rite L*a*b* (D50, 2°) for charts made before November 2014 (colour-science
/// "ColorChecker24 - Before November 2014", converted to L*a*b* and rounded to 2 decimals).
const LAB_BEFORE_2014: [[f64; 3]; 24] = [
    [37.99, 13.56, 14.06],
    [65.71, 18.13, 17.81],
    [49.93, -4.88, -21.90],
    [43.14, -13.09, 21.90],
    [55.11, 8.84, -25.40],
    [70.72, -33.40, -0.20],
    [62.66, 36.07, 57.10],
    [40.02, 10.41, -45.96],
    [51.12, 48.24, 16.25],
    [30.33, 22.98, -21.59],
    [72.53, -23.71, 57.26],
    [71.94, 19.36, 67.86],
    [28.78, 14.18, -50.30],
    [55.26, -38.34, 31.37],
    [42.10, 53.38, 28.19],
    [81.73, 4.04, 79.82],
    [51.94, 49.99, -14.57],
    [51.04, -28.63, -28.64],
    [96.54, -0.43, 1.19],
    [81.26, -0.64, -0.33],
    [66.77, -0.73, -0.50],
    [50.87, -0.15, -0.27],
    [35.66, -0.42, -1.23],
    [20.46, -0.08, -0.97],
];

#[cfg(test)]
mod tests {
    use super::*;

    /// colour-science xyY (D50) for a few patches of the 2014 chart.
    #[test]
    fn matches_colour_science() {
        let want = [
            (0, [0.43748, 0.37852, 0.09832]),
            (5, [0.2864, 0.38972, 0.41436]),
            (14, [0.56287, 0.33556, 0.1278]),
            (18, [0.34891, 0.36421, 0.88069]),
            (23, [0.34377, 0.35606, 0.03151]),
        ];
        let wn = D50.to_xyz();
        for (i, xyy) in want {
            let xyz = lab_to_xyz(&Vec3::from(LAB_AFTER_2014[i]), &wn);
            let s = xyz.sum();
            assert!((xyz.x / s - xyy[0]).abs() < 2e-4 && (xyz.y / s - xyy[1]).abs() < 2e-4, "{i}");
            assert!((xyz.y - xyy[2]).abs() < 2e-4, "{i}: Y {}", xyz.y);
        }
    }

    #[test]
    fn neutrals_are_near_d65_after_adaptation() {
        let x = ChartEdition::After2014.xyz_d65();
        for i in NEUTRALS {
            let s = x[i].sum();
            assert!((x[i].x / s - 0.3127).abs() < 0.008 && (x[i].y / s - 0.329).abs() < 0.008, "{i}: {:?}", x[i] / s);
        }
        assert!((x[21].y - 0.1906).abs() < 5e-4, "neutral 5 is ~19% reflectance");
    }
}
