//! A Rust port of **OpenDRT v1.1.0** by Jed Smith
//! (<https://github.com/jedypod/open-display-transform>), GPLv3.
//!
//! The port follows the bundled DCTL (`opendrt/OpenDRT_v1.1.0.1.dctl`, the version used in
//! Resolve for this project) operation for operation in `f32`, so it reproduces Resolve's
//! output to float rounding; `tests/parity.rs` compiles the original DCTL and checks every
//! look, tonescale, creative white, display encoding, input and stickshift setting.
//!
//! - [`Config`]: everything the transform depends on; [`Config::from_presets`] resolves
//!   the DCTL's preset menus.
//! - [`OpenDrt`]: the transform for one [`Config`], with the per-frame constants
//!   precomputed.
//! - [`writer::lut_match_dctl`]: the DCTL with extra "LUT Match" look presets.
//!
//! Not ported: the tonescale overlay (`crv_enable`), which only draws a curve on screen.

mod params;
pub mod tables;
mod transform;
pub mod writer;

pub use params::{
    Config, DisplayGamut, Eotf, InputGamut, InputOetf, Kind, LookParams, ParamInfo, Presets, ToneParams,
};
pub use transform::OpenDrt;

/// A 3×3 matrix, row-major, as the DCTL's `float3x3`.
pub type M3 = [[f32; 3]; 3];

/// The bundled OpenDRT DCTL source, unmodified.
pub const DCTL_SOURCE: &str = include_str!("../opendrt/OpenDRT_v1.1.0.1.dctl");

/// File name of [`DCTL_SOURCE`].
pub const DCTL_FILE_NAME: &str = "OpenDRT_v1.1.0.1.dctl";
