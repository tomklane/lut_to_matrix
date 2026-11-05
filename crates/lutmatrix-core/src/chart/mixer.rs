//! The RGB Mixer model: what DaVinci Resolve's RGB Mixer does in a node (DECISIONS.md D46).
//!
//! Resolve's RGB Mixer multiplies the node's values by a 3 × 3 matrix: each output
//! channel (Red / Green / Blue Output) is a sum of the input R, G and B with its three
//! sliders, defaults 1 on the diagonal and 0 elsewhere. With *Preserve Luminance* on,
//! moving one slider moves the other two of that output the opposite way by the same
//! total, so each output's sliders keep summing to 1 (equal R, G, B stay equal: neutrals
//! and clipped white stay neutral). It works on whatever the node holds: in a DaVinci Wide
//! Gamut / Intermediate timeline that is DI log values, unless the node's colour space is
//! set to linear.
//!
//! Source: Blackmagic Design, DaVinci Resolve 18.6 Reference Manual, "RGB Mixer"
//! (Color page palettes): output-channel sliders default to 1.00, the others to 0;
//! Preserve Luminance raises/lowers the other two channels by the same amount.

use serde::{Deserialize, Serialize};

use crate::math::{Mat3, Vec3};
use crate::spaces::curves::{di_decode, di_encode};
use crate::spaces::davinci;

/// What the mixer node works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MixerSpace {
    /// DaVinci Intermediate log values (a node in a DWG/DI timeline, as it comes).
    #[default]
    Di,
    /// Linear DWG (the node's colour space set to linear).
    Linear,
}

/// A fitted RGB Mixer: `rows[out][in]`, as Resolve's Red / Green / Blue Output sliders.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mixer {
    pub rows: [[f64; 3]; 3],
    pub space: MixerSpace,
    pub preserve_luminance: bool,
}

impl Mixer {
    pub fn matrix(&self) -> Mat3 {
        Mat3::from_fn(|r, c| self.rows[r][c])
    }

    /// The mixer applied to a colour, CIE XYZ (D65) in and out.
    pub fn apply_xyz(&self, xyz: &Vec3) -> Vec3 {
        let d = davinci::xyz_to_dwg() * xyz;
        let o = match self.space {
            MixerSpace::Di => (self.matrix() * d.map(di_encode)).map(di_decode),
            MixerSpace::Linear => self.matrix() * d,
        };
        davinci::dwg_to_xyz() * o
    }

    /// From fit parameters: 9 values, or with Preserve Luminance the 6 off-diagonal
    /// ones (each diagonal is 1 minus the other two of its row).
    pub fn from_params(q: &[f64], space: MixerSpace, preserve_luminance: bool) -> Mixer {
        let rows = if preserve_luminance {
            [[1.0 - q[0] - q[1], q[0], q[1]], [q[2], 1.0 - q[2] - q[3], q[3]], [q[4], q[5], 1.0 - q[4] - q[5]]]
        } else {
            [[q[0], q[1], q[2]], [q[3], q[4], q[5]], [q[6], q[7], q[8]]]
        };
        Mixer { rows, space, preserve_luminance }
    }

    pub fn params(&self) -> Vec<f64> {
        let r = &self.rows;
        if self.preserve_luminance {
            vec![r[0][1], r[0][2], r[1][0], r[1][2], r[2][0], r[2][1]]
        } else {
            r.iter().flatten().copied().collect()
        }
    }

    /// The same mixer with each output's sliders made to sum to 1 (keeping their
    /// off-diagonal values).
    pub fn with_preserve(m: Mat3, space: MixerSpace, preserve_luminance: bool) -> Mixer {
        let rows: [[f64; 3]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| m[(r, c)]));
        let mut x = Mixer { rows, space, preserve_luminance };
        if preserve_luminance {
            for r in 0..3 {
                let off: f64 = (0..3).filter(|&c| c != r).map(|c| x.rows[r][c]).sum();
                x.rows[r][r] = 1.0 - off;
            }
        }
        x
    }

    /// Resolve's panel as text: one line per output.
    pub fn text(&self) -> String {
        ["Red", "Green", "Blue"]
            .iter()
            .zip(&self.rows)
            .map(|(n, r)| format!("{n} Output: R {:.2}  G {:.2}  B {:.2}", r[0], r[1], r[2]))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Weights of the level Saturation keeps fixed in a linear node, as measured (below).
pub const RESOLVE_SAT_WEIGHTS: [f64; 3] = [0.235, 0.573, 0.192];

/// Saturation's working values from linear (sign kept): a square root. Measured (D47).
pub fn sat_encode(v: f64) -> f64 {
    v.signum() * v.abs().sqrt()
}

/// The user's two-node chain in Resolve (DECISIONS.md D47):
/// 1. a node with its colour space gamma set to linear: Gain R, G, B (a multiply per
///    channel), then Saturation `s`. Measured from Resolve stills (Saturation 50 and 100,
///    same Gain): Resolve does it on the square root of the linear values, around a
///    weighted level `Y = w · √g`, `out = max(Y + s·(√g − Y), 0)²`; the slider is 50·s
///    (0 … 100, 50 = unity, 100 = double);
/// 2. a node in DWG/DI with the RGB Mixer ([`Mixer`], normally on DI values).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NodeChain {
    pub gains: [f64; 3],
    pub saturation: f64,
    /// The weights of the level Saturation keeps fixed ([`RESOLVE_SAT_WEIGHTS`]).
    pub luma: [f64; 3],
    pub mixer: Mixer,
}

impl NodeChain {
    /// Node 1 on linear DWG.
    pub fn node1(&self, d: &Vec3) -> Vec3 {
        let g = Vec3::new(d.x * self.gains[0], d.y * self.gains[1], d.z * self.gains[2]).map(sat_encode);
        let y = self.luma[0] * g.x + self.luma[1] * g.y + self.luma[2] * g.z;
        (Vec3::repeat(y) + (g - Vec3::repeat(y)) * self.saturation).map(|v| v.max(0.0).powi(2))
    }

    /// Both nodes, CIE XYZ (D65) in and out.
    pub fn apply_xyz(&self, xyz: &Vec3) -> Vec3 {
        self.mixer.apply_xyz(&(davinci::dwg_to_xyz() * self.node1(&(davinci::xyz_to_dwg() * xyz))))
    }

    /// Resolve's Saturation slider for this chain.
    pub fn saturation_slider(&self) -> f64 {
        50.0 * self.saturation
    }

    /// The values to type into Resolve, as text.
    pub fn text(&self) -> String {
        format!(
            "Node 1 (gamma linear): Gain R {:.3}  G {:.3}  B {:.3}, Saturation {:.1}\nNode 2 (RGB Mixer, {}{}):\n{}",
            self.gains[0],
            self.gains[1],
            self.gains[2],
            self.saturation_slider(),
            match self.mixer.space {
                MixerSpace::Di => "DI",
                MixerSpace::Linear => "linear",
            },
            if self.mixer.preserve_luminance { ", Preserve Luminance" } else { "" },
            self.mixer.text()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserve_luminance_keeps_equal_rgb_and_round_trips() {
        let q = [0.3, -0.1, 0.05, 0.2, -0.4, 0.1];
        let m = Mixer::from_params(&q, MixerSpace::Di, true);
        for r in &m.rows {
            assert!((r.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        }
        assert_eq!(m.params(), q.to_vec());
        // Equal R, G, B (any level) stay where they are.
        for l in [0.01, 0.18, 5.0] {
            let x = davinci::dwg_to_xyz() * Vec3::repeat(l);
            assert!((m.apply_xyz(&x) - x).norm() < 1e-9 * l.max(1.0));
        }
        let free = Mixer::from_params(&[1.0, 0.2, 0.0, 0.0, 0.9, 0.1, 0.0, 0.0, 1.1], MixerSpace::Linear, false);
        assert_eq!(free.params().len(), 9);
        assert!(free.text().starts_with("Red Output: R 1.00  G 0.20  B 0.00"));
    }

    #[test]
    fn node_chain_saturation_on_square_roots() {
        assert!((RESOLVE_SAT_WEIGHTS.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        let id = Mixer::from_params(&[0.0; 6], MixerSpace::Di, true);
        let c = NodeChain { gains: [1.0; 3], saturation: 1.6, luma: RESOLVE_SAT_WEIGHTS, mixer: id };
        let d = Vec3::new(0.3, 0.12, 0.05);
        let o = c.node1(&d);
        // The weighted level of the square roots is kept; their spread grows by s.
        let w = |v: &Vec3| {
            let r = v.map(sat_encode);
            RESOLVE_SAT_WEIGHTS[0] * r.x + RESOLVE_SAT_WEIGHTS[1] * r.y + RESOLVE_SAT_WEIGHTS[2] * r.z
        };
        assert!((w(&o) - w(&d)).abs() < 1e-12);
        let (rd, ro) = (d.map(sat_encode), o.map(sat_encode));
        assert!(((ro.x - ro.y) - 1.6 * (rd.x - rd.y)).abs() < 1e-12);
        // Grays stay; unity is a no-op; nothing goes below 0.
        let g = Vec3::repeat(0.4);
        assert!((c.node1(&g) - g).norm() < 1e-12);
        let one = NodeChain { saturation: 1.0, ..c };
        assert!((one.node1(&d) - d).norm() < 1e-12);
        let two = NodeChain { saturation: 2.0, ..c };
        assert!(two.node1(&Vec3::new(0.0, 0.0, 0.5)).iter().all(|v| *v >= 0.0));
        assert_eq!(c.saturation_slider(), 80.0);
    }
}
