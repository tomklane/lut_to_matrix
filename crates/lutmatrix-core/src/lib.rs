//! LUT Matrix core library.
//!
//! All colour math, LUT parsing, matrix fitting and DCTL generation live here.
//! This crate has no UI dependencies; the CLI and the Tauri app are thin
//! front ends over it.

pub mod analysis;
pub mod batch;
pub mod dctl;
pub mod detect;
pub mod difference;
pub mod film;
pub mod fit;
pub mod gray_axis;
pub mod lut;
pub mod math;
pub mod opendrt_match;
pub mod preset_store;
pub mod sample;
pub mod spaces;

/// The app version, defined once in the workspace `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
