//! Fitting correctness on synthetic LUTs built from known XYZ matrices.

use lutmatrix_core::analysis::{analyze, analyze_transform, Analysis, Severity};
use lutmatrix_core::lut::{FnTransform, Lut, Lut3D, LutFormat};
use lutmatrix_core::math::{whites, Mat3, Vec3, Xy};
use lutmatrix_core::sample::{GrayAnchor, RingLimit, SampleStatus, Settings, WhiteMode};
use lutmatrix_core::spaces::{CameraSpaceId, DisplayGamut, DisplayTransfer, InputSpace, OutputSpace};

const SIZE: usize = 65;

/// Build a camera-log → display LUT whose colour transform is exactly `m_true` in XYZ,
/// optionally followed by a tone curve `tone(Y_nits) -> Y_nits` applied to luminance
/// (chromaticity-preserving) or per display channel.
#[derive(Clone, Copy)]
enum Tone {
    None,
    Luminance(fn(f64) -> f64),
    PerChannel(fn(f64) -> f64),
}

/// The exact transform (code in → code out).
fn exact(input: InputSpace, output: OutputSpace, m_true: Mat3, gain_nits: f64, tone: Tone) -> impl Fn(&Vec3) -> Vec3 {
    let cam_to_xyz = input.rgb_to_xyz_d65();
    let to_disp = output.xyz_to_rgb();
    move |code: &Vec3| {
        let cam = input.decode(code);
        let mut scene = cam_to_xyz * cam;
        // Luminance tone curve on *scene* Y (chromaticity-preserving), before the matrix.
        if let Tone::Luminance(f) = tone {
            if scene.y > 1e-9 {
                scene *= f(scene.y * gain_nits) / (scene.y * gain_nits);
            }
        }
        let xyz = m_true * scene * gain_nits;
        let mut rgb = to_disp * xyz;
        if let Tone::PerChannel(f) = tone {
            rgb = rgb.map(|v| if v > 0.0 { f(v) } else { v });
        }
        output.encode(&rgb).map(|v| v.clamp(0.0, 1.0))
    }
}

/// The same transform baked into a 65³ LUT (adds tetrahedral interpolation error).
fn build_lut(input: InputSpace, output: OutputSpace, m_true: &Mat3, gain_nits: f64, tone: Tone) -> Lut {
    let f = exact(input, output, *m_true, gain_nits, tone);
    let cube = Lut3D::from_fn(SIZE, |c| f(&c));
    Lut {
        format: LutFormat::Cube,
        filename: Some("synthetic.cube".into()),
        title: None,
        comments: vec![],
        shaper: None,
        cube: Some(cube),
        warnings: vec![],
    }
}

/// A plausible "look" matrix, scaled so it maps D65 neutral to unit luminance gain.
fn look_matrix() -> Mat3 {
    let m = Mat3::new(1.08, -0.06, 0.02, -0.03, 1.04, -0.01, 0.01, -0.07, 1.10);
    let y = (m * whites::D65.to_xyz()).y;
    m / y
}

fn sdr(g: DisplayGamut, gamma: f64) -> OutputSpace {
    OutputSpace::new(g, DisplayTransfer::Gamma { gamma })
}
fn pq(g: DisplayGamut, peak: f64) -> OutputSpace {
    OutputSpace::new(g, DisplayTransfer::Pq { peak_nits: peak })
}

/// Preserve-LUT-white settings: needed wherever a test expects to recover a matrix
/// that shifts white (D65 mode, the default, would remove that shift).
fn preserve() -> Settings {
    Settings { white_mode: WhiteMode::PreserveLut, ..Settings::default() }
}

fn run(lut: &Lut, input: InputSpace, output: OutputSpace, s: &Settings) -> Analysis {
    analyze(lut, input, output, s, None).expect("analysis")
}

fn run_exact(f: impl Fn(&Vec3) -> Vec3, input: InputSpace, output: OutputSpace, s: &Settings) -> Analysis {
    analyze_transform(&FnTransform(f), input, output, s, None).expect("analysis")
}

/// Tolerances. Exact transforms must be recovered to numerical precision; a 65³ LUT
/// adds interpolation error (log-encoded in, gamma/PQ-encoded out), which is real
/// LUT behaviour and is what Resolve itself would show.
const TOL_EXACT: f64 = 1e-7;
/// (mean, max) ΔE2000 between applying `a` and `b` to the default test ring.
fn matrix_de(a: &Mat3, b: &Mat3, input: InputSpace, output: OutputSpace) -> (f64, f64) {
    use lutmatrix_core::difference::{delta_e_2000, xyz_to_lab};
    let wn = whites::D65.to_xyz();
    let d: Vec<f64> = lutmatrix_core::sample::hue_ring(&Settings::default(), &input, &output, 0.18)
        .iter()
        .map(|(_, x)| delta_e_2000(&xyz_to_lab(&(a * x), &wn), &xyz_to_lab(&(b * x), &wn)))
        .collect();
    (d.iter().sum::<f64>() / d.len() as f64, d.iter().cloned().fold(0.0, f64::max))
}

/// (mean, max) ΔE2000 the 65³ grid itself adds at the test ring, vs the exact transform.
fn grid_error(lut: &Lut, f: &dyn Fn(&Vec3) -> Vec3, input: InputSpace, output: OutputSpace) -> (f64, f64) {
    use lutmatrix_core::difference::{delta_e_2000, xyz_to_lab};
    use lutmatrix_core::lut::Interpolation;
    let to_xyz = |code: &Vec3| output.rgb_to_xyz() * output.decode(code);
    let g = input.encode(&(input.xyz_d65_to_rgb() * (whites::D65.to_xyz() * 0.18)));
    let wn = whites::D65.to_xyz() * (to_xyz(&f(&g)).y / 0.18);
    let d: Vec<f64> = lutmatrix_core::sample::hue_ring(&Settings::default(), &input, &output, 0.18)
        .iter()
        .map(|(_, x)| {
            let code = input.encode(&(input.xyz_d65_to_rgb() * x));
            let a = to_xyz(&f(&code));
            let b = to_xyz(&lut.apply(&code, Interpolation::Tetrahedral));
            delta_e_2000(&xyz_to_lab(&a, &wn), &xyz_to_lab(&b, &wn))
        })
        .collect();
    (d.iter().sum::<f64>() / d.len() as f64, d.iter().cloned().fold(0.0, f64::max))
}

/// The matrix fitted to a 65³ LUT must be as close to the generating matrix as the
/// LUT's own grid error allows: the perceptual fit follows the LUT as interpolated
/// (what Resolve applies), so it may deviate by a small multiple of that error.
fn assert_grid_close(
    a: &Mat3,
    truth: &Mat3,
    lut: &Lut,
    f: &dyn Fn(&Vec3) -> Vec3,
    input: InputSpace,
    output: OutputSpace,
    what: &str,
) {
    let (gm, gx) = grid_error(lut, f, input, output);
    let (mean, max) = matrix_de(a, truth, input, output);
    assert!(
        mean <= 3.0 * gm + 0.05 && max <= 3.0 * gx + 0.1,
        "{what}: vs true matrix ΔE2000 mean {mean:.3} max {max:.3}; grid error mean {gm:.3} max {gx:.3}\n{a}"
    );
}

#[test]
fn synthetic_round_trip_recovers_matrix() {
    let cases = [
        (CameraSpaceId::ArriLogC3Awg3, sdr(DisplayGamut::Rec709, 2.4), 10.0),
        (CameraSpaceId::ArriLogC4Awg4, sdr(DisplayGamut::P3D65, 2.6), 8.0),
        (CameraSpaceId::SonySLog3SGamut3Cine, pq(DisplayGamut::Rec2020, 1000.0), 26.0),
        (CameraSpaceId::RedLog3G10Rwg, pq(DisplayGamut::P3D65, 4000.0), 20.0),
        (CameraSpaceId::PanasonicVLogVGamut, sdr(DisplayGamut::Rec709, 2.2), 12.0),
        (CameraSpaceId::CanonLog3CinemaGamut, sdr(DisplayGamut::Rec709, 2.4), 11.0),
        (CameraSpaceId::BmdFilmGen5, sdr(DisplayGamut::Rec709, 2.4), 11.0),
        (CameraSpaceId::FujiFLog2FGamut, pq(DisplayGamut::Rec2020, 1000.0), 26.0),
        (CameraSpaceId::DaVinciIntermediateDwg, sdr(DisplayGamut::Rec709, 2.4), 11.0),
        (CameraSpaceId::AcesCctAp1, sdr(DisplayGamut::P3D65, 2.6), 9.0),
    ];
    let m_true = look_matrix();
    for (id, out, gain_at_gray) in cases {
        let input = InputSpace::new(id);
        let gain = gain_at_gray / 0.18;
        for refine in [false, true] {
            let s = Settings { refine, ..preserve() };
            let a = run_exact(exact(input, out, m_true, gain, Tone::None), input, out, &s);
            let err = (a.m_xyz - m_true).abs().max();
            assert!(err < TOL_EXACT, "{id:?} → {} exact refine={refine}: {err:e}\n{}", out.name(), a.m_xyz);
            assert!(a.stats_refined.as_ref().unwrap_or(&a.stats_ls).max < 1e-5);

            let lut = build_lut(input, out, &m_true, gain, Tone::None);
            let a = run(&lut, input, out, &s);
            let f = exact(input, out, m_true, gain, Tone::None);
            assert_grid_close(&a.m_xyz, &m_true, &lut, &f, input, out, &format!("{id:?} → {} 65³ refine={refine}", out.name()));
            // Fit error against the LUT itself (includes its grid error): ΔE2000 for
            // SDR; ΔE ITP for PQ, where Log3G10's wide range makes a 65³ grid coarse.
            let st = a.stats_refined.as_ref().unwrap_or(&a.stats_ls);
            let limit = if out.transfer.is_hdr() { 2.5 } else { 0.3 };
            assert!(st.mean < limit, "{id:?}: 65³ mean error {}", st.mean);
        }
    }
}

#[test]
fn identity_colour_lut_gives_identity_matrix() {
    for (id, out) in [
        (CameraSpaceId::ArriLogC4Awg4, sdr(DisplayGamut::Rec709, 2.4)),
        (CameraSpaceId::SonySLog3SGamut3, pq(DisplayGamut::P3D65, 1000.0)),
    ] {
        let input = InputSpace::new(id);
        let lut = build_lut(input, out, &Mat3::identity(), 11.0 / 0.18, Tone::None);
        for white_mode in [WhiteMode::PreserveLut, WhiteMode::D65] {
            let s = Settings { white_mode, ..preserve() };
            let f = exact(input, out, Mat3::identity(), 11.0 / 0.18, Tone::None);
            let a = run_exact(f, input, out, &s);
            let err = (a.m_xyz - Mat3::identity()).abs().max();
            assert!(err < TOL_EXACT, "{id:?} {white_mode:?} exact: {err:e}\n{}", a.m_xyz);
            let a = run(&lut, input, out, &s);
            let f = exact(input, out, Mat3::identity(), 11.0 / 0.18, Tone::None);
            assert_grid_close(&a.m_xyz, &Mat3::identity(), &lut, &f, input, out, &format!("{id:?} {white_mode:?} 65³"));
        }
    }
}

#[test]
fn p3_dci_white_preserved_or_removed() {
    // A naive P3 rendering shown on a DCI-white projector: equal display RGB = DCI white.
    // In XYZ that is NPM(P3-DCI) · NPM(P3-D65)⁻¹, normalised to unit luminance gain.
    let shift = DisplayGamut::P3Dci.primaries().rgb_to_xyz() * DisplayGamut::P3D65.primaries().xyz_to_rgb();
    let shift = shift / (shift * whites::D65.to_xyz()).y;
    let input = InputSpace::new(CameraSpaceId::ArriLogC4Awg4);
    let out = sdr(DisplayGamut::P3Dci, 2.6);
    let lut = build_lut(input, out, &shift, 7.0 / 0.18, Tone::None);
    let d65 = whites::D65.to_xyz();
    let f = || exact(input, out, shift, 7.0 / 0.18, Tone::None);

    // Preserve: matrix keeps the DCI white shift.
    let a = run_exact(f(), input, out, &preserve());
    let w = Xy::from_xyz(&(a.m_xyz * d65));
    assert!((w.x - 0.314).abs() < 1e-9 && (w.y - 0.351).abs() < 1e-9, "preserve white {w:?}");
    assert!((a.m_xyz - shift).abs().max() < TOL_EXACT);
    let a = run(&lut, input, out, &preserve());
    let w = Xy::from_xyz(&(a.m_xyz * d65));
    assert!((w.x - 0.314).abs() < 2e-4 && (w.y - 0.351).abs() < 2e-4, "65³ preserve white {w:?}");

    // D65: neutral maps to D65 exactly. The colours are the LUT's colours adapted by
    // Bradford (measured white → D65), i.e. M = CAT · shift for this pure-matrix LUT.
    let exact_a = run_exact(f(), input, out, &Settings { white_mode: WhiteMode::D65, ..preserve() });
    let cat = exact_a.white_adaptation.expect("CAT used");
    assert!((exact_a.m_xyz - cat * shift).abs().max() < TOL_EXACT, "{}", exact_a.m_xyz);
    let grid_a = run(&lut, input, out, &Settings { white_mode: WhiteMode::D65, ..preserve() });
    for a in [&exact_a, &grid_a] {
        let wv = a.m_xyz * d65;
        assert!((wv - d65).abs().max() < 1e-12, "D65 neutral {wv:?}");
    }
    assert_grid_close(&grid_a.m_xyz, &(cat * shift), &lut, &f(), input, out, "DCI D65 mode 65³");
}

#[test]
fn exposure_normalisation_removes_a_luminance_tone_curve() {
    // Filmic-ish curve on luminance only (chromaticity preserved).
    fn tone(y: f64) -> f64 {
        let x = y / 100.0;
        100.0 * x * x.powf(0.5) / (x.powf(1.5) + 0.05)
    }
    let input = InputSpace::new(CameraSpaceId::ArriLogC3Awg3);
    let out = sdr(DisplayGamut::Rec709, 2.4);
    let m_true = look_matrix();
    let a = run_exact(exact(input, out, m_true, 10.0 / 0.18, Tone::Luminance(tone)), input, out, &preserve());
    assert!((a.m_xyz - m_true).abs().max() < TOL_EXACT, "{}", a.m_xyz);
    assert!((a.contrast_at_gray - 1.0).abs() > 0.05, "tone curve should be visible: contrast {}", a.contrast_at_gray);
    let lut = build_lut(input, out, &m_true, 10.0 / 0.18, Tone::Luminance(tone));
    let a = run(&lut, input, out, &preserve());
    let f = exact(input, out, m_true, 10.0 / 0.18, Tone::Luminance(tone));
    assert_grid_close(&a.m_xyz, &m_true, &lut, &f, input, out, "luminance tone curve 65³");
}

#[test]
fn per_channel_tone_curve_is_flagged() {
    fn tone(v: f64) -> f64 {
        // Steep per-channel S-curve in display-linear nits.
        let x = v / 100.0;
        100.0 * x * x / (x * x + 0.02)
    }
    let input = InputSpace::new(CameraSpaceId::ArriLogC4Awg4);
    let out = sdr(DisplayGamut::Rec709, 2.4);
    let lut = build_lut(input, out, &Mat3::identity(), 12.0 / 0.18, Tone::PerChannel(tone));
    let a = run(&lut, input, out, &preserve());
    assert!(
        a.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning && d.code == "exposure_dependent_colour"),
        "{:#?}",
        a.diagnostics
    );
}

#[test]
fn anchor_b_samples_where_gray_hits_display_middle_gray() {
    fn tone(y: f64) -> f64 {
        y * 0.5 // the LUT renders everything a stop darker
    }
    let input = InputSpace::new(CameraSpaceId::ArriLogC4Awg4);
    let out = sdr(DisplayGamut::Rec709, 2.4);
    let s = Settings { anchor: GrayAnchor::DisplayTarget, display_middle_gray_nits: Some(12.0), ..preserve() };
    let f = exact(input, out, Mat3::identity(), 12.0 / 0.18, Tone::Luminance(tone));
    let a = run_exact(f, input, out, &s);
    assert!((a.anchor_stops - 1.0).abs() < 1e-9, "anchor {}", a.anchor_stops);
    assert!((a.gray_out_nits - 12.0).abs() < 1e-9, "gray out {}", a.gray_out_nits);
    assert!((a.stops_from_display_middle_gray + 1.0).abs() < 1e-9);
    let lut = build_lut(input, out, &Mat3::identity(), 12.0 / 0.18, Tone::Luminance(tone));
    let a = run(&lut, input, out, &s);
    assert!((a.anchor_stops - 1.0).abs() < 0.01, "65³ anchor {}", a.anchor_stops);
    assert!((a.gray_out_nits - 12.0).abs() < 1e-6, "gray out {}", a.gray_out_nits);
}

#[test]
fn bracket_and_constraints() {
    let input = InputSpace::new(CameraSpaceId::ArriLogC4Awg4);
    let out = sdr(DisplayGamut::Rec709, 2.4);
    let m_true = look_matrix();
    let f = || exact(input, out, m_true, 10.0 / 0.18, Tone::None);
    let a = run_exact(f(), input, out, &Settings { bracket_stops: 0.5, ..preserve() });
    assert_eq!(a.samples.len(), 3 * (36 * 4 + 1));
    assert!((a.m_xyz - m_true).abs().max() < TOL_EXACT);
    // Lock neutral in preserve mode: M·D65 equals the LUT's white exactly.
    let lut = build_lut(input, out, &m_true, 10.0 / 0.18, Tone::None);
    let a = run(&lut, input, out, &Settings { lock_neutral: true, ..preserve() });
    let d65 = whites::D65.to_xyz();
    let measured_white = a.m_xyz * d65;
    assert!(a.stats_refined.as_ref().unwrap().neutral_duv < 1e-12);
    assert!((measured_white - m_true * d65).abs().max() < 1e-4);
}

#[test]
fn out_of_gamut_and_clipped_samples_are_skipped() {
    let input = InputSpace::new(CameraSpaceId::ArriLogC4Awg4);
    let out = sdr(DisplayGamut::Rec709, 2.4);
    // Bright rendering so saturated colours clip at the top.
    let s = Settings { ring_limit: RingLimit::Camera, saturations: vec![0.5, 1.2], ..preserve() };
    let a = run_exact(exact(input, out, Mat3::identity(), 60.0 / 0.18, Tone::None), input, out, &s);
    let count = |st| a.samples.iter().filter(|r| r.status == st).count();
    assert!(count(SampleStatus::OutsideCameraGamut) >= 30, "outside camera gamut");
    assert!(count(SampleStatus::ClippedOutput) > 0, "clipped");
    assert!(a.samples.iter().filter(|r| r.status != SampleStatus::Ok).all(|r| r.error_ls.is_none()));
    assert!((a.m_xyz - Mat3::identity()).abs().max() < TOL_EXACT, "{}", a.m_xyz);
}

/// Diagnostic (run with --ignored --nocapture): how much error does a 65³ grid add?
#[test]
#[ignore]
fn report_grid_interpolation_error() {
    use lutmatrix_core::difference::{delta_e_2000, xyz_to_lab};
    use lutmatrix_core::lut::Interpolation;
    let input = InputSpace::new(CameraSpaceId::ArriLogC3Awg3);
    let out = sdr(DisplayGamut::Rec709, 2.4);
    let m_true = look_matrix();
    let f = exact(input, out, m_true, 10.0 / 0.18, Tone::None);
    for size in [33usize, 65, 129] {
        let cube = Lut3D::from_fn(size, |c| f(&c));
        let wn = whites::D65.to_xyz() * 100.0;
        let mut worst: f64 = 0.0;
        let mut sum = 0.0;
        let ring = lutmatrix_core::sample::hue_ring(&Settings::default(), &input, &out, 0.18);
        for (_, x) in &ring {
            let code = input.encode(&(input.xyz_d65_to_rgb() * x));
            let a = out.rgb_to_xyz() * out.decode(&f(&code));
            let b = out.rgb_to_xyz() * out.decode(&cube.apply(&code, Interpolation::Tetrahedral));
            let d = delta_e_2000(&xyz_to_lab(&a, &wn), &xyz_to_lab(&b, &wn));
            worst = worst.max(d);
            sum += d;
        }
        println!("{size}³: LUT interpolation ΔE2000 mean {:.3} max {:.3}", sum / ring.len() as f64, worst);
    }
}

#[test]
fn d65_is_the_default_white_mode() {
    assert_eq!(Settings::default().white_mode, WhiteMode::D65);
    // With the default, a white-shifting look comes out D65-neutral.
    let input = InputSpace::new(CameraSpaceId::ArriLogC4Awg4);
    let out = sdr(DisplayGamut::Rec709, 2.4);
    let a = run_exact(exact(input, out, look_matrix(), 10.0 / 0.18, Tone::None), input, out, &Settings::default());
    let d65 = whites::D65.to_xyz();
    assert!((a.m_xyz * d65 - d65).abs().max() < 1e-12);
}

// ---------------------------------------------------------------------------
// Fitting through a reference display transform
// ---------------------------------------------------------------------------
mod through {
    use super::*;
    use lutmatrix_core::analysis::{analyze_with, Reference};
    use lutmatrix_core::spaces::davinci::{di_decode, di_encode, dwg_to_xyz, xyz_to_dwg};

    /// A per-channel filmic DRT, DWG/DI in → Rec.709 gamma 2.4 out (like many real DRTs,
    /// it adds saturation through its per-channel curve).
    fn drt(code: &Vec3) -> Vec3 {
        let xyz = dwg_to_xyz() * code.map(di_decode);
        let rgb = DisplayGamut::Rec709.primaries().xyz_to_rgb() * xyz;
        let tone = |v: f64| {
            let v = v.max(0.0) * 1.6;
            (v * v / (v * v + 0.12)).powf(0.9)
        };
        rgb.map(|v| tone(v).powf(1.0 / 2.4))
    }

    /// LUT that is "DRT after a known XYZ matrix": camera log → DRT(M·scene).
    fn lut_through(input: InputSpace, m_true: Mat3) -> impl Fn(&Vec3) -> Vec3 {
        let cam_to_xyz = input.rgb_to_xyz_d65();
        move |code: &Vec3| drt(&(xyz_to_dwg() * (m_true * (cam_to_xyz * input.decode(code)))).map(di_encode))
    }

    fn with_d65_fixed(m: Mat3) -> Mat3 {
        // M' = M + (w − M·w)·wᵀ/|w|²  ⇒  M'·w = w
        let w = whites::D65.to_xyz();
        m + (w - m * w) * w.transpose() / w.norm_squared()
    }

    fn reference() -> FnTransform<fn(&Vec3) -> Vec3> {
        FnTransform(drt as fn(&Vec3) -> Vec3)
    }

    fn run_through(m_true: Mat3, white_mode: WhiteMode) -> Analysis {
        let input = InputSpace::new(CameraSpaceId::ArriLogC3Awg3);
        let out = sdr(DisplayGamut::Rec709, 2.4);
        let r = reference();
        let rf = Reference {
            transform: &r,
            name: "test DRT".into(),
            input: InputSpace::new(CameraSpaceId::DaVinciIntermediateDwg),
            output: out,
        };
        let lut = FnTransform(lut_through(input, m_true));
        let s = Settings { white_mode, ..Settings::default() };
        analyze_with(&lut, input, out, &s, None, Some(&rf)).expect("analysis")
    }

    #[test]
    fn recovers_matrix_through_nonlinear_drt_d65() {
        let m_true = with_d65_fixed(look_matrix());
        let a = run_through(m_true, WhiteMode::D65);
        let err = (a.m_xyz - m_true).abs().max();
        assert!(err < 1e-5, "through-DRT recovery error {err:e}\n{}", a.m_xyz);
        let r = a.reference.as_ref().unwrap();
        assert!(r.gain_percent > 95.0, "gain {}", r.gain_percent);
        assert!(a.stats_refined.as_ref().unwrap().mean < 1e-3);
        // The plain colorimetric matrix is a poor fit through this DRT (double counting).
        assert!(r.stats_colorimetric.mean > 10.0 * a.stats_refined.as_ref().unwrap().mean.max(1e-4));
        assert_eq!(a.settings.bracket_stops, 1.0, "through mode defaults to a ±1 stop bracket");
    }

    #[test]
    fn recovers_white_shifting_matrix_in_preserve_mode() {
        let m_true = look_matrix();
        let a = run_through(m_true, WhiteMode::PreserveLut);
        let err = (a.m_xyz - m_true).abs().max();
        assert!(err < 1e-5, "{err:e}\n{}", a.m_xyz);
    }

    #[test]
    fn lut_equal_to_reference_gives_identity_and_warns() {
        let a = run_through(Mat3::identity(), WhiteMode::D65);
        assert!((a.m_xyz - Mat3::identity()).abs().max() < 1e-6);
        assert!(a.diagnostics.iter().any(|d| d.code == "small_gain"), "{:#?}", a.diagnostics);
    }

    #[test]
    fn dctl_header_names_the_reference() {
        use lutmatrix_core::dctl::{generate_dctl, ExportMeta};
        let a = run_through(with_d65_fixed(look_matrix()), WhiteMode::D65);
        let meta = ExportMeta::from_analysis(&a, "x.cube", "2026-10-16");
        let src = generate_dctl(&a.m_xyz, &meta);
        assert!(src.contains("// Fitted through: test DRT"));
        assert!(src.contains("USE THIS DCTL DIRECTLY BEFORE THAT TRANSFORM"));
        assert!(!src.contains("before the output transform."));
    }
}
