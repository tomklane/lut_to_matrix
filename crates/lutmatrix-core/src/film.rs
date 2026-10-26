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
//!   variants to words (`DLook4-N_gn_V2_…` → `2383v1_D60_Neutralized_V2`).
//!   The table is the user's (it describes their LUT pack), so it lives in their data
//!   folder, not in the app.

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
        let stem = file_name.rsplit_once('.').map_or(file_name, |(a, _)| a);
        let lower = stem.to_ascii_lowercase();
        // Longest code first, so "DLook10" wins over "DLook1".
        let mut looks: Vec<_> = self.looks.iter().collect();
        looks.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
        let (code, stock) = looks.into_iter().find(|(k, _)| {
            let k = k.to_ascii_lowercase();
            lower.starts_with(&k) && lower[k.len()..].chars().next().map_or(true, |c| !c.is_ascii_alphanumeric())
        })?;
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
        Some(PrintName { name: parts.join("_"), look: code.clone(), variant, flags, revision })
    }
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
    Negative,
    Print,
}

/// A guess from the file name: a print if the stock-names table knows it or the name
/// mentions printing (film log in, a print stock number); otherwise a negative. The user
/// can move a LUT to the other list.
pub fn guess_stage(file_name: &str, names: &StockNames) -> Stage {
    if names.print_name(file_name).is_some() {
        return Stage::Print;
    }
    let squashed: String = file_name.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    const PRINT: [&str; 12] =
        ["print", "filmlog", "cineon", "dlook", "2383", "2393", "2395", "3513", "3514", "3521", "3523", "cp30"];
    if PRINT.iter().any(|k| squashed.contains(k)) && negative_name(file_name).is_none() {
        Stage::Print
    } else {
        Stage::Negative
    }
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
        assert_eq!(guess_stage("AWG4LC4_to_5201_pull2.cube", &n), Stage::Negative);
        assert_eq!(guess_stage("DLook4-N_gn_V2_Profile_To_Rec709.dat", &n), Stage::Print);
        assert_eq!(guess_stage("FilmLog_to_Kodak2383_Rec709.cube", &n), Stage::Print);
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
