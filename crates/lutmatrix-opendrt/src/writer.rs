//! Write a copy of the bundled OpenDRT DCTL with extra "LUT Match" look presets.
//!
//! Each preset sets every look parameter, plus its own middle-gray level as part of its
//! tonescale: `lg_scale` multiplies the Display Grey Luminance slider, but only with
//! Tonescale Preset = USE LOOK PRESET. With any stock tonescale preset `lg_scale` is 1,
//! so every look shares the slider's exposure (user decision, DECISIONS.md D24).
//! The stock presets are untouched.

use crate::params::{Config, Kind, LookParams, Presets, ADDED_DISPLAY_ENCODINGS};
use crate::DCTL_SOURCE;

/// One preset to add.
#[derive(Debug, Clone, PartialEq)]
pub struct LutMatchPreset {
    /// Shown in the Look Preset menu (sanitised to DCTL-safe ASCII).
    pub name: String,
    pub look: LookParams,
    /// Display Grey Luminance the fit found, nits (at the DCTL's default slider of 10).
    pub grey_nits: f32,
    /// What it was fitted to (a comment in the DCTL).
    pub source: String,
}

impl LutMatchPreset {
    /// `lg_scale` as the DCTL computes it: the literal written for `grey_nits / 10`.
    pub fn lg_scale(&self) -> f32 {
        self.grey_nits / 10.0
    }

    /// The OpenDRT configuration Resolve runs for this preset with the other menus set
    /// as in `p` (its `look` is ignored) and the Display Grey Luminance slider at
    /// `grey_slider`.
    pub fn config(&self, p: &Presets, grey_slider: f32) -> Option<Config> {
        let mut cfg = Config::from_look(self.look, p)?;
        cfg.grey_nits = grey_slider;
        cfg.grey_scale = if p.tonescale == 0 { self.lg_scale() } else { 1.0 };
        Some(cfg)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("preset name '{0}' is empty after removing characters a DCTL menu cannot show")]
    BadName(String),
    #[error("the bundled DCTL no longer has the expected text: {0}")]
    Source(&'static str),
    #[error("{0} is not finite in preset '{1}'")]
    NotFinite(&'static str, String),
}

/// Menu-safe name: ASCII letters, digits, space and `_-.+/&#` only (commas and braces
/// would break `DEFINE_UI_PARAMS`).
pub fn sanitize_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || " _-.+/&#".contains(c) { c } else { ' ' })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A float as a C literal that reads back as exactly this `f32`.
fn lit(v: f32) -> String {
    let s = format!("{v:?}"); // shortest round-trip: "1.0", "0.1", "1e-10"
    format!("{s}f")
}

fn replace_once(src: &str, from: &str, to: &str, what: &'static str) -> Result<String, WriteError> {
    if src.matches(from).count() != 1 {
        return Err(WriteError::Source(what));
    }
    Ok(src.replacen(from, to, 1))
}

/// The bundled OpenDRT DCTL with `presets` appended to the Look Preset menu.
pub fn lut_match_dctl(presets: &[LutMatchPreset], note: &str) -> Result<String, WriteError> {
    let mut src = DCTL_SOURCE.to_string();
    let n0 = crate::tables::LOOK_PRESET_LABELS.len();

    // Look Preset menu.
    let ids: String = (0..n0).map(|i| format!("look{i}")).collect::<Vec<_>>().join(", ");
    let names = crate::tables::LOOK_PRESET_LABELS.join(", ");
    let combo = format!("DEFINE_UI_PARAMS(look_preset, Look Preset, DCTLUI_COMBO_BOX, 0, {{{ids}}}, {{{names}}})");
    let mut ids2 = ids.clone();
    let mut names2 = names.clone();
    let mut clean = Vec::new();
    for (i, p) in presets.iter().enumerate() {
        let name = sanitize_name(&p.name);
        if name.is_empty() {
            return Err(WriteError::BadName(p.name.clone()));
        }
        ids2.push_str(&format!(", look{}", n0 + i));
        names2.push_str(&format!(", {name}"));
        clean.push(name);
    }
    let combo2 = format!("DEFINE_UI_PARAMS(look_preset, Look Preset, DCTLUI_COMBO_BOX, 0, {{{ids2}}}, {{{names2}}})");
    src = replace_once(&src, &combo, &combo2, "look preset menu")?;

    // Extra Display Encoding Presets, appended so the stock indices keep their meaning.
    src = add_display_encodings(src)?;

    // Modification notice (GPLv3 §5a).
    let note = sanitize_comment(note);
    src = replace_once(
        &src,
        "  License: GPLv3\n  -------------------------------------------------*/",
        &format!(
            "  License: GPLv3\n  -------------------------------------------------\n\n  MODIFIED by LUT Matrix {} (https://github.com/tomklane/lut_to_matrix): added\n  'LUT Match' look presets fitted to specific LUTs, a per-preset Display Grey\n  Luminance scale (lg_scale), applied only with Tonescale Preset = USE LOOK PRESET,\n  and a 'DCI - 2.6 Power / P3-D65' Display Encoding Preset.\n  Original presets unchanged. Distributed under GPLv3.{}\n  -------------------------------------------------*/",
            env!("CARGO_PKG_VERSION"),
            if note.is_empty() { String::new() } else { format!("\n  {note}") }
        ),
        "license header",
    )?;

    // lg_scale variable after the look parameter declarations.
    let decl_end = "hs_y, hs_y_rng;";
    let start = src.find("// Init look preset variables").ok_or(WriteError::Source("parameter declarations"))?;
    let at = src[start..].find(decl_end).ok_or(WriteError::Source("parameter declarations"))? + start + decl_end.len();
    src.insert_str(at, "\n  float lg_scale = 1.0f; // per-preset Display Grey Luminance scale (LUT Match presets)");

    // Preset branches after the last stock look.
    let last = format!("  else if (look_preset=={}) {{ // {}\n", n0 - 1, crate::tables::LOOK_PRESET_LABELS[n0 - 1]);
    let b0 = src.find(&last).ok_or(WriteError::Source("last stock look preset"))?;
    let b1 = src[b0..].find("\n  }\n").ok_or(WriteError::Source("last stock look preset"))? + b0 + "\n  }\n".len();
    let info = LookParams::info();
    let mut branches = String::new();
    for (i, (p, name)) in presets.iter().zip(&clean).enumerate() {
        if !p.grey_nits.is_finite() {
            return Err(WriteError::NotFinite("grey", name.clone()));
        }
        let vals: Vec<String> = info
            .iter()
            .zip(p.look.to_array())
            .map(|(pi, v)| {
                if !v.is_finite() {
                    return Err(WriteError::NotFinite(pi.name, name.clone()));
                }
                Ok(match pi.kind {
                    Kind::Float => format!("{} = {}", pi.name, lit(v)),
                    _ => format!("{} = {}", pi.name, v.round() as i32),
                })
            })
            .collect::<Result<_, _>>()?;
        branches.push_str(&format!(
            "  else if (look_preset=={}) {{ // {name}: fitted to {}; grey {:.2} nits at slider 10\n    {};\n    lg_scale = {};\n  }}\n",
            n0 + i,
            sanitize_comment(&p.source),
            p.grey_nits,
            vals.join(", "),
            lit(p.lg_scale()),
        ));
    }
    src.insert_str(b1, &branches);

    // Stock tonescale presets revert to the slider's exposure.
    let ts = "  // if (tonescale_preset==0) // Do nothing and use tonescale settings from the look preset\n";
    src = replace_once(
        &src,
        ts,
        &format!("{ts}  // LUT Match presets carry their own grey (lg_scale) as part of their tonescale; a stock\n  // tonescale preset reverts exposure to the Display Grey Luminance slider for every look.\n  if (tonescale_preset!=0) lg_scale = 1.0f;\n"),
        "tonescale presets",
    )?;
    src = replace_once(&src, "const float ts_y0 = tn_Lg/100.0f*", "const float ts_y0 = tn_Lg*lg_scale/100.0f*", "ts_y0")?;
    src = replace_once(
        &src,
        "_powf((tn_Lg/100.0f), -1.0f/tn_con)",
        "_powf((tn_Lg*lg_scale/100.0f), -1.0f/tn_con)",
        "s_Lp100",
    )?;
    debug_assert!(src.is_ascii());
    Ok(src)
}

/// Append [`ADDED_DISPLAY_ENCODINGS`] to the Display Encoding Preset menu, each with a
/// branch after the last stock one setting surround, display gamut and EOTF.
fn add_display_encodings(src: String) -> Result<String, WriteError> {
    let n0 = crate::tables::DISPLAY_ENCODING_PRESET_LABELS.len();
    let mut ids = String::new();
    let mut labels = String::new();
    let mut branches = String::new();
    for (k, (gamut, eotf, surround, label)) in ADDED_DISPLAY_ENCODINGS.iter().enumerate() {
        let i = n0 + k;
        ids.push_str(&format!(", denc{i}"));
        labels.push_str(&format!(", {label}"));
        branches.push_str(&format!(
            "  else if (display_encoding_preset=={i}) {{ // {label} (added by LUT Matrix)\n    tn_su = {surround};\n    display_gamut = {};\n    eotf = {};\n  }}\n",
            gamut.index(),
            eotf.index(),
        ));
    }
    let last_label = crate::tables::DISPLAY_ENCODING_PRESET_LABELS[n0 - 1];
    let mut src = replace_once(&src, "dolbypq}, {",&format!("dolbypq{ids}}}, {{"), "display encoding ids")?;
    src = replace_once(&src, &format!("{last_label}}})"), &format!("{last_label}{labels}}})"), "display encoding labels")?;
    let last = format!("  else if (display_encoding_preset=={}) {{", n0 - 1);
    let b0 = src.find(&last).ok_or(WriteError::Source("last display encoding preset"))?;
    let b1 = src[b0..].find("\n  }\n").ok_or(WriteError::Source("last display encoding preset"))? + b0 + "\n  }\n".len();
    src.insert_str(b1, &branches);
    Ok(src)
}

/// One-line, ASCII, no comment terminators.
fn sanitize_comment(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c } else { ' ' }).collect();
    s.replace("*/", "* /").replace("/*", "/ *").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(name: &str) -> LutMatchPreset {
        let mut look = LookParams::preset(3).unwrap();
        look.tn_con = 1.234_567_9;
        look.brl_r = -1e-7;
        LutMatchPreset { name: name.into(), look, grey_nits: 7.3, source: "x.cube".into() }
    }

    #[test]
    fn writes_menu_branches_and_exposure_rule() {
        let d = lut_match_dctl(&[preset("LUT Match {A}, b"), preset("Second")], "").unwrap();
        assert!(d.is_ascii());
        assert!(d.contains("Standard 2, LUT Match A b, Second})"));
        assert!(d.contains("look9, look10}"));
        assert!(d.contains("else if (look_preset==10) { // Second: fitted to x.cube"));
        assert!(d.contains("tn_con = 1.2345679f"));
        assert!(d.contains("brl_r = -1e-7f"));
        assert!(d.contains("lg_scale = 0.73f;"));
        assert!(d.contains("if (tonescale_preset!=0) lg_scale = 1.0f;"));
        // header, declaration, 2 presets, tonescale rule (comment + code), ts_y0, s_Lp100
        assert_eq!(d.matches("lg_scale").count(), 8);
        // P3-D65 2.6 display encoding appended to the menu, with its branch.
        assert!(d.contains("rec2100hlg, dolbypq, denc9}, {"));
        assert!(d.contains("Dolby - PQ / P3-D65, DCI - 2.6 Power / P3-D65})"));
        assert!(d.contains(
            "  else if (display_encoding_preset==9) { // DCI - 2.6 Power / P3-D65 (added by LUT Matrix)\n    tn_su = 0;\n    display_gamut = 1;\n    eotf = 3;\n  }\n"
        ));
        // Stock presets unchanged: removing the additions gives back the original text.
        assert!(d.contains(&DCTL_SOURCE[DCTL_SOURCE.find("if (look_preset==0)").unwrap()..DCTL_SOURCE.find("  else if (look_preset==8)").unwrap()]));
    }

    #[test]
    fn literals_round_trip() {
        for v in [1.0f32, 0.1, 1.234_567_9, -3e-9, 6.0e-1, 24.999_998] {
            let s = lit(v);
            assert_eq!(s.trim_end_matches('f').parse::<f32>().unwrap(), v);
        }
    }

    #[test]
    fn rejects_empty_names() {
        assert!(lut_match_dctl(&[preset("{},")], "").is_err());
    }
}
