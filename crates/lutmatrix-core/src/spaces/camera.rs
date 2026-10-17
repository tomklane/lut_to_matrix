//! Camera log spaces: the LUT *input* side.
//!
//! Each entry in [`CAMERA_SPACES`] ties a log curve to a gamut and lists the
//! keywords used to detect it from filenames, TITLE lines and comments.
//!
//! **Adding a camera space** = one new entry here (plus a [`LogCurve`] variant if
//! the curve is new). Detection, the UI pop-up menu, the CLI and the fixtures all
//! iterate over this table.

use serde::{Deserialize, Serialize};

use super::curves::LogCurve;
use super::gamuts;
use crate::math::{whites, Cat, Mat3, Primaries, Vec3};

/// A detection keyword. Matching is done on a normalised, lower-case form of the
/// text with separators removed (see `detect.rs`), so write keywords that way:
/// "logc4", "sgamut3cine".
#[derive(Debug, Clone, Copy)]
pub struct Keyword {
    pub text: &'static str,
    /// Evidence weight. ≥ 3 is decisive on its own ("logc4"), 1 is a hint ("alexa").
    pub weight: f64,
    /// Keywords that, if present in the same text, cancel this one
    /// (e.g. "slog3" for S-Gamut3 is cancelled by "cine").
    pub unless: &'static [&'static str],
}

const fn kw(text: &'static str, weight: f64) -> Keyword {
    Keyword { text, weight, unless: &[] }
}
const fn kw_unless(text: &'static str, weight: f64, unless: &'static [&'static str]) -> Keyword {
    Keyword { text, weight, unless }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraSpaceId {
    ArriLogC3Awg3,
    ArriLogC4Awg4,
    SonySLog3SGamut3,
    SonySLog3SGamut3Cine,
    RedLog3G10Rwg,
    CanonLog2CinemaGamut,
    CanonLog3CinemaGamut,
    PanasonicVLogVGamut,
    BmdFilmGen5,
    FujiFLogFGamut,
    FujiFLog2FGamut,
    DaVinciIntermediateDwg,
    AcesCctAp1,
}

#[derive(Debug, Clone)]
pub struct CameraSpace {
    pub id: CameraSpaceId,
    /// Short UI label.
    pub name: &'static str,
    pub curve: LogCurve,
    pub gamut: Primaries,
    pub gamut_name: &'static str,
    pub keywords: &'static [Keyword],
}

pub static CAMERA_SPACES: &[CameraSpace] = &[
    CameraSpace {
        id: CameraSpaceId::ArriLogC3Awg3,
        name: "ARRI LogC3 / AWG3",
        curve: LogCurve::ArriLogC3 { ei: 800 },
        gamut: gamuts::AWG3,
        gamut_name: "ARRI Wide Gamut 3",
        keywords: &[
            kw("logc3", 4.0),
            kw("logcv3", 4.0),
            kw("awg3", 4.0),
            kw("alexawidegamut", 3.0),
            // ARRI's alexalutconv names LUTs "LogC2Video" (Log C → video).
            kw("logc2video", 3.0),
            kw_unless("logc", 2.0, &["logc4", "logc2video"]),
            kw_unless("alexa", 1.0, &["alexa35"]),
            kw("k1s1", 1.0),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::ArriLogC4Awg4,
        name: "ARRI LogC4 / AWG4",
        curve: LogCurve::ArriLogC4,
        gamut: gamuts::AWG4,
        gamut_name: "ARRI Wide Gamut 4",
        keywords: &[
            kw("logc4", 4.0),
            kw("awg4", 4.0),
            kw("alexa35", 3.0),
            kw("reveal", 1.0),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::SonySLog3SGamut3,
        name: "Sony S-Log3 / S-Gamut3",
        curve: LogCurve::SLog3,
        gamut: gamuts::S_GAMUT3,
        gamut_name: "S-Gamut3",
        keywords: &[
            kw_unless("sgamut3", 4.0, &["sgamut3cine", "sgamut3c"]),
            kw_unless("slog3", 2.0, &["cine"]),
            kw("venice", 0.5),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::SonySLog3SGamut3Cine,
        name: "Sony S-Log3 / S-Gamut3.Cine",
        curve: LogCurve::SLog3,
        gamut: gamuts::S_GAMUT3_CINE,
        gamut_name: "S-Gamut3.Cine",
        keywords: &[
            kw("sgamut3cine", 5.0),
            kw("sgamut3c", 4.0),
            kw("slog3cine", 4.0),
            kw_unless("slog3", 2.0, &["sgamut3"]),
            kw("venice", 0.5),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::RedLog3G10Rwg,
        name: "RED Log3G10 / RWG",
        curve: LogCurve::Log3G10,
        gamut: gamuts::RWG,
        gamut_name: "REDWideGamutRGB",
        keywords: &[
            kw("log3g10", 4.0),
            kw("redwidegamut", 4.0),
            kw("rwg", 3.0),
            kw("ipp2", 2.0),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::CanonLog2CinemaGamut,
        name: "Canon Log 2 / Cinema Gamut",
        curve: LogCurve::CanonLog2,
        gamut: gamuts::CINEMA_GAMUT,
        gamut_name: "Canon Cinema Gamut",
        keywords: &[
            kw("canonlog2", 4.0),
            kw("clog2", 4.0),
            kw_unless("cinemagamut", 1.5, &["clog3", "canonlog3"]),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::CanonLog3CinemaGamut,
        name: "Canon Log 3 / Cinema Gamut",
        curve: LogCurve::CanonLog3,
        gamut: gamuts::CINEMA_GAMUT,
        gamut_name: "Canon Cinema Gamut",
        keywords: &[
            kw("canonlog3", 4.0),
            kw("clog3", 4.0),
            kw_unless("cinemagamut", 1.5, &["clog2", "canonlog2"]),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::PanasonicVLogVGamut,
        name: "Panasonic V-Log / V-Gamut",
        curve: LogCurve::VLog,
        gamut: gamuts::V_GAMUT,
        gamut_name: "V-Gamut",
        keywords: &[kw("vlog", 4.0), kw("vgamut", 4.0), kw("varicam", 2.0)],
    },
    CameraSpace {
        id: CameraSpaceId::BmdFilmGen5,
        name: "Blackmagic Film Gen 5 / BMD WG",
        curve: LogCurve::BmdFilmGen5,
        gamut: gamuts::BMD_WG_GEN5,
        gamut_name: "Blackmagic Wide Gamut Gen 5",
        keywords: &[
            kw("bmdfilmgen5", 5.0),
            kw("blackmagicfilmgen5", 5.0),
            kw("gen5", 3.0),
            kw("bmdwg", 3.0),
            kw("blackmagicwidegamut", 3.0),
            kw("bmdfilm", 2.0),
            kw("blackmagic", 1.0),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::FujiFLogFGamut,
        name: "Fujifilm F-Log / F-Gamut",
        curve: LogCurve::FLog,
        gamut: gamuts::F_GAMUT,
        gamut_name: "F-Gamut",
        keywords: &[
            kw_unless("flog", 4.0, &["flog2"]),
            kw_unless("fgamut", 1.5, &["flog2"]),
            kw_unless("fujifilm", 1.0, &["flog2"]),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::FujiFLog2FGamut,
        name: "Fujifilm F-Log2 / F-Gamut",
        curve: LogCurve::FLog2,
        gamut: gamuts::F_GAMUT,
        gamut_name: "F-Gamut",
        keywords: &[kw("flog2", 5.0)],
    },
    CameraSpace {
        id: CameraSpaceId::DaVinciIntermediateDwg,
        name: "DaVinci Intermediate / DWG",
        curve: LogCurve::DaVinciIntermediate,
        gamut: gamuts::DWG,
        gamut_name: "DaVinci Wide Gamut",
        keywords: &[
            kw("davinciintermediate", 5.0),
            kw("davinciwidegamut", 4.0),
            kw("dwg", 3.0),
            kw("davinciwg", 3.0),
        ],
    },
    CameraSpace {
        id: CameraSpaceId::AcesCctAp1,
        name: "ACEScct / AP1",
        curve: LogCurve::AcesCct,
        gamut: gamuts::AP1,
        gamut_name: "ACES AP1",
        keywords: &[kw("acescct", 5.0), kw("ap1", 2.0), kw("aces", 1.0)],
    },
];

pub fn camera_space(id: CameraSpaceId) -> &'static CameraSpace {
    CAMERA_SPACES
        .iter()
        .find(|s| s.id == id)
        .expect("every CameraSpaceId has a registry entry")
}

/// A fully specified LUT input space: the registry entry plus per-LUT options.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InputSpace {
    pub id: CameraSpaceId,
    /// ARRI LogC3 exposure index (ignored by other curves).
    #[serde(default = "default_ei")]
    pub arri_ei: u16,
    /// Chromatic adaptation used when the gamut's white is not D65 (ACES AP1, BMD WG).
    #[serde(default)]
    pub white_cat: Cat,
}

fn default_ei() -> u16 {
    800
}

impl InputSpace {
    pub fn new(id: CameraSpaceId) -> Self {
        Self { id, arri_ei: 800, white_cat: Cat::Bradford }
    }

    pub fn def(&self) -> &'static CameraSpace {
        camera_space(self.id)
    }

    pub fn curve(&self) -> LogCurve {
        match self.def().curve {
            LogCurve::ArriLogC3 { .. } => LogCurve::ArriLogC3 { ei: self.arri_ei },
            c => c,
        }
    }

    pub fn name(&self) -> String {
        match self.def().curve {
            LogCurve::ArriLogC3 { .. } => format!("{} (EI {})", self.def().name, self.arri_ei),
            _ => self.def().name.to_string(),
        }
    }

    /// Camera-linear RGB → CIE XYZ, adapted to D65 when the gamut white differs
    /// (DECISIONS.md D6), so that neutral input is always D65.
    pub fn rgb_to_xyz_d65(&self) -> Mat3 {
        let g = self.def().gamut;
        let m = g.rgb_to_xyz();
        if g.white == whites::D65 {
            m
        } else {
            self.white_cat.matrix(g.white, whites::D65) * m
        }
    }

    pub fn xyz_d65_to_rgb(&self) -> Mat3 {
        self.rgb_to_xyz_d65().try_inverse().expect("invertible")
    }

    /// Scene-linear camera RGB → LUT input code values.
    pub fn encode(&self, rgb: &Vec3) -> Vec3 {
        let c = self.curve();
        rgb.map(|v| c.encode(v))
    }

    pub fn decode(&self, code: &Vec3) -> Vec3 {
        let c = self.curve();
        code.map(|v| c.decode(v))
    }
}
