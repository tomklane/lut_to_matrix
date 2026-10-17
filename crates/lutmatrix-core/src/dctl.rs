//! DaVinci Resolve DCTL and matrix exports.
//!
//! The DCTL performs, explicitly and in this order:
//! 1. DaVinci Intermediate → linear DaVinci Wide Gamut
//! 2. linear DWG → CIE XYZ (`DWG_TO_XYZ`)
//! 3. the fitted matrix in XYZ (`M_XYZ`), blended with identity by a UI slider
//! 4. XYZ → linear DWG (`XYZ_TO_DWG`)
//! 5. linear DWG → DaVinci Intermediate
//!
//! Portability (CUDA / OpenCL / Metal): only `typedef struct`, `make_float3`,
//! per-component float arithmetic, `_log2f` / `_powf` and `f`-suffixed literals are
//! used; no `__CONSTANT__` arrays, no float3 operator overloads.

use std::fmt::Write as _;

use serde::Serialize;

use crate::analysis::Analysis;
use crate::math::Mat3;
use crate::sample::WhiteMode;
use crate::spaces::curves::{DI_A, DI_B, DI_C, DI_LIN_CUT, DI_LOG_CUT, DI_M};
use crate::spaces::davinci;

/// Everything printed in the DCTL header and exports.
#[derive(Debug, Clone, Serialize)]
pub struct ExportMeta {
    pub source_lut: String,
    pub input_space: String,
    pub output_space: String,
    pub white_mode: String,
    pub middle_gray: String,
    pub fit_error: String,
    pub fit_method: String,
    /// Display transform the matrix was fitted through, if any (the DCTL must then be
    /// used directly before it).
    #[serde(default)]
    pub reference: Option<String>,
    pub version: String,
    /// ISO date (YYYY-MM-DD), injected so exports are reproducible in tests.
    pub date: String,
}

impl ExportMeta {
    pub fn from_analysis(a: &Analysis, source_lut: &str, date: &str) -> Self {
        let white_mode = match a.settings.white_mode {
            WhiteMode::D65 => format!("D65 ({:?} CAT from LUT white + exact D65 neutral constraint)", a.settings.cat),
            WhiteMode::PreserveLut if a.settings.lock_neutral => {
                "Preserve LUT white (neutral locked to the LUT's white)".into()
            }
            WhiteMode::PreserveLut => "Preserve LUT white (unconstrained fit)".into(),
        };
        let anchor = if a.anchor_stops == 0.0 {
            "anchor A: scene middle gray".to_string()
        } else {
            format!("anchor B: {:+.2} stops, where the LUT outputs display middle gray", a.anchor_stops)
        };
        let middle_gray = format!(
            "scene {:.4}, display {:.2} cd/m2 ({}); {anchor}; LUT renders scene gray at {:.2} cd/m2 ({:+.2} stops)",
            a.settings.scene_middle_gray,
            a.display_middle_gray_nits,
            a.display_middle_gray_source,
            a.gray_out_nits,
            a.stops_from_display_middle_gray,
        );
        let s = a.stats_refined.as_ref().unwrap_or(&a.stats_ls);
        let fit_error = format!(
            "{}: mean {:.3}, p95 {:.3}, max {:.3}; neutral {:.3} (du'v' {:.5}); {} samples",
            s.metric.label(),
            s.mean,
            s.p95,
            s.max,
            s.neutral,
            s.neutral_duv,
            s.n
        );
        let reference = a.reference.as_ref().map(|r| {
            format!(
                "{} ({} in, {} out); mean error alone {:.3}, with matrix {:.3} ({:.0}% better)",
                r.name, r.input_name, r.output_name, r.stats_alone.mean, s.mean, r.gain_percent
            )
        });
        let fit_method = if a.reference.is_some() {
            format!("minimise {} of reference(M) vs LUT, exposure matched, {} stop bracket", s.metric.label(), a.settings.bracket_stops)
        } else if a.m_xyz_refined.is_some() {
            format!("least squares, refined by minimising {}", s.metric.label())
        } else {
            "least squares".into()
        };
        Self {
            source_lut: source_lut.to_string(),
            input_space: a.input_name.clone(),
            output_space: a.output_name.clone(),
            white_mode,
            middle_gray,
            fit_error,
            fit_method,
            reference,
            version: crate::VERSION.to_string(),
            date: date.to_string(),
        }
    }
}

/// Today's date (UTC) as YYYY-MM-DD, without a date-time dependency.
pub fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    date_utc(secs)
}

/// Seconds since 1970 (UTC) → "YYYY-MM-DD".
pub fn date_utc(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 → (year, month, day). H. Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Float literal for DCTL: 10 decimals (more than float32 resolves) and an `f` suffix.
fn lit(v: f64) -> String {
    let v = if v == 0.0 { 0.0 } else { v }; // no "-0"
    format!("{v:.10}f")
}

fn matrix_fn(out: &mut String, name: &str, doc: &str, m: &Mat3) {
    let _ = writeln!(out, "// {doc}");
    let _ = writeln!(out, "__DEVICE__ Mat3x3 {name}()");
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "    Mat3x3 m;");
    for (r, field) in ["r0", "r1", "r2"].iter().enumerate() {
        let _ = writeln!(
            out,
            "    m.{field} = make_float3({:>16}, {:>16}, {:>16});",
            lit(m[(r, 0)]),
            lit(m[(r, 1)]),
            lit(m[(r, 2)])
        );
    }
    let _ = writeln!(out, "    return m;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
}

/// Sanitise text for a `//` comment line: one line, ASCII only (some GPU compilers
/// reject non-ASCII bytes even in comments).
fn c(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            'Δ' => out.push('d'),
            '′' | '’' | '‘' => out.push('\''),
            '“' | '”' => out.push('"'),
            '—' | '–' => out.push('-'),
            '→' => out.push_str("->"),
            '²' => out.push('2'),
            '³' => out.push('3'),
            '°' => out.push_str(" deg"),
            '×' => out.push('x'),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

/// Generate the DCTL source.
pub fn generate_dctl(m_xyz: &Mat3, meta: &ExportMeta) -> String {
    let dwg_to_xyz = davinci::dwg_to_xyz();
    let xyz_to_dwg = davinci::xyz_to_dwg();
    let mut s = String::new();
    let _ = writeln!(s, "// ============================================================================");
    let _ = writeln!(s, "// LUT Matrix - 3x3 colour matrix in linear CIE XYZ, extracted from a LUT");
    let _ = writeln!(s, "// ----------------------------------------------------------------------------");
    let _ = writeln!(s, "// Source LUT:    {}", c(&meta.source_lut));
    let _ = writeln!(s, "// LUT input:     {}", c(&meta.input_space));
    let _ = writeln!(s, "// LUT output:    {}", c(&meta.output_space));
    let _ = writeln!(s, "// White mode:    {}", c(&meta.white_mode));
    let _ = writeln!(s, "// Middle gray:   {}", c(&meta.middle_gray));
    let _ = writeln!(s, "// Fit:           {}", c(&meta.fit_method));
    let _ = writeln!(s, "// Fit error:     {}", c(&meta.fit_error));
    if let Some(r) = &meta.reference {
        let _ = writeln!(s, "// Fitted through: {}", c(r));
        let _ = writeln!(s, "//                USE THIS DCTL DIRECTLY BEFORE THAT TRANSFORM.");
    }
    let _ = writeln!(s, "// Generated by:  LUT Matrix {} on {} (UTC)", c(&meta.version), c(&meta.date));
    let _ = writeln!(s, "// ----------------------------------------------------------------------------");
    let _ = writeln!(s, "// Input and output: DaVinci Wide Gamut / DaVinci Intermediate.");
    let _ = writeln!(s, "// Pipeline: DI -> linear DWG -> XYZ (DWG_TO_XYZ) -> M_XYZ (blended by");
    let _ = writeln!(s, "// 'Matrix Amount') -> linear DWG (XYZ_TO_DWG) -> DI.");
    if meta.reference.is_none() {
        let _ = writeln!(s, "// Place it where the timeline is DWG/DI, before the output transform.");
    }
    let _ = writeln!(s, "//");
    let _ = writeln!(s, "// DWG primaries R(0.8000, 0.3130) G(0.1682, 0.9877) B(0.0790, -0.1155), white");
    let _ = writeln!(s, "// D65 (0.3127, 0.3290), and the DaVinci Intermediate constants are from");
    let _ = writeln!(s, "// Blackmagic Design, \"DaVinci Wide Gamut Intermediate\" v1.1 (2021-07-31).");
    let _ = writeln!(s, "// DWG<->XYZ matrices are derived from those primaries in double precision.");
    let _ = writeln!(s, "// ============================================================================");
    let _ = writeln!(s);
    let _ = writeln!(s, "DEFINE_UI_PARAMS(p_Amount, Matrix Amount, DCTLUI_SLIDER_FLOAT, 1.0, 0.0, 1.0, 0.001)");
    let _ = writeln!(s);
    let _ = writeln!(s, "typedef struct");
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "    float3 r0, r1, r2; // rows");
    let _ = writeln!(s, "}} Mat3x3;");
    let _ = writeln!(s);
    let _ = writeln!(s, "__DEVICE__ float3 mat_mul(Mat3x3 m, float3 v)");
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "    return make_float3(m.r0.x * v.x + m.r0.y * v.y + m.r0.z * v.z,");
    let _ = writeln!(s, "                       m.r1.x * v.x + m.r1.y * v.y + m.r1.z * v.z,");
    let _ = writeln!(s, "                       m.r2.x * v.x + m.r2.y * v.y + m.r2.z * v.z);");
    let _ = writeln!(s, "}}");
    let _ = writeln!(s);
    matrix_fn(&mut s, "DWG_TO_XYZ", "Linear DaVinci Wide Gamut -> CIE XYZ (D65).", &dwg_to_xyz);
    matrix_fn(
        &mut s,
        "M_XYZ",
        "Fitted colour matrix in linear CIE XYZ (D65): XYZ in -> XYZ out.",
        m_xyz,
    );
    matrix_fn(&mut s, "XYZ_TO_DWG", "CIE XYZ (D65) -> linear DaVinci Wide Gamut.", &xyz_to_dwg);

    let _ = writeln!(s, "// DaVinci Intermediate (Blackmagic, DWG Intermediate v1.1).");
    let _ = writeln!(s, "#define DI_A        {}", lit(DI_A));
    let _ = writeln!(s, "#define DI_B        {}", lit(DI_B));
    let _ = writeln!(s, "#define DI_C        {}", lit(DI_C));
    let _ = writeln!(s, "#define DI_M        {}", lit(DI_M));
    let _ = writeln!(s, "#define DI_LIN_CUT  {}", lit(DI_LIN_CUT));
    let _ = writeln!(s, "#define DI_LOG_CUT  {}", lit(DI_LOG_CUT));
    let _ = writeln!(s);
    let _ = writeln!(s, "__DEVICE__ float di_to_linear(float v)");
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "    if (v <= DI_LOG_CUT) return v / DI_M;");
    let _ = writeln!(s, "    return _powf(2.0f, v / DI_C - DI_B) - DI_A;");
    let _ = writeln!(s, "}}");
    let _ = writeln!(s);
    let _ = writeln!(s, "__DEVICE__ float linear_to_di(float l)");
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "    if (l <= DI_LIN_CUT) return l * DI_M;");
    let _ = writeln!(s, "    return (_log2f(l + DI_A) + DI_B) * DI_C;");
    let _ = writeln!(s, "}}");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "__DEVICE__ float3 transform(int p_Width, int p_Height, int p_X, int p_Y, float p_R, float p_G, float p_B)"
    );
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "    // 1. DaVinci Intermediate -> linear DaVinci Wide Gamut");
    let _ = writeln!(s, "    float3 dwg = make_float3(di_to_linear(p_R), di_to_linear(p_G), di_to_linear(p_B));");
    let _ = writeln!(s);
    let _ = writeln!(s, "    // 2. Linear DWG -> CIE XYZ");
    let _ = writeln!(s, "    float3 xyz = mat_mul(DWG_TO_XYZ(), dwg);");
    let _ = writeln!(s);
    let _ = writeln!(s, "    // 3. Fitted matrix in XYZ, blended with identity (p_Amount = 0 is a no-op)");
    let _ = writeln!(s, "    float3 fit = mat_mul(M_XYZ(), xyz);");
    let _ = writeln!(s, "    xyz = make_float3(xyz.x + (fit.x - xyz.x) * p_Amount,");
    let _ = writeln!(s, "                      xyz.y + (fit.y - xyz.y) * p_Amount,");
    let _ = writeln!(s, "                      xyz.z + (fit.z - xyz.z) * p_Amount);");
    let _ = writeln!(s);
    let _ = writeln!(s, "    // 4. CIE XYZ -> linear DWG");
    let _ = writeln!(s, "    dwg = mat_mul(XYZ_TO_DWG(), xyz);");
    let _ = writeln!(s);
    let _ = writeln!(s, "    // 5. Linear DWG -> DaVinci Intermediate");
    let _ = writeln!(s, "    return make_float3(linear_to_di(dwg.x), linear_to_di(dwg.y), linear_to_di(dwg.z));");
    let _ = writeln!(s, "}}");
    s
}

/// Machine-readable export of all matrices and metadata.
#[derive(Debug, Clone, Serialize)]
pub struct MatrixExport {
    pub meta: ExportMeta,
    /// Row-major 3×3, XYZ (D65) in → XYZ (D65) out.
    pub m_xyz: [[f64; 3]; 3],
    pub m_xyz_least_squares: [[f64; 3]; 3],
    pub dwg_to_xyz: [[f64; 3]; 3],
    pub xyz_to_dwg: [[f64; 3]; 3],
    /// The same transform expressed in linear DWG (XYZ_TO_DWG · M_XYZ · DWG_TO_XYZ),
    /// for tools that take a single RGB matrix. The DCTL keeps the three separate.
    pub m_dwg_linear: [[f64; 3]; 3],
}

fn rows(m: &Mat3) -> [[f64; 3]; 3] {
    [
        [m[(0, 0)], m[(0, 1)], m[(0, 2)]],
        [m[(1, 0)], m[(1, 1)], m[(1, 2)]],
        [m[(2, 0)], m[(2, 1)], m[(2, 2)]],
    ]
}

pub fn matrix_export(a: &Analysis, meta: &ExportMeta) -> MatrixExport {
    let dwg = davinci::dwg_to_xyz();
    let inv = davinci::xyz_to_dwg();
    MatrixExport {
        meta: meta.clone(),
        m_xyz: rows(&a.m_xyz),
        m_xyz_least_squares: rows(&a.m_xyz_ls),
        dwg_to_xyz: rows(&dwg),
        xyz_to_dwg: rows(&inv),
        m_dwg_linear: rows(&(inv * a.m_xyz * dwg)),
    }
}

pub fn export_json(a: &Analysis, meta: &ExportMeta) -> String {
    serde_json::to_string_pretty(&matrix_export(a, meta)).expect("serialisable")
}

/// Plain-text export (tab-separated rows, 10 decimals).
pub fn export_text(a: &Analysis, meta: &ExportMeta) -> String {
    let e = matrix_export(a, meta);
    let mut s = String::new();
    let _ = writeln!(s, "# LUT Matrix {} — {}", meta.version, meta.date);
    let _ = writeln!(s, "# Source LUT: {}", c(&meta.source_lut));
    let _ = writeln!(s, "# Input: {} | Output: {}", c(&meta.input_space), c(&meta.output_space));
    let _ = writeln!(s, "# White mode: {}", c(&meta.white_mode));
    let _ = writeln!(s, "# Middle gray: {}", c(&meta.middle_gray));
    let _ = writeln!(s, "# Fit error: {}", c(&meta.fit_error));
    let block = |s: &mut String, name: &str, m: &[[f64; 3]; 3]| {
        let _ = writeln!(s, "\n{name}");
        for r in m {
            let _ = writeln!(s, "{:.10}\t{:.10}\t{:.10}", r[0], r[1], r[2]);
        }
    };
    block(&mut s, "M_XYZ (linear CIE XYZ D65 -> XYZ D65)", &e.m_xyz);
    block(&mut s, "DWG_TO_XYZ", &e.dwg_to_xyz);
    block(&mut s, "XYZ_TO_DWG", &e.xyz_to_dwg);
    block(&mut s, "M_XYZ_LEAST_SQUARES", &e.m_xyz_least_squares);
    block(&mut s, "M_DWG_LINEAR (= XYZ_TO_DWG * M_XYZ * DWG_TO_XYZ)", &e.m_dwg_linear);
    s
}

/// Suggested DCTL file name for a LUT file name: "<stem>_MXYZ.dctl", restricted to
/// characters that are safe on Windows and macOS.
pub fn dctl_file_name(lut_name: &str) -> String {
    let stem = lut_name.rsplit_once('.').map(|(a, _)| a).unwrap_or(lut_name);
    let safe: String = stem
        .chars()
        .map(|ch| if ch.is_alphanumeric() || "-_.".contains(ch) { ch } else { '_' })
        .collect();
    format!("{}_MXYZ.dctl", if safe.is_empty() { "LUT".into() } else { safe })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_742), (2026, 10, 16));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn literals() {
        assert_eq!(lit(-0.0), "0.0000000000f");
        assert_eq!(lit(0.70062239209), "0.7006223921f");
        assert_eq!(lit(-1.5), "-1.5000000000f");
    }

    #[test]
    fn comments_are_ascii() {
        assert_eq!(c("ΔE2000 — Ärger → 709\nx"), "dE2000 - ?rger -> 709 x");
    }

    #[test]
    fn file_names() {
        assert_eq!(dctl_file_name("ARRI LogC4 → 709.cube"), "ARRI_LogC4___709_MXYZ.dctl");
        assert_eq!(dctl_file_name("a.b.Cube"), "a.b_MXYZ.dctl");
    }
}
