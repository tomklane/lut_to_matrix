//! LUT Matrix core library.
//!
//! All colour math, LUT parsing, matrix fitting and DCTL generation live here.
//! This crate has no UI dependencies; the CLI and the Tauri app are thin
//! front ends over it.

pub mod math;
pub mod lut;
pub mod spaces;

/// The app version, defined once in the workspace `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
