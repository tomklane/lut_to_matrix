//! Colour space registry: camera log inputs, display outputs, DaVinci working space.

pub mod camera;
pub mod curves;
pub mod display;
pub mod gamuts;

pub use camera::{camera_space, CameraSpace, CameraSpaceId, InputSpace, CAMERA_SPACES};
pub use curves::LogCurve;
pub use display::{DisplayGamut, DisplayTransfer, OutputSpace};

/// DaVinci Wide Gamut / DaVinci Intermediate, the DCTL's working space.
pub mod davinci {
    use super::curves;
    use super::gamuts::DWG;
    use crate::math::Mat3;

    /// Linear DWG → CIE XYZ (D65), derived in f64 from the DWG primaries.
    pub fn dwg_to_xyz() -> Mat3 {
        DWG.rgb_to_xyz()
    }

    /// CIE XYZ (D65) → linear DWG.
    pub fn xyz_to_dwg() -> Mat3 {
        DWG.xyz_to_rgb()
    }

    pub use curves::{di_decode, di_encode};
}
