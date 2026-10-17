//! Parity of the Rust port with the original OpenDRT DCTL.
//!
//! - `matches_compiled_dctl`: compiles the bundled DCTL as C++ (with `tests/shim/dctl_shim.h`
//!   standing in for Resolve's DCTL runtime) in both preset mode and stickshift mode
//!   (`INIT_PARAMS`), and compares the port on every look × tonescale preset, every
//!   creative white, display encoding, input gamut and transfer function, the HDR sliders,
//!   and random stickshift parameter sets. Skipped (with a message) when no C++ compiler
//!   is found (`CXX`, else `c++`).
//! - `matches_reference_fixture`: the same comparison against committed outputs of the
//!   compiled DCTL, so platforms without a compiler (Windows CI) are checked too.
//!   Regenerate with `LUTMATRIX_REGEN_OPENDRT_FIXTURE=1 cargo test -p lutmatrix-opendrt`.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lutmatrix_opendrt::{
    Config, DisplayGamut, Eotf, InputGamut, InputOetf, LookParams, OpenDrt, Presets, DCTL_SOURCE,
};
use regex::Regex;
use serde::{Deserialize, Serialize};

const FIXTURE: &str = "tests/fixtures/opendrt_reference.json";

/// One configuration, as the C++ harness parameters and as the Rust [`Config`].
struct Case {
    name: String,
    stickshift: bool,
    cpp: Vec<(String, f64)>,
    cfg: Config,
}

fn input_set(cfg: &Config) -> Vec<[f32; 3]> {
    let mut v = Vec::new();
    let n = 7;
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                let c = |i: usize| -0.03 + 1.1 * i as f32 / (n - 1) as f32;
                v.push([c(r), c(g), c(b)]);
            }
        }
    }
    for i in 0..=40 {
        let x = -0.05 + 1.15 * i as f32 / 40.0;
        v.push([x, x, x]);
    }
    if cfg.in_oetf == InputOetf::Linear {
        for p in &mut v {
            for c in p.iter_mut() {
                *c *= 12.0;
            }
        }
    }
    v
}

fn preset_case(name: String, p: Presets, edit: impl Fn(&mut Config)) -> Case {
    let mut cfg = Config::from_presets(&p).unwrap();
    edit(&mut cfg);
    let cpp = vec![
        ("look_preset".into(), p.look as f64),
        ("tonescale_preset".into(), p.tonescale as f64),
        ("_cwp".into(), p.creative_white as f64),
        ("_cwp_lm".into(), p.creative_white_limit as f64),
        ("display_encoding_preset".into(), p.display_encoding as f64),
        ("in_gamut".into(), cfg.in_gamut.index() as f64),
        ("in_oetf".into(), cfg.in_oetf.index() as f64),
        ("tn_Lp".into(), cfg.peak_nits as f64),
        ("tn_gb".into(), cfg.hdr_grey_boost as f64),
        ("pt_hdr".into(), cfg.hdr_purity as f64),
        ("tn_Lg".into(), cfg.grey_nits as f64),
    ];
    Case { name, stickshift: false, cpp, cfg }
}

fn stickshift_case(name: String, cfg: Config) -> Case {
    let mut cpp: Vec<(String, f64)> = LookParams::info()
        .iter()
        .zip(cfg.look.to_array())
        .map(|(i, v)| (format!("_{}", i.name), v as f64))
        .collect();
    cpp.extend([
        ("_tn_su".into(), cfg.surround as f64),
        ("_display_gamut".into(), cfg.display_gamut.index() as f64),
        ("_eotf".into(), cfg.eotf.index() as f64),
        ("_clamp".into(), cfg.clamp as u8 as f64),
        ("in_gamut".into(), cfg.in_gamut.index() as f64),
        ("in_oetf".into(), cfg.in_oetf.index() as f64),
        ("tn_Lp".into(), cfg.peak_nits as f64),
        ("tn_gb".into(), cfg.hdr_grey_boost as f64),
        ("pt_hdr".into(), cfg.hdr_purity as f64),
        ("tn_Lg".into(), cfg.grey_nits as f64),
    ]);
    Case { name, stickshift: true, cpp, cfg }
}

/// Deterministic xorshift for the random stickshift sets.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn index(&mut self, n: usize) -> usize {
        ((self.next() * n as f32) as usize).min(n - 1)
    }
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    let none = |_: &mut Config| {};
    for look in 0..9 {
        for tone in 0..14 {
            let p = Presets { look, tonescale: tone, ..Presets::default() };
            out.push(preset_case(format!("look {look} tonescale {tone}"), p, none));
        }
    }
    for enc in 0..9 {
        for look in [0, 4, 5, 6] {
            for peak in [100.0, 1000.0] {
                let p = Presets { look, display_encoding: enc, ..Presets::default() };
                out.push(preset_case(format!("encoding {enc} look {look} peak {peak}"), p, |c| c.peak_nits = peak));
            }
        }
    }
    for cw in 0..7 {
        for lm in [0.25, 0.8] {
            for enc in [0, 2, 3, 4, 5] {
                let p = Presets { creative_white: cw, creative_white_limit: lm, display_encoding: enc, look: 6, ..Presets::default() };
                out.push(preset_case(format!("creative white {cw} limit {lm} encoding {enc}"), p, none));
            }
        }
    }
    for g in InputGamut::ALL {
        for tf in [InputOetf::Linear, InputOetf::DaVinciIntermediate] {
            out.push(preset_case(format!("input {g:?} {tf:?}"), Presets::default(), |c| {
                c.in_gamut = *g;
                c.in_oetf = tf;
            }));
        }
    }
    for tf in InputOetf::ALL {
        out.push(preset_case(format!("transfer {tf:?}"), Presets { look: 1, ..Presets::default() }, |c| {
            c.in_gamut = InputGamut::ArriWideGamut3;
            c.in_oetf = *tf;
        }));
    }
    for (lg, lp, gb, pt) in [(3.0, 100.0, 0.13, 0.5), (6.5, 100.0, 0.13, 0.5), (18.0, 100.0, 0.13, 0.5), (25.0, 100.0, 0.0, 0.0),
        (10.0, 400.0, 0.13, 0.5), (10.0, 1000.0, 0.3, 1.0), (14.0, 600.0, 0.0, 0.0)]
    {
        for enc in [0, 6, 8] {
            let p = Presets { look: 3, display_encoding: enc, ..Presets::default() };
            out.push(preset_case(format!("grey {lg} peak {lp} boost {gb} purity {pt} encoding {enc}"), p, |c| {
                c.grey_nits = lg;
                c.peak_nits = lp;
                c.hdr_grey_boost = gb;
                c.hdr_purity = pt;
            }));
        }
    }
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..150 {
        let mut cfg = Config::from_presets(&Presets { look: rng.index(9), ..Presets::default() }).unwrap();
        let mut a = cfg.look.to_array();
        for (k, info) in LookParams::info().iter().enumerate() {
            if rng.next() < 0.6 {
                let v = info.min + rng.next() * (info.max - info.min);
                a[k] = if info.kind == lutmatrix_opendrt::Kind::Float { v } else { v.round() };
            }
        }
        cfg.look = LookParams::from_array(&a);
        cfg.surround = rng.index(3) as u8;
        cfg.display_gamut = DisplayGamut::from_index(rng.index(6)).unwrap();
        cfg.eotf = Eotf::from_index(rng.index(6)).unwrap();
        cfg.clamp = rng.next() < 0.7;
        cfg.in_gamut = InputGamut::from_index(rng.index(15)).unwrap();
        cfg.in_oetf = InputOetf::from_index(rng.index(10)).unwrap();
        cfg.peak_nits = if rng.next() < 0.5 { 100.0 } else { 100.0 + 900.0 * rng.next() };
        cfg.grey_nits = 3.0 + 22.0 * rng.next();
        cfg.hdr_grey_boost = rng.next();
        cfg.hdr_purity = rng.next();
        out.push(stickshift_case(format!("stickshift #{i}"), cfg));
    }
    out
}

/* ---- C++ harness ---- */

/// The DCTL with every UI parameter turned into a settable static (as `make_harness.py`).
fn harness_source(dctl: &str) -> (String, Vec<(String, bool)>) {
    let re = Regex::new(r"^(//\s*)?DEFINE_UI_PARAMS\((\w+),\s*([^,]*),\s*(\w+),\s*([^,]+?)(?:,\s*(.*))?\)\s*$").unwrap();
    let mut params: Vec<(String, bool)> = Vec::new(); // (name, is_float)
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in dctl.lines() {
        let st = line.trim();
        if st == "// #define INIT_PARAMS" {
            continue; // set with -DINIT_PARAMS for the stickshift build
        }
        if let Some(c) = re.captures(st) {
            let commented = c.get(1).is_some();
            let name = c[2].to_string();
            if (commented && !name.starts_with('_')) || !seen.insert(name.clone()) {
                continue;
            }
            let (kind, default) = (&c[4], &c[5]);
            match kind {
                "DCTLUI_COMBO_BOX" => {
                    let rest = c.get(6).unwrap().as_str();
                    let ids = &rest[rest.find('{').unwrap() + 1..rest.find('}').unwrap()];
                    out.push(format!("enum {{{ids}}}; static int {name} = {default};"));
                    params.push((name, false));
                }
                "DCTLUI_CHECK_BOX" => {
                    out.push(format!("static int {name} = {default};"));
                    params.push((name, false));
                }
                _ => {
                    out.push(format!("static float {name} = {default}f;"));
                    params.push((name, true));
                }
            }
            continue;
        }
        out.push(line.to_string());
    }
    (out.join("\n"), params)
}

fn cxx() -> Option<String> {
    let cxx = std::env::var("CXX").unwrap_or_else(|_| "c++".into());
    let ok = Command::new(&cxx).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    ok.then_some(cxx)
}

fn build_harness(cxx: &str, dir: &Path, dctl: &str) -> (PathBuf, PathBuf) {
    let (src, params) = harness_source(dctl);
    std::fs::write(dir.join("opendrt_harness.dctl"), src).unwrap();
    let table: Vec<String> = params
        .iter()
        .map(|(n, f)| if *f { format!("{{\"{n}\", &{n}, 0}}") } else { format!("{{\"{n}\", 0, &{n}}}") })
        .collect();
    let main = format!(
        r#"#include "dctl_shim.h"
#include "opendrt_harness.dctl"
#include <string.h>
struct P {{ const char* n; float* f; int* i; }};
static P table[] = {{ {} }};
int main() {{
  char cmd[8], nm[128]; double v; float r, g, b;
  while (scanf("%7s", cmd) == 1) {{
    if (cmd[0] == 'P') {{
      if (scanf("%127s %lf", nm, &v) != 2) return 3;
      bool ok = false;
      for (auto& p : table) if (!strcmp(p.n, nm)) {{ if (p.f) *p.f = (float)v; else *p.i = (int)v; ok = true; }}
      if (!ok) {{ fprintf(stderr, "unknown param %s\n", nm); return 2; }}
    }} else {{
      if (scanf("%f %f %f", &r, &g, &b) != 3) return 3;
      float3 o = transform(1920, 1080, 10, 10, r, g, b);
      printf("%.9g %.9g %.9g\n", o.x, o.y, o.z);
    }}
  }}
  return 0;
}}
"#,
        table.join(",\n")
    );
    std::fs::write(dir.join("harness_main.cpp"), main).unwrap();
    let shim = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/shim");
    let mut exes = Vec::new();
    for (exe, def) in [("odrt_presets", None), ("odrt_stickshift", Some("-DINIT_PARAMS"))] {
        let path = dir.join(exe);
        let mut c = Command::new(cxx);
        c.args(["-std=c++14", "-O2", "-ffp-contract=off", "-w", "-I"]).arg(&shim).arg("-I").arg(dir);
        if let Some(d) = def {
            c.arg(d);
        }
        let st = c.arg(dir.join("harness_main.cpp")).arg("-o").arg(&path).status().expect("run C++ compiler");
        assert!(st.success(), "compiling the OpenDRT harness failed");
        exes.push(path);
    }
    (exes[0].clone(), exes[1].clone())
}

/// Run every case of one mode through one harness process.
fn run_harness(exe: &Path, cases: &[&Case], inputs: &[Vec<[f32; 3]>]) -> Vec<Vec<[f32; 3]>> {
    let mut script = String::new();
    for (case, inp) in cases.iter().zip(inputs) {
        for (k, v) in &case.cpp {
            script.push_str(&format!("P {k} {v:.9e}\n"));
        }
        for p in inp {
            script.push_str(&format!("C {:.9e} {:.9e} {:.9e}\n", p[0], p[1], p[2]));
        }
    }
    let mut child =
        Command::new(exe).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(script.as_bytes()));
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(out.status.success(), "harness failed: {}", String::from_utf8_lossy(&out.stderr));
    let mut vals = String::from_utf8(out.stdout).unwrap().lines().map(|l| {
        let v: Vec<f32> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
        [v[0], v[1], v[2]]
    }).collect::<Vec<_>>().into_iter();
    inputs.iter().map(|inp| inp.iter().map(|_| vals.next().expect("harness output")).collect()).collect()
}

fn close(a: f32, b: f32, tol: f32) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() <= tol * (1.0 + b.abs())
}

/// Compare the port with reference outputs; returns the largest relative difference.
fn compare(name: &str, cfg: &Config, inputs: &[[f32; 3]], want: &[[f32; 3]], tol: f32) -> f32 {
    let drt = OpenDrt::new(cfg);
    let mut worst = 0.0f32;
    for (i, w) in inputs.iter().zip(want) {
        let got = drt.apply(*i);
        for c in 0..3 {
            assert!(close(got[c], w[c], tol), "{name}: input {i:?} → port {got:?}, DCTL {w:?}");
            if !got[c].is_nan() {
                worst = worst.max((got[c] - w[c]).abs() / (1.0 + w[c].abs()));
            }
        }
    }
    worst
}

/// The DCTL written with two extra LUT Match presets: compiled, every stock look is
/// unchanged and the new ones match the port (with `grey_scale` = `lg_scale` only for
/// Tonescale Preset = USE LOOK PRESET).
#[test]
fn lut_match_dctl_matches_port() {
    use lutmatrix_opendrt::writer::{lut_match_dctl, LutMatchPreset};
    let Some(cxx) = cxx() else {
        eprintln!("SKIPPED: no C++ compiler (set CXX)");
        return;
    };
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut presets = Vec::new();
    for (name, grey) in [("LUT Match One", 6.3f32), ("LUT Match Two", 14.7)] {
        let mut a = LookParams::preset(rng.index(9)).unwrap().to_array();
        for (k, info) in LookParams::info().iter().enumerate() {
            let v = info.min + rng.next() * (info.max - info.min);
            a[k] = if info.kind == lutmatrix_opendrt::Kind::Float { v } else { v.round() };
        }
        presets.push(LutMatchPreset { name: name.into(), look: LookParams::from_array(&a), grey_nits: grey, source: "test".into() });
    }
    let dctl = lut_match_dctl(&presets, "test build").unwrap();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("opendrt_lutmatch");
    std::fs::create_dir_all(&dir).unwrap();
    let (exe, _) = build_harness(&cxx, &dir, &dctl);
    let mut cases = Vec::new();
    for look in 0..11 {
        for tonescale in [0, 4, 11] {
            for display_encoding in [0, 6] {
                for slider in [10.0f32, 7.0] {
                    let p = Presets { look, tonescale, display_encoding, ..Presets::default() };
                    let mut case = if look < 9 {
                        preset_case(String::new(), p, |c| c.grey_nits = slider)
                    } else {
                        let mut c = preset_case(String::new(), Presets { look: 0, ..p }, |_| {});
                        c.cfg = presets[look - 9].config(&p, slider).unwrap();
                        c.cpp[0].1 = look as f64;
                        c.cpp.retain(|(k, _)| k != "tn_Lg");
                        c.cpp.push(("tn_Lg".into(), slider as f64));
                        c
                    };
                    case.name = format!("look {look} tonescale {tonescale} encoding {display_encoding} slider {slider}");
                    cases.push(case);
                }
            }
        }
    }
    let sel: Vec<&Case> = cases.iter().collect();
    let inputs: Vec<Vec<[f32; 3]>> = sel.iter().map(|c| input_set(&c.cfg)).collect();
    let outs = run_harness(&exe, &sel, &inputs);
    let mut worst = 0.0f32;
    for ((case, inp), want) in sel.iter().zip(&inputs).zip(&outs) {
        worst = worst.max(compare(&case.name, &case.cfg, inp, want, 2e-5));
    }
    eprintln!("LUT Match DCTL vs port: {} cases, worst relative difference {worst:.2e}", cases.len());
}

#[derive(Serialize, Deserialize)]
struct FixtureCase {
    name: String,
    config: Config,
    inputs: Vec<[f32; 3]>,
    outputs: Vec<[f32; 3]>,
}

#[test]
fn matches_compiled_dctl() {
    let Some(cxx) = cxx() else {
        eprintln!("SKIPPED: no C++ compiler (set CXX); the committed fixture still checks the port");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("opendrt_parity");
    std::fs::create_dir_all(&dir).unwrap();
    let (presets_exe, stick_exe) = build_harness(&cxx, &dir, DCTL_SOURCE);
    let all = cases();
    let mut worst = 0.0f32;
    let mut fixture = Vec::new();
    for stick in [false, true] {
        let sel: Vec<&Case> = all.iter().filter(|c| c.stickshift == stick).collect();
        let inputs: Vec<Vec<[f32; 3]>> = sel.iter().map(|c| input_set(&c.cfg)).collect();
        let outs = run_harness(if stick { &stick_exe } else { &presets_exe }, &sel, &inputs);
        for (k, ((case, inp), want)) in sel.iter().zip(&inputs).zip(&outs).enumerate() {
            worst = worst.max(compare(&case.name, &case.cfg, inp, want, 2e-5));
            if k % 8 == 0 {
                // A slice of every eighth case for the committed fixture (no NaN: JSON has none).
                let rows: Vec<usize> =
                    (0..inp.len()).step_by(16).filter(|&r| want[r].iter().all(|v| v.is_finite())).collect();
                fixture.push(FixtureCase {
                    name: case.name.clone(),
                    config: case.cfg,
                    inputs: rows.iter().map(|&r| inp[r]).collect(),
                    outputs: rows.iter().map(|&r| want[r]).collect(),
                });
            }
        }
    }
    eprintln!("OpenDRT port vs compiled DCTL: {} cases, worst relative difference {worst:.2e}", all.len());
    if std::env::var_os("LUTMATRIX_REGEN_OPENDRT_FIXTURE").is_some() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        std::fs::write(&path, serde_json::to_string(&fixture).unwrap() + "\n").unwrap();
        eprintln!("wrote {}", path.display());
    }
}

#[test]
fn matches_reference_fixture() {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE)).unwrap();
    let cases: Vec<FixtureCase> = serde_json::from_str(&text).unwrap();
    assert!(cases.len() > 40);
    let mut worst = 0.0f32;
    for c in &cases {
        // Other platforms' libm differ from the generating one by a few ulp.
        worst = worst.max(compare(&c.name, &c.config, &c.inputs, &c.outputs, 1e-4));
    }
    eprintln!("OpenDRT port vs fixture: {} cases, worst relative difference {worst:.2e}", cases.len());
}

/// The preset tables in `src/tables.rs` are generated from the DCTL; check them again here.
#[test]
fn preset_tables_match_dctl() {
    let block = Regex::new(r"if \((look_preset|tonescale_preset)==(\d+)\) \{ // [^\n]*\n\s*(.*?);\n").unwrap();
    let mut n = 0;
    for c in block.captures_iter(DCTL_SOURCE) {
        let idx: usize = c[2].parse().unwrap();
        let mut look = LookParams::preset(0).unwrap();
        if &c[1] == "look_preset" {
            look = LookParams::preset(idx).unwrap();
        } else {
            lutmatrix_opendrt::ToneParams::preset(idx).unwrap().apply_to(&mut look);
        }
        for a in c[3].split(',') {
            let (k, v) = a.split_once('=').unwrap();
            let v: f32 = v.trim().trim_end_matches('f').parse().unwrap();
            assert_eq!(look.get(k.trim()), Some(v), "{} {idx} {k}", &c[1]);
            look.set(k.trim(), v);
        }
        n += 1;
    }
    assert_eq!(n, 9 + 13);
}
