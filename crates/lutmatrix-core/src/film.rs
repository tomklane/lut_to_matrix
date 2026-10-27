//! Film stages: naming negative and print LUTs, and stacking them (DECISIONS.md D36).
//!
//! A *negative* LUT takes camera log to a film-negative encoding (e.g. Cineon printing
//! density); a *print* LUT takes that encoding to a display. A [`Stack`] runs a negative
//! then a print, exactly as two LUT nodes in Resolve would (no resampling), so every
//! analysis and the OpenDRT match can treat the pair as one LUT.
//!
//! Names: a stacked pair is called `negative_print`, using only film stocks:
//! - negative: the camera stock number from the file name plus any pull/push
//!   (`AWG4LC4_to_5201_pull2.cube` → `5201pull2`);
//! - print: from the user's [`StockNames`] table, which maps look codes and profile
//!   variants to words (`DLook4-N_gn_V2_…` → `2383v1_D60_Neutralized_V2`); a creative
//!   white move `DxxtoDyy_Bradford` adds `_Dyy`.
//!   The table is the user's (it describes their LUT pack), so it lives in their data
//!   folder, not in the app.
//! - negatives without a stock number that write film log (Deluxe FilmLog 33/34
//!   input transforms) are `FilmLog`.
//! - full LUTs (camera log → display) are named on their own: `LogC34_<print>` is
//!   `FilmLog_<print>`.
//! - names that come out the same get a, b, c… ([`dedupe`]).

use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::lut::{Interpolation, Lut, Transform};
use crate::math::Vec3;

/// A negative LUT followed by a print LUT (or any chain), applied in order.
#[derive(Debug, Clone)]
pub struct Stack {
    pub stages: Vec<Lut>,
}

impl Transform for Stack {
    fn apply_rgb(&self, rgb: &Vec3, interp: Interpolation) -> Vec3 {
        self.stages.iter().fold(*rgb, |v, l| l.apply(&v, interp))
    }
    /// The first stage's input range; later stages take whatever it produces (clamped
    /// to their range, as Resolve does).
    fn in_domain_rgb(&self, rgb: &Vec3) -> bool {
        self.stages.first().map_or(true, |l| l.in_domain(rgb))
    }
    /// The last stage's largest entries bound the output.
    fn output_max(&self) -> Option<[f64; 3]> {
        self.stages.last().and_then(|l| l.output_max())
    }
}

/// The user's names for print looks and profile variants.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StockNames {
    /// Look code → stock name, e.g. "DLook4" → "2383v1". Matched at the start of the
    /// file name, case-insensitively, and only as a whole code ("DLook4" ≠ "DLook42").
    pub looks: BTreeMap<String, String>,
    /// Variant code → word, e.g. "N" → "D60", "U" → "3514DIConstrain".
    pub variants: BTreeMap<String, String>,
    /// The flag after the variant (`_gn`) and its word, e.g. "gn" → "Neutralized".
    pub flags: BTreeMap<String, String>,
    /// Negative file-name prefix → name, for negatives the built-in rules don't cover.
    pub negatives: BTreeMap<String, String>,
}

/// The longest key of `table` that starts `stem` as a whole word (case-insensitive).
fn prefix_match<'a>(table: &'a BTreeMap<String, String>, stem: &str) -> Option<(&'a String, &'a String)> {
    let lower = stem.to_ascii_lowercase();
    let mut keys: Vec<_> = table.iter().collect();
    keys.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    keys.into_iter().find(|(k, _)| {
        let k = k.to_ascii_lowercase();
        lower.starts_with(&k) && lower[k.len()..].chars().next().map_or(true, |c| !c.is_ascii_alphanumeric())
    })
}

fn stem_of(file_name: &str) -> &str {
    file_name.rsplit_once('.').map_or(file_name, |(a, _)| a)
}

/// What the name of a print file says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintName {
    /// The full name, e.g. "2383v1_D60_Neutralized_V2".
    pub name: String,
    /// The look code as written in the table ("DLook4").
    pub look: String,
    /// The variant code ("N"), if the file has one.
    pub variant: Option<String>,
    /// Flag codes after the variant ("gn").
    pub flags: Vec<String>,
    /// Profile revision ("V2").
    pub revision: Option<String>,
}

impl StockNames {
    /// `stock-names.json` in the user's data folder (next to the preset library).
    pub fn default_path() -> Option<std::path::PathBuf> {
        crate::preset_store::data_dir().ok().map(|d| d.join("stock-names.json"))
    }

    /// Read a names file; a missing file is an empty table.
    pub fn load(path: &std::path::Path) -> Result<StockNames, String> {
        match std::fs::read_to_string(path) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StockNames::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// The name of a print LUT from its file name; `None` if no look code matches (the
    /// caller falls back to the file name, and the user can rename).
    pub fn print_name(&self, file_name: &str) -> Option<PrintName> {
        let stem = stem_of(file_name);
        // Longest code first, so "DLook10" wins over "DLook1".
        let (code, stock) = prefix_match(&self.looks, stem)?;
        let rest = &stem[code.len()..];
        let mut parts = vec![stock.clone()];
        let mut variant = None;
        let mut flags = Vec::new();
        let mut revision = None;
        // "-N_gn_V2_Profile_…": the variant follows '-', then '_'-separated flags and revision.
        if let Some(r) = rest.strip_prefix('-') {
            let mut toks = r.split('_');
            if let Some(v) = toks.next() {
                let word = self.variants.iter().find(|(k, _)| k.eq_ignore_ascii_case(v));
                if let Some((k, w)) = word {
                    variant = Some(k.clone());
                    parts.push(w.clone());
                }
            }
            for t in toks {
                if let Some((k, w)) = self.flags.iter().find(|(k, _)| k.eq_ignore_ascii_case(t)) {
                    flags.push(k.clone());
                    parts.push(w.clone());
                } else if revision_re().is_match(t) {
                    let rev = t.to_ascii_uppercase();
                    parts.push(rev.clone());
                    revision = Some(rev);
                } else {
                    break;
                }
            }
        }
        // A creative white moved with Bradford: "D55toD58_Bradford" → "D58".
        if let Some(c) = white_move_re().captures(stem) {
            parts.push(format!("D{}", &c[1]));
        }
        Some(PrintName { name: parts.join("_"), look: code.clone(), variant, flags, revision })
    }

    /// The name of a negative: a table entry, else the camera stock number and pull/push
    /// ("5201pull2"), else "FilmLog" for a film-log transform without a stock.
    pub fn negative_name(&self, file_name: &str) -> Option<String> {
        let stem = stem_of(file_name);
        if let Some((_, n)) = prefix_match(&self.negatives, stem) {
            return Some(n.clone());
        }
        negative_name(file_name).or_else(|| stem.to_ascii_lowercase().contains("filmlog").then(|| "FilmLog".to_string()))
    }

    /// The name of a full LUT (camera log → display), matched on its own:
    /// - `[33X_]LogC33_` / `LogC34_` + a print → `FilmLog_<print name>` (they are the
    ///   FilmLog input transform baked in front of that print);
    /// - a negative stock and a print stock in the name (`…5207_pull2_K2393…`) →
    ///   `5207pull2_2393`;
    /// - else the stock-names table, if it knows the file.
    pub fn full_name(&self, file_name: &str) -> Option<String> {
        let stem = stem_of(file_name);
        if let Some(c) = logc_film_re().captures(stem) {
            let rest = &stem[c.get(0).unwrap().end()..];
            let print = self.print_name(rest).map(|p| p.name).unwrap_or_else(|| {
                let lower = rest.to_ascii_lowercase();
                let end = ["_to_", "_profile"].iter().filter_map(|t| lower.find(t)).min().unwrap_or(rest.len());
                rest[..end].to_string()
            });
            return Some(format!("FilmLog_{print}"));
        }
        if let (Some(n), Some(p)) = (negative_name(file_name), print_stock_re().captures(stem)) {
            return Some(format!("{n}_{}", &p[1]));
        }
        self.print_name(file_name).map(|p| p.name)
    }

    /// The automatic name for a LUT at `stage` (the file name if no rule applies).
    pub fn name_for(&self, stage: Stage, file_name: &str) -> String {
        match stage {
            Stage::Negative => self.negative_name(file_name),
            Stage::Print => self.print_name(file_name).map(|p| p.name),
            Stage::Full => self.full_name(file_name),
        }
        .unwrap_or_else(|| stem_of(file_name).to_string())
    }
}

/// Names that come out the same get a lowercase letter each, in order: a, b, c … z,
/// aa, ab … ("5219JBs1" ×2 → "5219JBs1a", "5219JBs1b"). Unique names are left alone.
pub fn dedupe(names: &mut [String]) {
    let mut count = std::collections::HashMap::new();
    for n in names.iter() {
        *count.entry(n.clone()).or_insert(0usize) += 1;
    }
    let mut seen = std::collections::HashMap::new();
    for n in names.iter_mut() {
        if count[n.as_str()] > 1 {
            let k = seen.entry(n.clone()).or_insert(0usize);
            let mut i = *k;
            *k += 1;
            let mut tag = String::new();
            loop {
                tag.insert(0, (b'a' + (i % 26) as u8) as char);
                if i < 26 {
                    break;
                }
                i = i / 26 - 1;
            }
            n.push_str(&tag);
        }
    }
}

fn white_move_re() -> &'static Regex {
    static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)D\d{2}toD(\d{2})_?Bradford").unwrap())
}

fn logc_film_re() -> &'static Regex {
    static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^(?:\d+X_)?LogC3[34]_").unwrap())
}

fn print_stock_re() -> &'static Regex {
    static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?:^|[^0-9])(2383|2393|2395|3513|3514|3521|3523)(?:[^0-9]|$)").unwrap())
}

fn revision_re() -> &'static Regex {
    static R: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^v\d+$").unwrap())
}

/// The negative's name from its file name: the camera stock number plus any pull/push
/// ("5201pull2"). `None` if no stock number is found.
pub fn negative_name(file_name: &str) -> Option<String> {
    static STOCK: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static PUSH: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let stem = file_name.rsplit_once('.').map_or(file_name, |(a, _)| a);
    // Kodak 52xx/72xx/50xx, Fuji 85xx/86xx camera stocks: four digits on their own.
    let stock = STOCK.get_or_init(|| Regex::new(r"(?:^|[^0-9])((?:5[0-2]|7[2-3]|8[5-6])\d{2})(?:[^0-9]|$)").unwrap());
    let push = PUSH.get_or_init(|| Regex::new(r"(?i)(pull|push)[ _\-]?(\d)").unwrap());
    let s = stock.captures(stem)?.get(1)?.as_str().to_string();
    Some(match push.captures(stem) {
        Some(c) => format!("{s}{}{}", c[1].to_ascii_lowercase(), &c[2]),
        None => s,
    })
}

/// Which stage a LUT is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Camera log → film negative (film log out).
    Negative,
    /// Film negative → display.
    Print,
    /// Camera log → display: already a whole pipeline; matched on its own, not stacked.
    Full,
}

/// The stage of a LUT, measured (DECISIONS.md D37), and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageMeasure {
    pub stage: Stage,
    /// How fast the output rises through the mid-tones (where it passes code 0.42):
    /// output code per stop of exposure when the input camera space is known, else
    /// output code per input code.
    pub slope: f64,
    /// `slope` is per stop (the input space is known).
    pub per_stop: bool,
    pub reason: String,
}

/// Output code per stop below which a LUT is log → log (a negative). Log encodings run
/// at 0.06–0.10 per stop (LogC3 0.071, Cineon-style negatives ~0.09); display outputs
/// at 0.13 or more through the mid-tones.
pub const NEGATIVE_PER_STOP: f64 = 0.115;
/// The same test when the input space is unknown: output code per input code.
pub const NEGATIVE_SLOPE: f64 = 1.4;

/// Measure what stage a LUT is:
/// 1. **Camera space in?** From the name or header (medium or high confidence: "LogC",
///    "AWG", "ACES2065"…). If so, scene gray is swept −10…+10 stops through it;
///    otherwise the neutral axis of the input code is swept.
/// 2. **Log or display out?** Where the output passes the mid-tones (code 0.42), a log
///    output rises like a log encoding (≤ 0.115 code per stop, or ≤ 1.4 per input code); a
///    display output two to three times faster.
/// 3. Camera in, log out → **negative**; camera in, display out → **full**; anything
///    else in (film log), display out → **print**.
pub fn measure_stage(lut: &Lut) -> StageMeasure {
    let mean = |o: Vec3| (o.x + o.y + o.z) / 3.0;
    let d = crate::detect::detect(lut);
    let camera = d.input.confidence >= crate::detect::Confidence::Medium;
    // (x, output) along the ramp, x in stops (camera) or input code (unknown).
    let ramp: Vec<(f64, f64)> = if camera {
        let inp = d.input.value;
        let to_cam = inp.xyz_d65_to_rgb();
        let wn = crate::math::whites::D65.to_xyz();
        (0..=320)
            .filter_map(|k| {
                let s = -10.0 + k as f64 / 16.0;
                let code = inp.encode(&(to_cam * (wn * (0.18 * 2f64.powf(s)))));
                lut.in_domain(&code).then(|| (s, mean(lut.apply(&code, Interpolation::Tetrahedral))))
            })
            .collect()
    } else {
        let (lo, hi) = lut.input_domain();
        let (lo, hi) = (lo.iter().cloned().fold(f64::NEG_INFINITY, f64::max), hi.iter().cloned().fold(f64::INFINITY, f64::min));
        (0..=512)
            .map(|k| {
                let c = lo + (hi - lo) * k as f64 / 512.0;
                (c, mean(lut.apply(&Vec3::new(c, c, c), Interpolation::Tetrahedral)))
            })
            .collect()
    };
    let span = if camera { 1.0 } else { 0.06 };
    let slope = ramp
        .windows(2)
        .position(|w| w[0].1 < 0.42 && w[1].1 >= 0.42)
        .map(|k| {
            let x = ramp[k].0;
            let near = |t: f64| ramp.iter().min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs())).copied().unwrap();
            let (a, b) = (near(x - span / 2.0), near(x + span / 2.0));
            (b.1 - a.1) / (b.0 - a.0).max(1e-9)
        })
        .unwrap_or(0.0); // never reaches the mid-tones: a dark log encoding
    let log_out = if camera { slope < NEGATIVE_PER_STOP } else { slope < NEGATIVE_SLOPE };
    let how = if camera { format!("{slope:.3} code per stop") } else { format!("slope {slope:.2}") };
    let (stage, reason) = match (camera, log_out) {
        (true, true) => (Stage::Negative, format!("{} in, log out ({how})", d.input.value.name())),
        (true, false) => (Stage::Full, format!("{} in, display out ({how})", d.input.value.name())),
        (false, true) => (Stage::Negative, format!("log out ({how}); input space not recognised")),
        (false, false) => (Stage::Print, format!("film log in, display out ({how})")),
    };
    StageMeasure { stage, slope, per_stop: camera, reason }
}

/// The stacked pair's name, `negative_print`.
pub fn pair_name(negative: &str, print: &str) -> String {
    format!("{negative}_{print}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lut::{FnTransform, Lut3D};

    fn names() -> StockNames {
        let m = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        StockNames {
            looks: m(&[("DLook1", "1111"), ("DLook10", "2393v2"), ("DLook4", "2383v1")]),
            variants: m(&[("NAT", "Native"), ("N", "D60"), ("P3", "DCI"), ("U", "3514DIConstrain")]),
            flags: m(&[("gn", "Neutralized")]),
            ..StockNames::default()
        }
    }

    #[test]
    fn print_names_follow_the_table() {
        let n = names();
        let p = n.print_name("DLook4-N_gn_V2_Profile_To_Rec709_2-4_G1_Og1_P1_LumToZero.dat").unwrap();
        assert_eq!(p.name, "2383v1_D60_Neutralized_V2");
        assert_eq!((p.look.as_str(), p.variant.as_deref(), p.revision.as_deref()), ("DLook4", Some("N"), Some("V2")));
        assert_eq!(p.flags, vec!["gn"]);
        assert_eq!(n.print_name("Dlook4-NAT_Profile_To_Rec709.dat").unwrap().name, "2383v1_Native", "case-insensitive");
        assert_eq!(n.print_name("DLook10-P3_gn_Profile.dat").unwrap().name, "2393v2_DCI_Neutralized", "DLook10, not DLook1");
        assert_eq!(n.print_name("DLook10-U_Profile.dat").unwrap().name, "2393v2_3514DIConstrain");
        assert!(n.print_name("DLook42D60ToDciP3_To_Rec709.dat").is_none(), "DLook4 is not a prefix of DLook42");
        assert!(n.print_name("Other.cube").is_none());
        assert_eq!(
            n.print_name("DLook10-NAT_Profile_To_DciP3_2-6_G1_Og1_D55toD58_Bradford_P1_LumToZero.dat").unwrap().name,
            "2393v2_Native_D58",
            "creative white move"
        );
    }

    #[test]
    fn full_lut_names() {
        let n = names();
        assert_eq!(
            n.full_name("LogC34_DLook4-N_gn_V2_Profile_To_DciP3_2-6_G1_Og1_P1_LumToZero.dat").as_deref(),
            Some("FilmLog_2383v1_D60_Neutralized_V2")
        );
        assert_eq!(n.full_name("33X_LogC33_Ulook4b_To_Rec709_2-4_G1_Og1_P1_LumToZero.cube").as_deref(), Some("FilmLog_Ulook4b"));
        assert_eq!(n.full_name("ACES2065_to_5207_pull2_K2393_D65_709G24.cube").as_deref(), Some("5207pull2_2393"));
        assert_eq!(n.name_for(Stage::Full, "AlexaV3_K1S1_LogC2Video_Rec709_EE.cube"), "AlexaV3_K1S1_LogC2Video_Rec709_EE");
    }

    #[test]
    fn duplicates_get_letters() {
        let mut v: Vec<String> = ["FilmLog", "5201pull2", "FilmLog", "FilmLog"].iter().map(|s| s.to_string()).collect();
        dedupe(&mut v);
        assert_eq!(v, ["FilmLoga", "5201pull2", "FilmLogb", "FilmLogc"]);
        let mut many: Vec<String> = vec!["x".into(); 28];
        dedupe(&mut many);
        assert_eq!((many[0].as_str(), many[25].as_str(), many[26].as_str(), many[27].as_str()), ("xa", "xz", "xaa", "xab"));
    }

    #[test]
    fn negative_names_keep_stock_and_pull_push() {
        assert_eq!(negative_name("AWG4LC4_to_5201_pull2.cube").as_deref(), Some("5201pull2"));
        assert_eq!(negative_name("LogC3_5219_Push1.cube").as_deref(), Some("5219push1"));
        assert_eq!(negative_name("ACES2065_to_5207_neg.cube").as_deref(), Some("5207"));
        assert_eq!(negative_name("Eterna_8543.cube").as_deref(), Some("8543"));
        assert_eq!(negative_name("LogC4_to_Rec709.cube"), None, "no stock number");
        assert_eq!(pair_name("5201pull2", "2383v1_Native"), "5201pull2_2383v1_Native");
        let n = names();
        assert_eq!(n.negative_name("ARRILogCWG_to_34FilmLog.cube").as_deref(), Some("FilmLog"));
        assert_eq!(n.negative_name("LogCv3ToFilmLogInput33.cube").as_deref(), Some("FilmLog"));
        assert_eq!(n.negative_name("AWG4LC4_to_5201_pull2.cube").as_deref(), Some("5201pull2"));
        assert_eq!(n.name_for(Stage::Negative, "Mystery.cube"), "Mystery");
    }

    #[test]
    fn stack_applies_in_order() {
        let a = Lut::from_3d(Lut3D::from_fn(9, |c| c * 0.5));
        let b = Lut::from_3d(Lut3D::from_fn(9, |c| c.map(|v| v * v)));
        let x = Vec3::new(0.8, 0.4, 0.2);
        let by_hand = b.apply(&a.apply(&x, Interpolation::Tetrahedral), Interpolation::Tetrahedral);
        let s = Stack { stages: vec![a, b] };
        assert_eq!(s.apply_rgb(&x, Interpolation::Tetrahedral), by_hand, "exactly two LUT nodes in a row");
        assert!((by_hand - Vec3::new(0.16, 0.04, 0.01)).abs().max() < 5e-3, "{by_hand:?}");
        let f = FnTransform(|c: &Vec3| *c);
        assert!(f.output_max().is_none());
    }
}
