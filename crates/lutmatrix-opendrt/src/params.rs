//! OpenDRT settings: the per-look parameters, the global sliders, and the preset menus
//! resolved exactly as the DCTL's preset mode does.

use serde::{Deserialize, Serialize};

pub use crate::tables::{LookParams, ToneParams};
use crate::tables::{LOOK_PARAMS, LOOK_PRESETS, TONESCALE_PRESETS};

/// How a look parameter is edited in the DCTL's stickshift UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Slider.
    Float,
    /// Check box (0 or 1).
    Toggle,
    /// Combo-box index.
    Choice,
}

/// Name, kind and slider range of one look parameter.
#[derive(Debug, Clone, Copy)]
pub struct ParamInfo {
    pub name: &'static str,
    pub kind: Kind,
    pub min: f32,
    pub max: f32,
}

impl LookParams {
    /// Metadata for every field, in [`LookParams::to_array`] order.
    pub fn info() -> &'static [ParamInfo] {
        &LOOK_PARAMS
    }

    /// Position of the field called `name` (the DCTL variable name without `_`).
    pub fn index_of(name: &str) -> Option<usize> {
        LOOK_PARAMS.iter().position(|p| p.name == name)
    }

    pub fn get(&self, name: &str) -> Option<f32> {
        Some(self.to_array()[Self::index_of(name)?])
    }

    /// Set a field by name; false if there is no such field.
    pub fn set(&mut self, name: &str, value: f32) -> bool {
        let Some(i) = Self::index_of(name) else { return false };
        let mut a = self.to_array();
        a[i] = value;
        *self = Self::from_array(&a);
        true
    }

    /// The look preset at combo index `i` (0 = Standard … 8 = Standard 2).
    pub fn preset(i: usize) -> Option<LookParams> {
        LOOK_PRESETS.get(i).map(|p| p.1)
    }
}

impl ToneParams {
    /// The tonescale preset at combo index `i` (1 = Low Contrast … 13); 0 (USE LOOK
    /// PRESET) has none.
    pub fn preset(i: usize) -> Option<ToneParams> {
        i.checked_sub(1).and_then(|k| TONESCALE_PRESETS.get(k)).map(|p| p.1)
    }
}

macro_rules! index_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($(#[$vm])* $v),* }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$v),*];
            /// The DCTL combo-box index.
            pub fn index(self) -> usize { self as usize }
            pub fn from_index(i: usize) -> Option<Self> { Self::ALL.get(i).copied() }
        }
    };
}

index_enum!(
    /// `in_gamut`.
    InputGamut { Xyz, Aces2065_1, AcesCg, P3D65, Rec2020, Rec709, ArriWideGamut3, ArriWideGamut4,
        RedWideGamutRgb, SGamut3, SGamut3Cine, VGamut, EGamut, EGamut2, DaVinciWideGamut }
);
index_enum!(
    /// `in_oetf`.
    InputOetf { Linear, DaVinciIntermediate, FilmlightTLog, AcesCct, ArriLogC3, ArriLogC4,
        RedLog3G10, PanasonicVLog, SonySLog3, FujiFLog2 }
);
index_enum!(
    /// `display_gamut`.
    DisplayGamut { Rec709, P3D65, Rec2020P3Limited, P3D60, P3Dci, Xyz }
);
index_enum!(
    /// `eotf` (the display's; OpenDRT applies its inverse).
    Eotf {
        Linear,
        /// 2.2 power (sRGB display).
        Power22,
        /// 2.4 power (Rec.1886).
        Power24,
        /// 2.6 power (DCI).
        Power26,
        /// SMPTE ST 2084 PQ.
        Pq,
        /// BT.2100 HLG.
        Hlg,
    }
);

/// Everything OpenDRT's transform depends on (the DCTL's stickshift mode).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub in_gamut: InputGamut,
    pub in_oetf: InputOetf,
    /// Display Peak Luminance, nits (`tn_Lp`).
    pub peak_nits: f32,
    /// HDR Grey Boost (`tn_gb`).
    pub hdr_grey_boost: f32,
    /// HDR Purity (`pt_hdr`).
    pub hdr_purity: f32,
    /// Display Grey Luminance, nits (`tn_Lg`).
    pub grey_nits: f32,
    /// Multiplier on `grey_nits` (`lg_scale`). Not in stock OpenDRT (always 1 there): the
    /// LUT Match presets written by this crate carry their own grey this way.
    pub grey_scale: f32,
    pub look: LookParams,
    /// Surround compensation: 0 dark, 1 dim, 2 bright (`tn_su`).
    pub surround: u8,
    pub display_gamut: DisplayGamut,
    pub eotf: Eotf,
    /// Clamp the output to 0–1 (`clamp`).
    pub clamp: bool,
}

/// The DCTL's preset menus.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Presets {
    /// Look Preset combo index.
    pub look: usize,
    /// Tonescale Preset combo index (0 = USE LOOK PRESET).
    pub tonescale: usize,
    /// Creative White combo index (0 = USE LOOK PRESET, 1 = D93 … 6 = D50).
    pub creative_white: usize,
    /// Creative White Limit (only used when `creative_white` ≠ 0).
    pub creative_white_limit: f32,
    /// Display Encoding Preset combo index (9 and up exist only in DCTLs written by
    /// [`crate::writer`]; see [`ADDED_DISPLAY_ENCODINGS`]).
    pub display_encoding: usize,
}

impl Default for Presets {
    /// The DCTL's defaults.
    fn default() -> Self {
        Presets { look: 0, tonescale: 0, creative_white: 0, creative_white_limit: 0.25, display_encoding: 0 }
    }
}

/// (display gamut, EOTF, surround) for each Display Encoding Preset, as in the bundled
/// OpenDRT v1.1.0.1 (which sets surround to dark for every preset).
const DISPLAY_ENCODINGS: [(DisplayGamut, Eotf, u8); 9] = [
    (DisplayGamut::Rec709, Eotf::Power24, 0),
    (DisplayGamut::Rec709, Eotf::Power22, 0),
    (DisplayGamut::P3D65, Eotf::Power22, 0),
    (DisplayGamut::P3D60, Eotf::Power26, 0),
    (DisplayGamut::P3Dci, Eotf::Power26, 0),
    (DisplayGamut::Xyz, Eotf::Power26, 0),
    (DisplayGamut::Rec2020P3Limited, Eotf::Pq, 0),
    (DisplayGamut::Rec2020P3Limited, Eotf::Hlg, 0),
    (DisplayGamut::P3D65, Eotf::Pq, 0),
];

/// Display Encoding Presets that the DCTLs written by [`crate::writer`] add after the
/// bundled ones (indices 9…): (display gamut, EOTF, surround, menu label). Not in the
/// bundled DCTL itself.
pub const ADDED_DISPLAY_ENCODINGS: [(DisplayGamut, Eotf, u8, &str); 1] = [
    // 9: P3-D65 with a 2.6 power EOTF (D65 cinema projection), dark surround like the
    // other 2.6 presets.
    (DisplayGamut::P3D65, Eotf::Power26, 0, "DCI - 2.6 Power / P3-D65"),
];

/// (display gamut, EOTF, surround) for Display Encoding Preset `i`: the bundled ones, then
/// [`ADDED_DISPLAY_ENCODINGS`].
fn display_encoding(i: usize) -> Option<(DisplayGamut, Eotf, u8)> {
    DISPLAY_ENCODINGS.get(i).copied().or_else(|| {
        let (g, e, s, _) = *ADDED_DISPLAY_ENCODINGS.get(i.checked_sub(DISPLAY_ENCODINGS.len())?)?;
        Some((g, e, s))
    })
}

impl Config {
    /// Resolve the preset menus exactly like the DCTL (look, then tonescale override,
    /// creative white override, clamp on, display encoding). Input and sliders are the
    /// DCTL defaults (DaVinci Wide Gamut / Intermediate, 100 nits peak, 10 nits grey);
    /// change the fields afterwards as needed. None if an index is out of range.
    pub fn from_presets(p: &Presets) -> Option<Config> {
        Config::from_look(LookParams::preset(p.look)?, p)
    }

    /// [`Config::from_presets`] with `look` in place of the look preset `p.look`.
    pub fn from_look(look: LookParams, p: &Presets) -> Option<Config> {
        let mut look = look;
        if p.tonescale != 0 {
            ToneParams::preset(p.tonescale)?.apply_to(&mut look);
        }
        if p.creative_white != 0 {
            if p.creative_white > 6 {
                return None;
            }
            look.cwp_lm = p.creative_white_limit;
            look.cwp = (p.creative_white - 1) as f32;
        }
        let (display_gamut, eotf, surround) = display_encoding(p.display_encoding)?;
        Some(Config {
            in_gamut: InputGamut::DaVinciWideGamut,
            in_oetf: InputOetf::DaVinciIntermediate,
            peak_nits: 100.0,
            hdr_grey_boost: 0.13,
            hdr_purity: 0.5,
            grey_nits: 10.0,
            grey_scale: 1.0,
            look,
            surround,
            display_gamut,
            eotf,
            clamp: true,
        })
    }
}

impl Default for Config {
    /// The DCTL with all its defaults (Standard look, Rec.1886).
    fn default() -> Self {
        Config::from_presets(&Presets::default()).expect("default presets exist")
    }
}
