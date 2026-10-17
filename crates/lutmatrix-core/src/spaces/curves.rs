//! Camera log encodings (scene-linear ↔ full-range normalised code value).
//!
//! Conventions, for every curve:
//! - Linear input is *scene-linear reflectance*: 0.18 = an 18% gray card.
//! - Code values are full-range normalised (0.0 = code 0, 1.0 = code 1023 at 10 bit),
//!   which is what Resolve feeds a `.cube` LUT. See DECISIONS.md D4.
//! - Each constant cites the manufacturer document it comes from. Values were
//!   cross-checked against colour-science 0.4.7; fixtures in `tests/fixtures`.
//!
//! To add a curve: add a variant to [`LogCurve`], implement both directions in
//! `encode`/`decode`, give it a name, then register a camera space that uses it in
//! `camera.rs`, add a fixture row in `tools/gen_fixtures.py` and regenerate.

use serde::{Deserialize, Serialize};

/// ARRI LogC3 exposure indices with published parameters.
pub const ARRI_LOGC3_EIS: [u16; 11] = [160, 200, 250, 320, 400, 500, 640, 800, 1000, 1280, 1600];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "curve", rename_all = "snake_case")]
pub enum LogCurve {
    /// ARRI LogC3 (ALEXA classic / Mini / LF / Mini LF), SUP 3.x, depends on EI.
    ArriLogC3 { ei: u16 },
    /// ARRI LogC4 (ALEXA 35 and later).
    ArriLogC4,
    /// Sony S-Log3.
    SLog3,
    /// RED Log3G10 (IPP2, version 3 with linear toe below -0.01).
    Log3G10,
    /// Canon Log 2 (v1.2, full range).
    CanonLog2,
    /// Canon Log 3 (v1.2, full range).
    CanonLog3,
    /// Panasonic V-Log.
    VLog,
    /// Blackmagic Film Generation 5.
    BmdFilmGen5,
    /// Fujifilm F-Log.
    FLog,
    /// Fujifilm F-Log2.
    FLog2,
    /// DaVinci Intermediate.
    DaVinciIntermediate,
    /// ACEScct.
    AcesCct,
}

// ---------------------------------------------------------------------------
// ARRI LogC3
// Source: ARRI, "ALEXA Log C Curve – Usage in VFX" (H. Brendel, 2017-03-09),
// Table "LogC curve parameters, SUP 3.x, linear scene exposure factor".
// Columns: cut, a, b, c, d, e, f.  0.18 linear = 18% gray.
// ---------------------------------------------------------------------------
const LOGC3_SUP3: [(u16, [f64; 7]); 11] = [
    (160, [0.005561, 5.555556, 0.080216, 0.269036, 0.381991, 5.842037, 0.092778]),
    (200, [0.006208, 5.555556, 0.076621, 0.266007, 0.382478, 5.776265, 0.092782]),
    (250, [0.006871, 5.555556, 0.072941, 0.262978, 0.382966, 5.710494, 0.092786]),
    (320, [0.007622, 5.555556, 0.068768, 0.259627, 0.383508, 5.637732, 0.092791]),
    (400, [0.008318, 5.555556, 0.064901, 0.256598, 0.383999, 5.571960, 0.092795]),
    (500, [0.009031, 5.555556, 0.060939, 0.253569, 0.384493, 5.506188, 0.092800]),
    (640, [0.009840, 5.555556, 0.056443, 0.250219, 0.385040, 5.433426, 0.092805]),
    (800, [0.010591, 5.555556, 0.052272, 0.247190, 0.385537, 5.367655, 0.092809]),
    (1000, [0.011361, 5.555556, 0.047996, 0.244161, 0.386036, 5.301883, 0.092814]),
    (1280, [0.012235, 5.555556, 0.043137, 0.240810, 0.386590, 5.229121, 0.092819]),
    (1600, [0.013047, 5.555556, 0.038625, 0.237781, 0.387093, 5.163350, 0.092824]),
];

fn logc3_params(ei: u16) -> [f64; 7] {
    LOGC3_SUP3
        .iter()
        .find(|(e, _)| *e == ei)
        .map(|(_, p)| *p)
        .unwrap_or(LOGC3_SUP3[7].1) // EI 800
}

// ---------------------------------------------------------------------------
// ARRI LogC4
// Source: ARRI, "ARRI LogC4 Logarithmic Color Space – Specification" (S. Cooper,
// H. Brendel), revision 2022-05-01, Eq. 1–3 and Appendix A CTL.
// ---------------------------------------------------------------------------
fn logc4_consts() -> (f64, f64, f64, f64, f64) {
    let a = (2f64.powi(18) - 16.0) / 117.45;
    let b = (1023.0 - 95.0) / 1023.0;
    let c = 95.0 / 1023.0;
    let s = (7.0 * 2f64.ln() * 2f64.powf(7.0 - 14.0 * c / b)) / (a * b);
    let t = (2f64.powf(14.0 * (-c / b) + 6.0) - 64.0) / a;
    (a, b, c, s, t)
}

// ---------------------------------------------------------------------------
// Sony S-Log3
// Source: Sony, "Technical Summary for S-Gamut3.Cine/S-Log3 and S-Gamut3/S-Log3"
// v1.00, §2 "S-Log3 formula" (reflection in, 10-bit code value / 1023 out).
// ---------------------------------------------------------------------------
const SLOG3_CUT_LIN: f64 = 0.011_25;
const SLOG3_CUT_CODE10: f64 = 171.210_294_692_9;

// ---------------------------------------------------------------------------
// RED Log3G10 (v3)
// Source: RED Digital Cinema, "White Paper on REDWideGamutRGB and Log3G10",
// Rev-C (2017-11-06): a = 0.224282, b = 155.975327, c = 0.01, g = 15.1927.
// ---------------------------------------------------------------------------
const L3G10_A: f64 = 0.224_282;
const L3G10_B: f64 = 155.975_327;
const L3G10_C: f64 = 0.01;
const L3G10_G: f64 = 15.1927;

// ---------------------------------------------------------------------------
// Canon Log 2 / Canon Log 3 (v1.2, "Full %" formulas)
// Source: Canon, "Description of the Canon Log, Canon Log 2 and Canon Log 3 Gamma
// Curves" white paper (2018-11-01), Appendix [2] and [3]; constants as in Canon's
// "Input Transform Version 202007" IDTs. Canon's linear x is relative to 90% white,
// so scene reflectance is divided by 0.9 first. Output is full-range.
// ---------------------------------------------------------------------------
const CLOG2_A: f64 = 0.241_360_77;
const CLOG2_B: f64 = 87.099_375_46;
const CLOG2_C: f64 = 0.092_864_125;

const CLOG3_LOG_A: f64 = 0.367_268_45;
const CLOG3_LOG_B: f64 = 14.983_25;
const CLOG3_HI_C: f64 = 0.122_405_37;
const CLOG3_LO_C: f64 = 0.127_839_01;
const CLOG3_LIN_M: f64 = 1.975_479_8;
const CLOG3_LIN_C: f64 = 0.125_122_19;
const CLOG3_CUT_LO: f64 = 0.097_465_473;
const CLOG3_CUT_HI: f64 = 0.152_778_91;

// ---------------------------------------------------------------------------
// Panasonic V-Log
// Source: Panasonic, "VARICAM V-Log/V-Gamut" reference manual (2014), p. 3:
// cut1 = 0.01, cut2 = 0.181, b = 0.00873, c = 0.241514, d = 0.598206.
// ---------------------------------------------------------------------------
const VLOG_CUT1: f64 = 0.01;
const VLOG_CUT2: f64 = 0.181;
const VLOG_B: f64 = 0.008_73;
const VLOG_C: f64 = 0.241_514;
const VLOG_D: f64 = 0.598_206;

// ---------------------------------------------------------------------------
// Blackmagic Film Generation 5
// Source: Blackmagic Design, "Blackmagic Generation 5 Color Science" (2021),
// Appendix "Blackmagic Film Generation 5 OETF".
// ---------------------------------------------------------------------------
const BMD5_A: f64 = 0.086_928_760_654_912_24;
const BMD5_B: f64 = 0.005_494_072_432_257_808;
const BMD5_C: f64 = 0.530_013_339_229_193_9;
const BMD5_D: f64 = 8.283_605_932_402_494;
const BMD5_E: f64 = 0.092_465_753_424_657_53;
const BMD5_LIN_CUT: f64 = 0.005;

// ---------------------------------------------------------------------------
// Fujifilm F-Log / F-Log2
// Sources: Fujifilm, "F-Log Data Sheet" Ver.1.1 (2022) and "F-Log2 Data Sheet"
// Ver.1.0 (2022): out = c·log10(a·in + b) + d (in ≥ cut1), e·in + f otherwise;
// in = scene-linear reflection.
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
struct FLogConsts {
    cut1: f64,
    cut2: f64,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}
const FLOG: FLogConsts = FLogConsts {
    cut1: 0.000_89,
    cut2: 0.100_537_775_223_865,
    a: 0.555_556,
    b: 0.009_468,
    c: 0.344_676,
    d: 0.790_453,
    e: 8.735_631,
    f: 0.092_864,
};
const FLOG2: FLogConsts = FLogConsts {
    cut1: 0.000_889,
    cut2: 0.100_686_685_370_811,
    a: 5.555_556,
    b: 0.064_829,
    c: 0.245_281,
    d: 0.384_316,
    e: 8.799_461,
    f: 0.092_864,
};

fn flog_encode(k: &FLogConsts, x: f64) -> f64 {
    if x < k.cut1 {
        k.e * x + k.f
    } else {
        k.c * (k.a * x + k.b).log10() + k.d
    }
}
fn flog_decode(k: &FLogConsts, y: f64) -> f64 {
    if y < k.cut2 {
        (y - k.f) / k.e
    } else {
        10f64.powf((y - k.d) / k.c) / k.a - k.b / k.a
    }
}

// ---------------------------------------------------------------------------
// DaVinci Intermediate
// Source: Blackmagic Design, "DaVinci Wide Gamut Intermediate" v1.1 (2021-07-31,
// published August 2021), "DaVinci Intermediate OETF".
// ---------------------------------------------------------------------------
pub const DI_A: f64 = 0.0075;
pub const DI_B: f64 = 7.0;
pub const DI_C: f64 = 0.073_292_48;
pub const DI_M: f64 = 10.444_268_55;
pub const DI_LIN_CUT: f64 = 0.002_624_09;
pub const DI_LOG_CUT: f64 = 0.027_406_68;

pub fn di_encode(l: f64) -> f64 {
    if l <= DI_LIN_CUT {
        l * DI_M
    } else {
        ((l + DI_A).log2() + DI_B) * DI_C
    }
}
pub fn di_decode(v: f64) -> f64 {
    if v <= DI_LOG_CUT {
        v / DI_M
    } else {
        2f64.powf(v / DI_C - DI_B) - DI_A
    }
}

// ---------------------------------------------------------------------------
// ACEScct
// Source: AMPAS, "ACEScct – A Quasi-Logarithmic Encoding of ACES Data for use within
// Color Grading Systems", S-2016-001 (2016), §4.4–4.5.
// ---------------------------------------------------------------------------
const ACESCCT_X_BRK: f64 = 0.007_812_5;
const ACESCCT_Y_BRK: f64 = 0.155_251_141_552_511;
const ACESCCT_A: f64 = 10.540_237_741_654_5;
const ACESCCT_B: f64 = 0.072_905_534_195_835_5;

impl LogCurve {
    pub fn name(&self) -> String {
        match self {
            LogCurve::ArriLogC3 { ei } => format!("ARRI LogC3 (EI {ei})"),
            LogCurve::ArriLogC4 => "ARRI LogC4".into(),
            LogCurve::SLog3 => "Sony S-Log3".into(),
            LogCurve::Log3G10 => "RED Log3G10".into(),
            LogCurve::CanonLog2 => "Canon Log 2".into(),
            LogCurve::CanonLog3 => "Canon Log 3".into(),
            LogCurve::VLog => "Panasonic V-Log".into(),
            LogCurve::BmdFilmGen5 => "Blackmagic Film Gen 5".into(),
            LogCurve::FLog => "Fujifilm F-Log".into(),
            LogCurve::FLog2 => "Fujifilm F-Log2".into(),
            LogCurve::DaVinciIntermediate => "DaVinci Intermediate".into(),
            LogCurve::AcesCct => "ACEScct".into(),
        }
    }

    /// Scene-linear reflectance → full-range normalised code value.
    pub fn encode(&self, x: f64) -> f64 {
        match *self {
            LogCurve::ArriLogC3 { ei } => {
                let [cut, a, b, c, d, e, f] = logc3_params(ei);
                if x > cut {
                    c * (a * x + b).log10() + d
                } else {
                    e * x + f
                }
            }
            LogCurve::ArriLogC4 => {
                let (a, b, c, s, t) = logc4_consts();
                if x >= t {
                    ((a * x + 64.0).log2() - 6.0) / 14.0 * b + c
                } else {
                    (x - t) / s
                }
            }
            LogCurve::SLog3 => {
                if x >= SLOG3_CUT_LIN {
                    (420.0 + ((x + 0.01) / (0.18 + 0.01)).log10() * 261.5) / 1023.0
                } else {
                    (x * (SLOG3_CUT_CODE10 - 95.0) / SLOG3_CUT_LIN + 95.0) / 1023.0
                }
            }
            LogCurve::Log3G10 => {
                let x = x + L3G10_C;
                if x < 0.0 {
                    x * L3G10_G
                } else {
                    L3G10_A * (x * L3G10_B + 1.0).log10()
                }
            }
            LogCurve::CanonLog2 => {
                let x = x / 0.9;
                if x < 0.0 {
                    -CLOG2_A * (1.0 - x * CLOG2_B).log10() + CLOG2_C
                } else {
                    CLOG2_A * (x * CLOG2_B + 1.0).log10() + CLOG2_C
                }
            }
            LogCurve::CanonLog3 => {
                let x = x / 0.9;
                let lo = (CLOG3_CUT_LO - CLOG3_LIN_C) / CLOG3_LIN_M;
                let hi = (CLOG3_CUT_HI - CLOG3_LIN_C) / CLOG3_LIN_M;
                if x < lo {
                    -CLOG3_LOG_A * (1.0 - x * CLOG3_LOG_B).log10() + CLOG3_LO_C
                } else if x <= hi {
                    CLOG3_LIN_M * x + CLOG3_LIN_C
                } else {
                    CLOG3_LOG_A * (x * CLOG3_LOG_B + 1.0).log10() + CLOG3_HI_C
                }
            }
            LogCurve::VLog => {
                if x < VLOG_CUT1 {
                    5.6 * x + 0.125
                } else {
                    VLOG_C * (x + VLOG_B).log10() + VLOG_D
                }
            }
            LogCurve::BmdFilmGen5 => {
                if x < BMD5_LIN_CUT {
                    BMD5_D * x + BMD5_E
                } else {
                    BMD5_A * (x + BMD5_B).ln() + BMD5_C
                }
            }
            LogCurve::FLog => flog_encode(&FLOG, x),
            LogCurve::FLog2 => flog_encode(&FLOG2, x),
            LogCurve::DaVinciIntermediate => di_encode(x),
            LogCurve::AcesCct => {
                if x <= ACESCCT_X_BRK {
                    ACESCCT_A * x + ACESCCT_B
                } else {
                    (x.log2() + 9.72) / 17.52
                }
            }
        }
    }

    /// Full-range normalised code value → scene-linear reflectance.
    pub fn decode(&self, y: f64) -> f64 {
        match *self {
            LogCurve::ArriLogC3 { ei } => {
                let [cut, a, b, c, d, e, f] = logc3_params(ei);
                if y > e * cut + f {
                    (10f64.powf((y - d) / c) - b) / a
                } else {
                    (y - f) / e
                }
            }
            LogCurve::ArriLogC4 => {
                let (a, b, c, s, t) = logc4_consts();
                if y >= 0.0 {
                    (2f64.powf(14.0 * (y - c) / b + 6.0) - 64.0) / a
                } else {
                    y * s + t
                }
            }
            LogCurve::SLog3 => {
                if y >= SLOG3_CUT_CODE10 / 1023.0 {
                    10f64.powf((y * 1023.0 - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
                } else {
                    (y * 1023.0 - 95.0) * SLOG3_CUT_LIN / (SLOG3_CUT_CODE10 - 95.0)
                }
            }
            LogCurve::Log3G10 => {
                if y < 0.0 {
                    y / L3G10_G - L3G10_C
                } else {
                    (10f64.powf(y / L3G10_A) - 1.0) / L3G10_B - L3G10_C
                }
            }
            LogCurve::CanonLog2 => {
                let x = if y < CLOG2_C {
                    -(10f64.powf((CLOG2_C - y) / CLOG2_A) - 1.0) / CLOG2_B
                } else {
                    (10f64.powf((y - CLOG2_C) / CLOG2_A) - 1.0) / CLOG2_B
                };
                x * 0.9
            }
            LogCurve::CanonLog3 => {
                let x = if y < CLOG3_CUT_LO {
                    -(10f64.powf((CLOG3_LO_C - y) / CLOG3_LOG_A) - 1.0) / CLOG3_LOG_B
                } else if y <= CLOG3_CUT_HI {
                    (y - CLOG3_LIN_C) / CLOG3_LIN_M
                } else {
                    (10f64.powf((y - CLOG3_HI_C) / CLOG3_LOG_A) - 1.0) / CLOG3_LOG_B
                };
                x * 0.9
            }
            LogCurve::VLog => {
                if y < VLOG_CUT2 {
                    (y - 0.125) / 5.6
                } else {
                    10f64.powf((y - VLOG_D) / VLOG_C) - VLOG_B
                }
            }
            LogCurve::BmdFilmGen5 => {
                if y < BMD5_D * BMD5_LIN_CUT + BMD5_E {
                    (y - BMD5_E) / BMD5_D
                } else {
                    ((y - BMD5_C) / BMD5_A).exp() - BMD5_B
                }
            }
            LogCurve::FLog => flog_decode(&FLOG, y),
            LogCurve::FLog2 => flog_decode(&FLOG2, y),
            LogCurve::DaVinciIntermediate => di_decode(y),
            LogCurve::AcesCct => {
                if y <= ACESCCT_Y_BRK {
                    (y - ACESCCT_B) / ACESCCT_A
                } else {
                    2f64.powf(y * 17.52 - 9.72)
                }
            }
        }
    }
}
