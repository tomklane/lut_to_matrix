//! DCTL output: float32 pipeline emulation, golden file, and a C/C++ compile-and-run
//! check of the generated source.

use std::path::{Path, PathBuf};
use std::process::Command;

use lutmatrix_core::dctl::{generate_dctl, ExportMeta};
use lutmatrix_core::math::{whites, Mat3, Vec3};
use lutmatrix_core::sample::{hue_ring, Settings};
use lutmatrix_core::spaces::davinci::{di_decode, di_encode, dwg_to_xyz, xyz_to_dwg};
use lutmatrix_core::spaces::{CameraSpaceId, DisplayGamut, DisplayTransfer, InputSpace, OutputSpace};

fn look() -> Mat3 {
    let m = Mat3::new(1.08, -0.06, 0.02, -0.03, 1.04, -0.01, 0.01, -0.07, 1.10);
    m / (m * whites::D65.to_xyz()).y
}

fn meta() -> ExportMeta {
    ExportMeta {
        source_lut: "Golden Test LUT.cube".into(),
        input_space: "ARRI LogC4 / AWG4".into(),
        output_space: "Rec.709 / Gamma 2.4".into(),
        white_mode: "D65 (Bradford CAT from LUT white + exact D65 neutral constraint)".into(),
        middle_gray: "scene 0.1800, display 12.81 cd/m2".into(),
        fit_error: "ΔE2000: mean 1.000, p95 2.000, max 3.000".into(),
        fit_method: "least squares, refined by minimising ΔE2000".into(),
        reference: None,
        version: "{VERSION}".into(),
        date: "2026-01-01".into(),
    }
}

// ---------------------------------------------------------------------------
// Parse the generated DCTL so the emulation uses exactly the numbers written.
// ---------------------------------------------------------------------------
struct Parsed {
    dwg_to_xyz: [[f32; 3]; 3],
    m_xyz: [[f32; 3]; 3],
    xyz_to_dwg: [[f32; 3]; 3],
    di: [f32; 6], // A, B, C, M, LIN_CUT, LOG_CUT
}

fn num(s: &str) -> f32 {
    s.trim().trim_end_matches('f').parse().unwrap_or_else(|_| panic!("bad literal '{s}'"))
}

fn parse_matrix(src: &str, name: &str) -> [[f32; 3]; 3] {
    let start = src.find(&format!("__DEVICE__ Mat3x3 {name}()")).unwrap_or_else(|| panic!("{name} missing"));
    let mut rows = [[0f32; 3]; 3];
    for (r, line) in src[start..].lines().filter(|l| l.contains("make_float3(")).take(3).enumerate() {
        let inner = &line[line.find('(').unwrap() + 1..line.rfind(')').unwrap()];
        let v: Vec<f32> = inner.split(',').map(num).collect();
        rows[r] = [v[0], v[1], v[2]];
    }
    rows
}

fn parse(src: &str) -> Parsed {
    let def = |k: &str| {
        let l = src.lines().find(|l| l.starts_with(&format!("#define {k} "))).unwrap();
        num(l.split_whitespace().nth(2).unwrap())
    };
    Parsed {
        dwg_to_xyz: parse_matrix(src, "DWG_TO_XYZ"),
        m_xyz: parse_matrix(src, "M_XYZ"),
        xyz_to_dwg: parse_matrix(src, "XYZ_TO_DWG"),
        di: [def("DI_A"), def("DI_B"), def("DI_C"), def("DI_M"), def("DI_LIN_CUT"), def("DI_LOG_CUT")],
    }
}

/// The DCTL's `transform`, step for step, in float32.
fn dctl_f32(p: &Parsed, rgb: [f32; 3], amount: f32) -> [f32; 3] {
    let [a, b, c, m, lin_cut, log_cut] = p.di;
    let to_lin = |v: f32| if v <= log_cut { v / m } else { 2f32.powf(v / c - b) - a };
    let to_di = |l: f32| if l <= lin_cut { l * m } else { ((l + a).log2() + b) * c };
    let mul = |mm: &[[f32; 3]; 3], v: [f32; 3]| {
        [
            mm[0][0] * v[0] + mm[0][1] * v[1] + mm[0][2] * v[2],
            mm[1][0] * v[0] + mm[1][1] * v[1] + mm[1][2] * v[2],
            mm[2][0] * v[0] + mm[2][1] * v[1] + mm[2][2] * v[2],
        ]
    };
    let dwg = [to_lin(rgb[0]), to_lin(rgb[1]), to_lin(rgb[2])];
    let xyz = mul(&p.dwg_to_xyz, dwg);
    let fit = mul(&p.m_xyz, xyz);
    let xyz = [
        xyz[0] + (fit[0] - xyz[0]) * amount,
        xyz[1] + (fit[1] - xyz[1]) * amount,
        xyz[2] + (fit[2] - xyz[2]) * amount,
    ];
    let dwg = mul(&p.xyz_to_dwg, xyz);
    [to_di(dwg[0]), to_di(dwg[1]), to_di(dwg[2])]
}

/// The same pipeline in f64 with the app's own (unrounded) values.
fn reference_f64(m_xyz: &Mat3, code: &Vec3, amount: f64) -> Vec3 {
    let dwg = code.map(di_decode);
    let xyz = dwg_to_xyz() * dwg;
    let xyz = xyz + (m_xyz * xyz - xyz) * amount;
    (xyz_to_dwg() * xyz).map(di_encode)
}

/// DaVinci Intermediate test codes: the app's sample ring at −2…+2 stops, grays, and
/// a few extremes (black, near-black negatives, bright highlights).
fn sample_codes() -> Vec<Vec3> {
    let input = InputSpace::new(CameraSpaceId::DaVinciIntermediateDwg);
    let out = OutputSpace::new(DisplayGamut::Rec709, DisplayTransfer::Gamma { gamma: 2.4 });
    let mut codes = Vec::new();
    for stops in [-2.0, -1.0, 0.0, 1.0, 2.0] {
        let g = 2f64.powf(stops);
        for (_, xyz) in hue_ring(&Settings::default(), &input, &out, 0.18) {
            codes.push((xyz_to_dwg() * xyz * g).map(di_encode));
        }
        codes.push((xyz_to_dwg() * whites::D65.to_xyz() * 0.18 * g).map(di_encode));
    }
    for v in [[0.0, 0.0, 0.0], [0.01, 0.02, 0.0], [0.9, 0.8, 0.7], [1.0, 1.0, 1.0], [0.6, 0.2, 0.9]] {
        codes.push(Vec3::from(v));
    }
    codes
}

/// Float32 tolerance on DaVinci Intermediate code values (≈ 0.005 of a 10-bit code),
/// relative for |code| > 1 (extreme out-of-gamut test colours reach code −9.5).
const TOL_F32: f64 = 5e-6;

fn close(got: f64, want: f64) -> bool {
    (got - want).abs() < TOL_F32 * want.abs().max(1.0)
}

#[test]
fn float32_dctl_pipeline_matches_double_precision() {
    let m = look();
    let src = generate_dctl(&m, &meta());
    let p = parse(&src);
    let mut worst = 0.0f64;
    for amount in [1.0f32, 0.5, 0.0] {
        for code in sample_codes() {
            let got = dctl_f32(&p, [code.x as f32, code.y as f32, code.z as f32], amount);
            let want = reference_f64(&m, &code, amount as f64);
            for k in 0..3 {
                let d = (got[k] as f64 - want[k]).abs() / want[k].abs().max(1.0);
                worst = worst.max(d);
                assert!(close(got[k] as f64, want[k]), "amount {amount} code {code:?}: f32 {got:?} vs f64 {want:?}");
            }
        }
    }
    eprintln!("float32 vs float64 worst DI code difference: {worst:.2e}");
}

#[test]
fn amount_zero_is_identity() {
    let p = parse(&generate_dctl(&look(), &meta()));
    for code in sample_codes() {
        let got = dctl_f32(&p, [code.x as f32, code.y as f32, code.z as f32], 0.0);
        for k in 0..3 {
            assert!(close(got[k] as f64, code[k]));
        }
    }
}

#[test]
fn dctl_contains_required_structure() {
    let src = generate_dctl(&look(), &meta());
    assert!(src.is_ascii(), "DCTL must be ASCII");
    for needle in [
        "__DEVICE__ float3 transform(int p_Width, int p_Height, int p_X, int p_Y, float p_R, float p_G, float p_B)",
        "__DEVICE__ Mat3x3 DWG_TO_XYZ()",
        "__DEVICE__ Mat3x3 M_XYZ()",
        "__DEVICE__ Mat3x3 XYZ_TO_DWG()",
        "DEFINE_UI_PARAMS(p_Amount, Matrix Amount, DCTLUI_SLIDER_FLOAT, 1.0, 0.0, 1.0, 0.001)",
        "// Source LUT:    Golden Test LUT.cube",
        "// White mode:",
        "// Middle gray:",
        "// Fit error:     dE2000",
        "LUT Matrix {VERSION} on 2026-01-01",
    ] {
        assert!(src.contains(needle), "missing: {needle}");
    }
    // The steps appear in the brief's order.
    let pos = |s: &str| src.find(s).unwrap_or_else(|| panic!("{s}"));
    let order = [
        "di_to_linear(p_R)",
        "mat_mul(DWG_TO_XYZ(), dwg)",
        "mat_mul(M_XYZ(), xyz)",
        "mat_mul(XYZ_TO_DWG(), xyz)",
        "linear_to_di(dwg.x)",
    ];
    assert!(order.windows(2).all(|w| pos(w[0]) < pos(w[1])));
    // Every float literal carries the f suffix.
    for line in src.lines().filter(|l| l.trim_start().starts_with("m.r") && l.contains("= make_float3(")) {
        assert_eq!(line.matches("f,").count() + line.matches("f)").count(), 3, "{line}");
    }
}

// ---------------------------------------------------------------------------
// Golden file
// ---------------------------------------------------------------------------
fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/golden_MXYZ.dctl")
}

/// Regenerate with: UPDATE_GOLDEN=1 cargo test -p lutmatrix-core --test dctl
#[test]
fn golden_dctl() {
    let src = generate_dctl(&look(), &meta());
    let path = golden_path();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &src).unwrap();
    }
    let want = std::fs::read_to_string(&path).expect("golden file missing; run with UPDATE_GOLDEN=1");
    let want = want.replace("\r\n", "\n"); // tolerate CRLF checkouts on Windows
    if src != want {
        let first = src.lines().zip(want.lines()).position(|(a, b)| a != b);
        panic!("generated DCTL differs from golden file at line {first:?}; rerun with UPDATE_GOLDEN=1 if intended");
    }
}

// ---------------------------------------------------------------------------
// Compile and run the DCTL as C99 and as C++ (proxies for OpenCL and CUDA/Metal),
// with a tiny shim for the DCTL built-ins. Skipped when no C compiler is present.
// ---------------------------------------------------------------------------
const SHIM: &str = r#"
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
typedef struct { float x, y, z; } float3;
static float3 make_float3(float x, float y, float z) { float3 r; r.x = x; r.y = y; r.z = z; return r; }
#define __DEVICE__ static
#define _powf powf
#define _log2f log2f
#define DEFINE_UI_PARAMS(name, label, kind, def, lo, hi, step) static float name = def;
"#;

const MAIN: &str = r#"
int main(void) {
    float r, g, b, amount;
    while (scanf("%f %f %f %f", &r, &g, &b, &amount) == 4) {
        p_Amount = amount;
        float3 o = transform(1, 1, 0, 0, r, g, b);
        printf("%.9g %.9g %.9g\n", o.x, o.y, o.z);
    }
    return 0;
}
"#;

fn have(cmd: &str) -> bool {
    Command::new(cmd).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn dctl_compiles_and_runs_as_c_and_cpp() {
    let m = look();
    let src = generate_dctl(&m, &meta());
    let dir = std::env::temp_dir().join(format!("lutmatrix-dctl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.dctl"), &src).unwrap();
    let prog = format!("{SHIM}\n#include \"test.dctl\"\n{MAIN}");
    std::fs::write(dir.join("main.c"), &prog).unwrap();

    let mut ran = false;
    for (compiler, args) in [
        ("cc", vec!["-std=c99", "-Wall", "-Werror", "-Wno-unused-parameter", "-Wno-unused-function"]),
        ("c++", vec!["-x", "c++", "-std=c++14", "-Wall", "-Werror", "-Wno-unused-parameter", "-Wno-unused-function"]),
    ] {
        if !have(compiler) {
            eprintln!("{compiler} not found; skipping");
            continue;
        }
        let exe = dir.join(format!("dctl_{}", compiler.replace('+', "p")));
        let out = Command::new(compiler)
            .args(&args)
            .arg("main.c")
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "{compiler} failed:\n{}", String::from_utf8_lossy(&out.stderr));

        let codes = sample_codes();
        let mut input = String::new();
        for amount in [1.0, 0.5] {
            for c in &codes {
                input.push_str(&format!("{:.9} {:.9} {:.9} {amount}\n", c.x, c.y, c.z));
            }
        }
        let mut child = Command::new(&exe)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), codes.len() * 2);
        for (i, line) in lines.iter().enumerate() {
            let amount = if i < codes.len() { 1.0 } else { 0.5 };
            let c = codes[i % codes.len()];
            // Compare against f64 using the f32-rounded input the program actually read.
            let c32 = c.map(|v| v as f32 as f64);
            let want = reference_f64(&m, &c32, amount);
            let got: Vec<f64> = line.split_whitespace().map(|v| v.parse().unwrap()).collect();
            for k in 0..3 {
                assert!(close(got[k], want[k]), "{compiler}: {c:?} → {got:?} vs {want:?}");
            }
        }
        ran = true;
    }
    if !ran {
        eprintln!("no C/C++ compiler available; DCTL compile check skipped");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_from_a_real_analysis() {
    use lutmatrix_core::analysis::analyze_transform;
    use lutmatrix_core::dctl::{export_json, export_text, ExportMeta};
    use lutmatrix_core::lut::FnTransform;
    let input = InputSpace::new(CameraSpaceId::ArriLogC4Awg4);
    let out = OutputSpace::new(DisplayGamut::Rec709, DisplayTransfer::Gamma { gamma: 2.4 });
    let m_true = look();
    let f = move |code: &Vec3| {
        let xyz = m_true * (input.rgb_to_xyz_d65() * input.decode(code)) * (11.0 / 0.18);
        out.encode(&(out.xyz_to_rgb() * xyz)).map(|v| v.clamp(0.0, 1.0))
    };
    let a = analyze_transform(&FnTransform(f), input, out, &Settings::default(), None).unwrap();
    let meta = ExportMeta::from_analysis(&a, "Some LUT.cube", "2026-10-16");
    assert_eq!(meta.version, lutmatrix_core::VERSION);
    let src = generate_dctl(&a.m_xyz, &meta);
    assert!(src.contains(&format!("LUT Matrix {} on 2026-10-16", lutmatrix_core::VERSION)));
    assert!(src.contains("// LUT input:     ARRI LogC4 / AWG4"));
    assert!(src.contains("// White mode:    D65"));
    assert!(src.is_ascii());

    let j: serde_json::Value = serde_json::from_str(&export_json(&a, &meta)).unwrap();
    let get = |k: &str| -> Mat3 {
        let r: Vec<Vec<f64>> = serde_json::from_value(j[k].clone()).unwrap();
        Mat3::from_fn(|i, k| r[i][k])
    };
    assert!((get("m_xyz") - a.m_xyz).abs().max() < 1e-15);
    assert!((get("dwg_to_xyz") - dwg_to_xyz()).abs().max() < 1e-15);
    let combined = xyz_to_dwg() * a.m_xyz * dwg_to_xyz();
    assert!((get("m_dwg_linear") - combined).abs().max() < 1e-14);
    assert_eq!(j["meta"]["version"], lutmatrix_core::VERSION);

    let txt = export_text(&a, &meta);
    assert!(txt.contains("M_XYZ (linear CIE XYZ D65 -> XYZ D65)") && txt.contains("XYZ_TO_DWG"));
}
