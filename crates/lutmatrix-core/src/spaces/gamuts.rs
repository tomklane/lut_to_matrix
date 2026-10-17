//! RGB gamut definitions (CIE 1931 xy primaries and white), each with its source.
//!
//! RGB↔XYZ matrices are always derived from these primaries in f64
//! ([`Primaries::rgb_to_xyz`]) rather than copied from rounded published matrices,
//! so every space is treated identically. Tests compare the derived matrices to the
//! manufacturers' published ones where those exist.

use crate::math::{whites, Primaries, Xy};

/// ARRI Wide Gamut 3. Source: ARRI, "ALEXA Log C Curve – Usage in VFX" (2017), §"ALEXA
/// Wide Gamut" primaries table.
pub const AWG3: Primaries = Primaries {
    r: Xy::new(0.6840, 0.3130),
    g: Xy::new(0.2210, 0.8480),
    b: Xy::new(0.0861, -0.1020),
    white: whites::D65,
};

/// ARRI Wide Gamut 4. Source: ARRI, "ARRI LogC4 Logarithmic Color Space –
/// Specification" (2022-05-01), Table 1.
pub const AWG4: Primaries = Primaries {
    r: Xy::new(0.7347, 0.2653),
    g: Xy::new(0.1424, 0.8576),
    b: Xy::new(0.0991, -0.0308),
    white: whites::D65,
};

/// Sony S-Gamut3. Source: Sony, "Technical Summary for S-Gamut3.Cine/S-Log3 and
/// S-Gamut3/S-Log3" v1.00, §1.
pub const S_GAMUT3: Primaries = Primaries {
    r: Xy::new(0.730, 0.280),
    g: Xy::new(0.140, 0.855),
    b: Xy::new(0.100, -0.050),
    white: whites::D65,
};

/// Sony S-Gamut3.Cine. Source: as [`S_GAMUT3`].
pub const S_GAMUT3_CINE: Primaries = Primaries {
    r: Xy::new(0.766, 0.275),
    g: Xy::new(0.225, 0.800),
    b: Xy::new(0.089, -0.087),
    white: whites::D65,
};

/// REDWideGamutRGB. Source: RED, "White Paper on REDWideGamutRGB and Log3G10" Rev-C (2017).
pub const RWG: Primaries = Primaries {
    r: Xy::new(0.780_308, 0.304_253),
    g: Xy::new(0.121_595, 1.493_994),
    b: Xy::new(0.095_612, -0.084_589),
    white: whites::D65,
};

/// Canon Cinema Gamut. Source: Canon, "Input Transform Version 202007" IDT
/// documentation / Canon Cinema Gamut definition (C300 Mark II and later).
pub const CINEMA_GAMUT: Primaries = Primaries {
    r: Xy::new(0.74, 0.27),
    g: Xy::new(0.17, 1.14),
    b: Xy::new(0.08, -0.10),
    white: whites::D65,
};

/// Panasonic V-Gamut. Source: Panasonic, "VARICAM V-Log/V-Gamut" (2014), p. 4.
pub const V_GAMUT: Primaries = Primaries {
    r: Xy::new(0.730, 0.280),
    g: Xy::new(0.165, 0.840),
    b: Xy::new(0.100, -0.030),
    white: whites::D65,
};

/// Blackmagic Wide Gamut (Gen 5). Source: Blackmagic Design, "Blackmagic Generation 5
/// Color Science" (2021), primaries table. Note the white is Blackmagic's own D65
/// approximation (see [`whites::BMD_WG`]).
pub const BMD_WG_GEN5: Primaries = Primaries {
    r: Xy::new(0.717_721_5, 0.317_118_1),
    g: Xy::new(0.228_041, 0.861_569),
    b: Xy::new(0.100_584_1, -0.082_045_2),
    white: whites::BMD_WG,
};

/// ITU-R BT.2020 primaries (Rec. ITU-R BT.2020-2, Table 3). Also Fujifilm F-Gamut:
/// "F-Log Data Sheet" Ver.1.1 states F-Log gamut complies with BT.2020.
pub const REC2020: Primaries = Primaries {
    r: Xy::new(0.708, 0.292),
    g: Xy::new(0.170, 0.797),
    b: Xy::new(0.131, 0.046),
    white: whites::D65,
};

/// Fujifilm F-Gamut = BT.2020 primaries, D65.
pub const F_GAMUT: Primaries = REC2020;

/// DaVinci Wide Gamut. Source: Blackmagic Design, "DaVinci Wide Gamut Intermediate"
/// v1.1 (2021-07-31), "DaVinci Wide Gamut" chromaticity table.
pub const DWG: Primaries = Primaries {
    r: Xy::new(0.8000, 0.3130),
    g: Xy::new(0.1682, 0.9877),
    b: Xy::new(0.0790, -0.1155),
    white: whites::D65,
};

/// ACES AP0 (ACES2065-1). Source: SMPTE ST 2065-1:2012 §4.1 (ACES RGB primaries) /
/// AMPAS TB-2014-004 Table 1; white = ACES.
pub const AP0: Primaries = Primaries {
    r: Xy::new(0.7347, 0.2653),
    g: Xy::new(0.0, 1.0),
    b: Xy::new(0.0001, -0.0770),
    white: whites::ACES,
};

/// ACES AP1 (ACEScg / ACEScct). Source: AMPAS S-2014-004 (ACEScg) §4.1; white = ACES.
pub const AP1: Primaries = Primaries {
    r: Xy::new(0.713, 0.293),
    g: Xy::new(0.165, 0.830),
    b: Xy::new(0.128, 0.044),
    white: whites::ACES,
};

/// ITU-R BT.709-6, Table 1 item 1.3.
pub const REC709: Primaries = Primaries {
    r: Xy::new(0.64, 0.33),
    g: Xy::new(0.30, 0.60),
    b: Xy::new(0.15, 0.06),
    white: whites::D65,
};

/// P3 primaries with D65 white. SMPTE EG 432-1:2010 (P3-D65).
pub const P3_D65: Primaries = Primaries {
    r: Xy::new(0.680, 0.320),
    g: Xy::new(0.265, 0.690),
    b: Xy::new(0.150, 0.060),
    white: whites::D65,
};

/// P3 primaries with DCI white. SMPTE RP 431-2:2011 Table A.1.
pub const P3_DCI: Primaries = Primaries {
    white: whites::DCI,
    ..P3_D65
};
