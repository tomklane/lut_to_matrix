//! Chart matching on synthetic stills: a ColorChecker rendered with perspective into a
//! cluttered image, in a known camera "look", encoding and orientation.

use std::process::Command;

use lutmatrix_core::chart::detect::{find_lattice, homography, Orientation};
use lutmatrix_core::chart::encoding::{StillGamut, StillSpace};
use lutmatrix_core::chart::fit::{Exposure, Model, Tone};
use lutmatrix_core::chart::highlights::{ClipSource, HighlightSettings};
use lutmatrix_core::chart::image::ChartImage;
use lutmatrix_core::chart::odrt::{fit_opendrt_chart, OdrtChartSettings};
use lutmatrix_core::chart::reference::ChartEdition;
use lutmatrix_core::chart::{analyze, ChartSettings};
use lutmatrix_core::detect::Confidence;
use lutmatrix_core::math::{whites, Mat3, Vec3};
use lutmatrix_core::spaces::curves::LogCurve;
use lutmatrix_core::spaces::davinci::{di_decode, di_encode, dwg_to_xyz, xyz_to_dwg};

/// A camera look that keeps D65 neutral.
fn cam() -> Mat3 {
    let m = Mat3::new(1.10, -0.12, 0.05, -0.05, 1.04, 0.0, 0.03, -0.10, 1.12);
    let w = whites::D65.to_xyz();
    Mat3::from_diagonal(&w.component_div(&(m * w))) * m
}

struct Render {
    /// Where chart (0,0) / (5,0) / (5,3) / (0,3) land, pixels.
    corners: [[f64; 2]; 4],
    w: usize,
    h: usize,
}

/// Render a chart: `patch(i)` gives the stored code value of patch i, `card` the code
/// of the black card. Background: deterministic clutter of soft blobs and a gradient.
fn render(r: &Render, patch: &dyn Fn(usize) -> [f64; 3], card: [f64; 3], bg: &dyn Fn(f64) -> [f64; 3]) -> Vec<f32> {
    let src = [[0.0, 0.0], [5.0, 0.0], [5.0, 3.0], [0.0, 3.0]];
    let h = homography(&src, &r.corners).unwrap();
    let inv = h.try_inverse().unwrap();
    let mut data = Vec::with_capacity(r.w * r.h * 3);
    let mut seed = 12345u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let blobs: Vec<(f64, f64, f64, f64)> =
        (0..60).map(|_| (rnd() * r.w as f64, rnd() * r.h as f64, 10.0 + 60.0 * rnd(), rnd())).collect();
    for y in 0..r.h {
        for x in 0..r.w {
            let p = inv * Vec3::new(x as f64, y as f64, 1.0);
            let (u, v) = (p.x / p.z, p.y / p.z);
            let code = if (-0.75..=5.75).contains(&u) && (-0.75..=3.75).contains(&v) {
                let (cu, cv) = (u.round(), v.round());
                if (u - cu).abs() < 0.41 && (v - cv).abs() < 0.41 && (0.0..=5.0).contains(&cu) && (0.0..=3.0).contains(&cv) {
                    patch((cv as usize) * 6 + cu as usize)
                } else {
                    card
                }
            } else if (-0.95..=5.95).contains(&u) && (-0.95..=3.95).contains(&v) {
                bg(0.9) // light frame around the card
            } else {
                let mut t = 0.15 + 0.2 * (x as f64 / r.w as f64);
                for (bx, by, br, bv) in &blobs {
                    let d = ((x as f64 - bx).powi(2) + (y as f64 - by).powi(2)).sqrt();
                    if d < *br {
                        t = 0.05 + 0.6 * bv;
                    }
                }
                bg(t)
            };
            // A little noise, as in a real still (a perfectly flat area at the image's
            // maximum would read as clipped).
            data.extend(code.iter().map(|c| (*c + 0.0015 * (rnd() - 0.5)) as f32));
        }
    }
    data
}

fn di_still(corners: [[f64; 2]; 4], look: Mat3, exposure: f64) -> ChartImage {
    let refs = ChartEdition::After2014.xyz_d65();
    let enc = |xyz: Vec3| -> [f64; 3] {
        let d = (xyz_to_dwg() * xyz).map(di_encode);
        [d.x, d.y, d.z]
    };
    let r = Render { corners, w: 960, h: 720 };
    let data = render(
        &r,
        &|i| enc(look * refs[i] * exposure),
        enc(whites::D65.to_xyz() * 0.004),
        &|t| enc(Vec3::new(0.6, 0.8, 1.0) * t * 0.5),
    );
    ChartImage::from_rgb("synthetic.tif", r.w, r.h, data, true).quantised(16)
}

const UPSIDE_DOWN: [[f64; 2]; 4] = [[690.0, 455.0], [310.0, 470.0], [300.0, 238.0], [670.0, 230.0]];

#[test]
fn finds_an_upside_down_chart_in_dwg_di_and_recovers_the_look() {
    let img = di_still(UPSIDE_DOWN, cam(), 0.5);
    let lat = find_lattice(&img).expect("chart found");
    assert!(lat.members.len() >= 18, "{} patches", lat.members.len());
    let a = analyze(&img, Some(&lat), &ChartSettings::default()).unwrap();
    assert_eq!(a.placement.orientation, Orientation::Rot180);
    for (i, c) in a.placement.centres().iter().enumerate() {
        let want = lutmatrix_core::chart::detect::Placement::from_corners(UPSIDE_DOWN).unwrap().project((i % 6) as f64, (i / 6) as f64);
        assert!((c[0] - want[0]).hypot(c[1] - want[1]) < 2.0, "patch {i}: {c:?} vs {want:?}");
    }
    assert_eq!(a.space.curve, LogCurve::DaVinciIntermediate);
    assert_eq!(a.space.gamut, StillGamut::DaVinciWideGamut);
    assert!(a.guess.curve_confidence >= Confidence::None);
    let f = a.fit.as_ref().unwrap();
    assert_eq!(f.used, 24, "{:?}", a.patches.iter().filter_map(|p| p.reason.clone()).collect::<Vec<_>>());
    assert!(f.mean_after < 0.15, "after {}", f.mean_after);
    assert!(f.mean_before > 1.0);
    assert!((f.exposure_ratio.log2() + 1.0).abs() < 0.02, "exposure {}", f.exposure_ratio.log2());
    // M undoes the look and the exposure.
    let m = f.m_xyz * cam() * 0.5;
    assert!((m - Mat3::identity()).abs().max() < 2e-3, "{m}");
    // Keeping the exposure: unity neutral gain, same colours.
    let s = ChartSettings { fit: lutmatrix_core::chart::fit::FitSettings { exposure: Exposure::Keep, ..Default::default() }, ..Default::default() };
    let k = analyze(&img, Some(&lat), &s).unwrap();
    let fk = k.fit.unwrap();
    assert!((fk.neutral_gain - 1.0).abs() < 2e-3 && fk.mean_after < 0.15, "{} {}", fk.neutral_gain, fk.mean_after);
}

#[test]
fn linear_xyz_float_still_rotated_90() {
    let refs = ChartEdition::After2014.xyz_d65();
    // Clockwise: dark skin top right, bluish green bottom right.
    let corners = [[640.0, 140.0], [660.0, 600.0], [380.0, 610.0], [370.0, 150.0]];
    let r = Render { corners, w: 900, h: 760 };
    let data = render(&r, &|i| (cam() * refs[i] * 0.8).into(), [0.003; 3], &|t| [t * 0.3, t * 0.35, t * 0.5]);
    let img = ChartImage::from_rgb("still.exr", r.w, r.h, data, true);
    let a = analyze(&img, None, &ChartSettings::default()).unwrap();
    assert_eq!(a.placement.orientation, Orientation::Rot90);
    assert_eq!(a.space.curve, LogCurve::Linear);
    assert_eq!(a.guess.curve_confidence, Confidence::High);
    assert_eq!(a.space.gamut, StillGamut::Xyz);
    let f = a.fit.unwrap();
    assert!(f.mean_after < 0.05, "{}", f.mean_after);
}

#[test]
fn clipped_patches_are_left_out() {
    // Two stops over: white and neutral 8 clip in a 16-bit integer DI file? DI reaches
    // code 1 only far above, so clip the file at 0.5 instead (a still exported with a
    // clipped highlight), and push some colours negative.
    let refs = ChartEdition::After2014.xyz_d65();
    let enc = |xyz: Vec3| -> [f64; 3] {
        let d = (xyz_to_dwg() * xyz).map(di_encode);
        [d.x.min(0.5), d.y.min(0.5), d.z.min(0.5)]
    };
    let r = Render { corners: UPSIDE_DOWN, w: 960, h: 720 };
    // A strong cyan cast makes the red channel of some patches negative (code 0).
    let tint = Mat3::from_diagonal(&Vec3::new(0.35, 1.0, 1.2));
    let look = dwg_to_xyz() * tint * xyz_to_dwg();
    let data = render(&r, &|i| enc(look * refs[i] * 2.0), enc(whites::D65.to_xyz() * 0.004), &|t| enc(Vec3::new(0.5, 0.8, 1.0) * t * 0.5));
    let img = ChartImage::from_rgb("t.tif", r.w, r.h, data, true).quantised(16);
    let a = analyze(&img, None, &ChartSettings::default()).unwrap();
    let white = &a.patches[18];
    assert!(!white.used && white.clipped_high.iter().any(|c| *c), "{:?}", white.reason);
    let f = a.fit.as_ref().unwrap();
    assert!(f.used < 24 && f.used >= 6);
    assert!(a.notes.iter().any(|n| n.contains("clipped")), "{:?}", a.notes);
}

#[test]
fn matches_one_camera_to_another() {
    let hero_look = Mat3::new(0.95, 0.06, 0.0, 0.02, 0.97, 0.01, 0.0, 0.03, 0.98);
    let hero = di_still(UPSIDE_DOWN, hero_look, 0.7);
    let ha = analyze(&hero, None, &ChartSettings::default()).unwrap();
    let other = di_still(UPSIDE_DOWN, cam(), 0.5);
    let s = ChartSettings { target: ha.as_target(), ..Default::default() };
    let a = analyze(&other, None, &s).unwrap();
    let f = a.fit.unwrap();
    assert!(f.mean_after < 0.15, "{}", f.mean_after);
    // M · cam · 0.5 = hero · 0.7
    let want = hero_look * 0.7;
    assert!((f.m_xyz * cam() * 0.5 - want).abs().max() < 3e-3);
}

#[test]
fn manual_corners_replace_detection() {
    let img = di_still(UPSIDE_DOWN, cam(), 0.5);
    let s = ChartSettings { corners: Some(UPSIDE_DOWN), ..Default::default() };
    let a = analyze(&img, None, &s).unwrap();
    assert_eq!(a.placement_source, "manual");
    assert_eq!(a.placement.orientation, Orientation::Rot180);
    assert!(a.fit.unwrap().mean_after < 0.15);
}

#[test]
fn opendrt_preset_on_a_perfect_chart_stays_put() {
    let img = di_still(UPSIDE_DOWN, Mat3::identity(), 1.0);
    let a = analyze(&img, None, &ChartSettings::default()).unwrap();
    let s = OdrtChartSettings { after_matrix: false, max_iterations: 10, ..Default::default() };
    let o = fit_opendrt_chart(&a, &s).unwrap();
    assert!(o.before.mean < 0.3, "stock on a perfect chart: {}", o.before.mean);
    assert!(o.after.mean <= o.before.mean + 1e-6);
    assert!(o.safety.ok(), "{:?}", o.safety);
    // A camera look: the preset gets closer than the stock look.
    let img = di_still(UPSIDE_DOWN, cam(), 1.0);
    let a = analyze(&img, None, &ChartSettings::default()).unwrap();
    let o = fit_opendrt_chart(&a, &OdrtChartSettings { after_matrix: false, ..Default::default() }).unwrap();
    assert!(o.after.mean < 0.8 * o.before.mean, "{} vs {}", o.after.mean, o.before.mean);
}

// ---------------------------------------------------------------------------
// The DCTL: compiled as C and C++ with a shim, compared with the f64 correction.
// ---------------------------------------------------------------------------
const SHIM: &str = r#"
#include <math.h>
#include <stdio.h>
typedef struct { float x, y, z; } float3;
static float3 make_float3(float x, float y, float z) { float3 r; r.x = x; r.y = y; r.z = z; return r; }
#define __DEVICE__ static
#define _powf powf
#define _log2f log2f
#define _fmaxf fmaxf
#define _fminf fminf
#define DEFINE_UI_PARAMS(name, label, kind, def, lo, hi, step) static float name = def;
"#;
const MAIN: &str = r#"
int main(void) {
    float r, g, b;
    while (scanf("%f %f %f", &r, &g, &b) == 3) {
        float3 o = transform(1, 1, 0, 0, r, g, b);
        printf("%.9g %.9g %.9g\n", o.x, o.y, o.z);
    }
    return 0;
}
"#;

#[test]
fn chart_dctl_compiles_and_matches() {
    let img = di_still(UPSIDE_DOWN, cam(), 0.5);
    let cases = [(Model::Matrix, false, false), (Model::MatrixTone, false, false), (Model::Matrix, true, true), (Model::MatrixTone, false, true)];
    for (model, preserve, protect) in cases {
        let s = ChartSettings {
            fit: lutmatrix_core::chart::fit::FitSettings { model, preserve_neutrals: preserve, ..Default::default() },
            // A clip inside the tested codes (up to 0.7), so the protection is exercised.
            highlights: HighlightSettings { enabled: protect, clip_di: Some(0.55), softness: 1.5, strength: 0.9, detect: 0.3 },
            ..Default::default()
        };
        let mut a = analyze(&img, None, &s).unwrap();
        // Exercise a real curve in the tone model.
        if let Some(f) = a.fit.as_mut() {
            if f.tone.is_some() {
                f.tone = Some(Tone { power: [1.2, 0.9, 1.05], black: [0.002, -0.001, 0.0] });
            }
        }
        let src = a.dctl("2026-01-01").unwrap();
        assert!(src.is_ascii());
        assert_eq!(src.contains("TONE_P_R"), model == Model::MatrixTone);
        assert_eq!(src.contains("p_HlClip"), protect);
        let fit = a.fit.clone().unwrap();
        let hl = a.highlights;
        let dir = std::env::temp_dir().join(format!("lutmatrix-chart-dctl-{}-{model:?}-{preserve}-{protect}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.dctl"), &src).unwrap();
        std::fs::write(dir.join("main.c"), format!("{SHIM}\n#include \"c.dctl\"\n{MAIN}")).unwrap();
        let codes: Vec<Vec3> = (0..200)
            .map(|i| {
                let f = i as f64;
                Vec3::new((f * 0.137).fract() * 0.7, (f * 0.291).fract() * 0.7, (f * 0.533).fract() * 0.7)
            })
            .collect();
        let want = |c: &Vec3| -> Vec3 { (xyz_to_dwg() * hl.apply(&fit, &c.map(di_decode))).map(di_encode) };
        for (cc, args) in [("cc", vec!["-std=c99"]), ("c++", vec!["-x", "c++", "-std=c++14"])] {
            if Command::new(cc).arg("--version").output().map(|o| !o.status.success()).unwrap_or(true) {
                continue;
            }
            let exe = dir.join(format!("t_{}", cc.replace('+', "p")));
            let out = Command::new(cc)
                .args(&args)
                .args(["-Wall", "-Werror", "-Wno-unused-parameter", "-Wno-unused-function", "main.c", "-o"])
                .arg(&exe)
                .arg("-lm")
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "{cc}: {}", String::from_utf8_lossy(&out.stderr));
            let input: String = codes.iter().map(|c| format!("{:.9} {:.9} {:.9}\n", c.x, c.y, c.z)).collect();
            let mut child = Command::new(&exe).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
            use std::io::Write;
            child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
            let text = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
            for (line, c) in text.lines().zip(&codes) {
                let got: Vec<f64> = line.split_whitespace().map(|v| v.parse().unwrap()).collect();
                let w = want(&c.map(|v| v as f32 as f64));
                for k in 0..3 {
                    assert!((got[k] - w[k]).abs() < 2e-5 * w[k].abs().max(1.0), "{cc} {model:?} {preserve} {protect} {c:?}: {got:?} vs {w:?}");
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn clip_level_is_measured_from_the_still() {
    // A DI still whose green piles up at 0.62 (a sensor clip), red and blue below.
    let refs = ChartEdition::After2014.xyz_d65();
    let enc = |xyz: Vec3| -> [f64; 3] {
        let d = (xyz_to_dwg() * xyz).map(di_encode);
        [d.x, d.y.min(0.62), d.z]
    };
    let r = Render { corners: UPSIDE_DOWN, w: 960, h: 720 };
    let data = render(&r, &|i| enc(cam() * refs[i] * 0.5), enc(whites::D65.to_xyz() * 0.004), &|t| enc(Vec3::new(0.9, 3.0, 1.0) * t * 20.0));
    let img = ChartImage::from_rgb("clip.exr", r.w, r.h, data, true);
    let a = analyze(&img, None, &ChartSettings::default()).unwrap();
    assert_eq!(a.highlights.measured_source, ClipSource::Measured, "{:?}", a.clip);
    assert!((a.highlights.clip_di - 0.62).abs() < 2e-3, "{}", a.highlights.clip_di);
    assert!(!a.highlights.enabled);
    // Set by hand wins.
    let s = ChartSettings { highlights: HighlightSettings { enabled: true, clip_di: Some(0.7), ..Default::default() }, ..Default::default() };
    let a = analyze(&img, None, &s).unwrap();
    assert_eq!((a.highlights.clip_di, a.highlights.source), (0.7, ClipSource::Set));
    // Nothing clips: the brightest value, flagged as such.
    let img = di_still(UPSIDE_DOWN, cam(), 0.5);
    let a = analyze(&img, None, &ChartSettings::default()).unwrap();
    assert_eq!(a.highlights.source, ClipSource::Brightest);
}

#[test]
fn space_override_is_used() {
    let img = di_still(UPSIDE_DOWN, cam(), 0.5);
    let sp = StillSpace { curve: LogCurve::AcesCct, gamut: StillGamut::AcesAp1 };
    let a = analyze(&img, None, &ChartSettings { space: Some(sp), ..Default::default() }).unwrap();
    assert_eq!(a.space, sp);
    assert!(!a.space_detected);
}
