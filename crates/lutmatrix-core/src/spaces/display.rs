//! Display spaces: the LUT *output* side.
//!
//! Display-linear values are expressed in **cd/m² (nits)** throughout, so SDR and
//! PQ outputs share one unit. SDR gamma outputs need a nominal peak luminance
//! (`sdr_peak_nits`, default 100) to convert relative signal to nits.

use serde::{Deserialize, Serialize};

use super::gamuts;
use crate::math::{Mat3, Primaries, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayGamut {
    P3D65,
    P3Dci,
    Rec709,
    Rec2020,
}

impl DisplayGamut {
    pub const ALL: [DisplayGamut; 4] = [
        DisplayGamut::Rec709,
        DisplayGamut::P3D65,
        DisplayGamut::P3Dci,
        DisplayGamut::Rec2020,
    ];

    pub fn primaries(self) -> Primaries {
        match self {
            DisplayGamut::P3D65 => gamuts::P3_D65,
            DisplayGamut::P3Dci => gamuts::P3_DCI,
            DisplayGamut::Rec709 => gamuts::REC709,
            DisplayGamut::Rec2020 => gamuts::REC2020,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DisplayGamut::P3D65 => "P3-D65",
            DisplayGamut::P3Dci => "P3-DCI",
            DisplayGamut::Rec709 => "Rec.709",
            DisplayGamut::Rec2020 => "Rec.2020",
        }
    }
}

/// PQ peak-luminance presets offered in the UI (any value 100–10000 is accepted).
pub const PQ_PEAK_PRESETS: [f64; 7] = [100.0, 300.0, 600.0, 1000.0, 2000.0, 4000.0, 10000.0];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplayTransfer {
    /// Pure power-law EOTF, L = V^gamma (2.2, 2.4 or 2.6).
    Gamma { gamma: f64 },
    /// SMPTE ST 2084 PQ. `peak_nits` is the mastering peak the LUT targets; it does not
    /// change the (absolute) PQ decode but marks where output clips.
    Pq { peak_nits: f64 },
}

impl DisplayTransfer {
    pub fn name(&self) -> String {
        match self {
            DisplayTransfer::Gamma { gamma } => format!("Gamma {gamma:.1}"),
            DisplayTransfer::Pq { peak_nits } => format!("PQ (ST 2084), {peak_nits:.0} nits"),
        }
    }

    pub fn is_hdr(&self) -> bool {
        matches!(self, DisplayTransfer::Pq { .. })
    }
}

// ---------------------------------------------------------------------------
// SMPTE ST 2084:2014 §5.2 / ITU-R BT.2100-2 Table 4 constants.
// ---------------------------------------------------------------------------
const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;
pub const PQ_MAX_NITS: f64 = 10000.0;

/// PQ EOTF: non-linear signal [0,1] → absolute luminance in cd/m².
pub fn pq_eotf(n: f64) -> f64 {
    let n = n.max(0.0);
    let p = n.powf(1.0 / PQ_M2);
    let num = (p - PQ_C1).max(0.0);
    let den = PQ_C2 - PQ_C3 * p;
    PQ_MAX_NITS * (num / den).powf(1.0 / PQ_M1)
}

/// Inverse PQ EOTF: absolute luminance in cd/m² → non-linear signal [0,1].
pub fn pq_inverse_eotf(nits: f64) -> f64 {
    let y = (nits / PQ_MAX_NITS).max(0.0);
    let p = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * p) / (1.0 + PQ_C3 * p)).powf(PQ_M2)
}

/// A fully specified LUT output space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OutputSpace {
    pub gamut: DisplayGamut,
    pub transfer: DisplayTransfer,
    /// Nominal peak for gamma (SDR) outputs, cd/m². Default 100 (BT.2035 grading
    /// reference for broadcast; set 48 for DCI cinema).
    #[serde(default = "default_sdr_peak")]
    pub sdr_peak_nits: f64,
}

fn default_sdr_peak() -> f64 {
    100.0
}

impl OutputSpace {
    pub fn new(gamut: DisplayGamut, transfer: DisplayTransfer) -> Self {
        Self { gamut, transfer, sdr_peak_nits: 100.0 }
    }

    pub fn name(&self) -> String {
        format!("{} / {}", self.gamut.name(), self.transfer.name())
    }

    /// Peak luminance the LUT can produce, cd/m².
    pub fn peak_nits(&self) -> f64 {
        match self.transfer {
            DisplayTransfer::Gamma { .. } => self.sdr_peak_nits,
            DisplayTransfer::Pq { peak_nits } => peak_nits,
        }
    }

    /// Non-linear signal → display-linear cd/m² (per channel).
    pub fn decode_channel(&self, v: f64) -> f64 {
        match self.transfer {
            DisplayTransfer::Gamma { gamma } => self.sdr_peak_nits * v.max(0.0).powf(gamma),
            DisplayTransfer::Pq { .. } => pq_eotf(v),
        }
    }

    /// Display-linear cd/m² → non-linear signal (per channel).
    pub fn encode_channel(&self, nits: f64) -> f64 {
        match self.transfer {
            DisplayTransfer::Gamma { gamma } => (nits / self.sdr_peak_nits).max(0.0).powf(1.0 / gamma),
            DisplayTransfer::Pq { .. } => pq_inverse_eotf(nits),
        }
    }

    pub fn decode(&self, rgb: &Vec3) -> Vec3 {
        rgb.map(|v| self.decode_channel(v))
    }

    pub fn encode(&self, rgb: &Vec3) -> Vec3 {
        rgb.map(|v| self.encode_channel(v))
    }

    /// Display-linear RGB (nits) → absolute XYZ (Y in nits). No chromatic adaptation:
    /// P3-DCI white stays DCI white in XYZ (that is what "preserve LUT white" keeps).
    pub fn rgb_to_xyz(&self) -> Mat3 {
        self.gamut.primaries().rgb_to_xyz()
    }

    pub fn xyz_to_rgb(&self) -> Mat3 {
        self.gamut.primaries().xyz_to_rgb()
    }
}

/// Default display middle gray, cd/m².
///
/// - PQ: 26 cd/m², the 18% grey card level of ITU-R BT.2408-8 (11/2024) §2.2 Table 1
///   (38% PQ; HDR Reference White = 203 cd/m², 58% PQ).
/// - SDR: no ITU value exists. We use the same placement relative to reference white,
///   26/203 ≈ 12.81% of SDR peak (≈ code 0.425 at gamma 2.4). This is a convention,
///   not a standard; it is a user setting (DECISIONS.md D7).
pub fn default_display_middle_gray_nits(out: &OutputSpace) -> f64 {
    match out.transfer {
        DisplayTransfer::Pq { .. } => BT2408_GREY_NITS,
        DisplayTransfer::Gamma { .. } => out.sdr_peak_nits * BT2408_GREY_NITS / BT2408_REF_WHITE_NITS,
    }
}

/// ITU-R BT.2408-8 Table 1: Grey Card (18%).
pub const BT2408_GREY_NITS: f64 = 26.0;
/// ITU-R BT.2408-8 Table 1: HDR Reference White (100%), also diffuse/graphics white.
pub const BT2408_REF_WHITE_NITS: f64 = 203.0;
