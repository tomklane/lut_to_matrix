//! The OpenDRT `transform()` function, ported line by line from the DCTL in `f32`.
//!
//! Expressions keep the DCTL's operation order (left-to-right, no reassociation, no fused
//! multiply-add) and its library calls (`powf(2, x)` stays a `powf`, `_exp10f` is
//! `powf(10, x)`), so results match a C/CUDA build of the DCTL to rounding.
//! Literals are copied digit for digit. Comments in quotes are from the DCTL.
#![allow(clippy::excessive_precision, clippy::approx_constant)]

use std::f32::consts::PI;
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Sub};

use crate::params::{Config, DisplayGamut, Eotf, InputGamut, InputOetf, LookParams};
use crate::tables::*;
use crate::M3;

const SQRT3: f32 = 1.73205080756887729353;

/// The DCTL's `float3`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct F3 {
    x: f32,
    y: f32,
    z: f32,
}

const fn f3(x: f32, y: f32, z: f32) -> F3 {
    F3 { x, y, z }
}

impl F3 {
    fn map(self, f: impl Fn(f32) -> f32) -> F3 {
        f3(f(self.x), f(self.y), f(self.z))
    }
}
impl Add for F3 {
    type Output = F3;
    fn add(self, b: F3) -> F3 {
        f3(self.x + b.x, self.y + b.y, self.z + b.z)
    }
}
impl Add<f32> for F3 {
    type Output = F3;
    fn add(self, b: f32) -> F3 {
        f3(self.x + b, self.y + b, self.z + b)
    }
}
impl Sub<f32> for F3 {
    type Output = F3;
    fn sub(self, b: f32) -> F3 {
        f3(self.x - b, self.y - b, self.z - b)
    }
}
impl Mul<f32> for F3 {
    type Output = F3;
    fn mul(self, b: f32) -> F3 {
        f3(self.x * b, self.y * b, self.z * b)
    }
}
impl Div<f32> for F3 {
    type Output = F3;
    fn div(self, b: f32) -> F3 {
        f3(self.x / b, self.y / b, self.z / b)
    }
}
impl AddAssign for F3 {
    fn add_assign(&mut self, b: F3) {
        *self = *self + b;
    }
}
impl AddAssign<f32> for F3 {
    fn add_assign(&mut self, b: f32) {
        *self = *self + b;
    }
}
impl MulAssign<f32> for F3 {
    fn mul_assign(&mut self, b: f32) {
        *self = *self * b;
    }
}

/// `vdot`: matrix × vector.
fn vdot(m: &M3, v: F3) -> F3 {
    f3(
        m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    )
}

const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn sdivf(a: f32, b: f32) -> f32 {
    if b == 0.0 {
        0.0
    } else {
        a / b
    }
}

fn spowf(a: f32, b: f32) -> f32 {
    if a <= 0.0 {
        a
    } else {
        a.powf(b)
    }
}

/// `_exp10f`.
fn exp10f(x: f32) -> f32 {
    10.0f32.powf(x)
}

fn hypotf2(x: f32, y: f32) -> f32 {
    (x * x + y * y).max(0.0).sqrt()
}

fn hypotf3(v: F3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).max(0.0).sqrt()
}

fn clampf(a: f32, mn: f32, mx: f32) -> f32 {
    a.max(mn).min(mx)
}

/* OETF linearisation */

fn oetf_davinci_intermediate(x: f32) -> f32 {
    if x <= 0.02740668 {
        x / 10.44426855
    } else {
        (x / 0.07329248 - 7.0).exp2() - 0.0075
    }
}
fn oetf_filmlight_tlog(x: f32) -> f32 {
    if x < 0.075 {
        (x - 0.075) / 16.184376489665897
    } else {
        ((x - 0.5520126568606655) / 0.09232902596577353).exp() - 0.0057048244042473785
    }
}
fn oetf_acescct(x: f32) -> f32 {
    if x <= 0.155251141552511 {
        (x - 0.0729055341958355) / 10.5402377416545
    } else {
        (x * 17.52 - 9.72).exp2()
    }
}
fn oetf_arri_logc3(x: f32) -> f32 {
    if x < 5.367655 * 0.010591 + 0.092809 {
        (x - 0.092809) / 5.367655
    } else {
        (exp10f((x - 0.385537) / 0.247190) - 0.052272) / 5.555556
    }
}
fn oetf_arri_logc4(x: f32) -> f32 {
    if x < -0.7774983977293537 {
        x * 0.3033266726886969 - 0.7774983977293537
    } else {
        ((14.0 * (x - 0.09286412512218964) / 0.9071358748778103 + 6.0).exp2() - 64.0) / 2231.8263090676883
    }
}
fn oetf_red_log3g10(x: f32) -> f32 {
    if x < 0.0 {
        (x / 15.1927) - 0.01
    } else {
        (exp10f(x / 0.224282) - 1.0) / 155.975327 - 0.01
    }
}
fn oetf_panasonic_vlog(x: f32) -> f32 {
    if x < 0.181 {
        (x - 0.125) / 5.6
    } else {
        exp10f((x - 0.598206) / 0.241514) - 0.00873
    }
}
fn oetf_sony_slog3(x: f32) -> f32 {
    if x < 171.2102946929 / 1023.0 {
        (x * 1023.0 - 95.0) * 0.01125 / (171.2102946929 - 95.0)
    } else {
        exp10f((x * 1023.0 - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
    }
}
fn oetf_fujifilm_flog2(x: f32) -> f32 {
    if x < 0.100686685370811 {
        (x - 0.092864) / 8.799461
    } else {
        exp10f((x - 0.384316) / 0.245281) / 5.555556 - 0.064829 / 5.555556
    }
}

fn linearize(rgb: F3, tf: InputOetf) -> F3 {
    match tf {
        InputOetf::Linear => rgb,
        InputOetf::DaVinciIntermediate => rgb.map(oetf_davinci_intermediate),
        InputOetf::FilmlightTLog => rgb.map(oetf_filmlight_tlog),
        InputOetf::AcesCct => rgb.map(oetf_acescct),
        InputOetf::ArriLogC3 => rgb.map(oetf_arri_logc3),
        InputOetf::ArriLogC4 => rgb.map(oetf_arri_logc4),
        InputOetf::RedLog3G10 => rgb.map(oetf_red_log3g10),
        InputOetf::PanasonicVLog => rgb.map(oetf_panasonic_vlog),
        InputOetf::SonySLog3 => rgb.map(oetf_sony_slog3),
        InputOetf::FujiFLog2 => rgb.map(oetf_fujifilm_flog2),
    }
}

/* Inverse display EOTFs */

fn eotf_hlg_inverse(rgb: F3) -> F3 {
    // "Apply the HLG Forward or Inverse EOTF for 1000 nits." (inverse branch)
    let yd = 0.2627 * rgb.x + 0.6780 * rgb.y + 0.0593 * rgb.z;
    let rgb = rgb * spowf(yd, (1.0 - 1.2) / 1.2);
    rgb.map(|v| {
        if v <= 1.0 / 12.0 {
            (3.0 * v).sqrt()
        } else {
            0.17883277 * (12.0 * v - 0.28466892).ln() + 0.55991073
        }
    })
}

fn eotf_pq_inverse(rgb: F3) -> F3 {
    let m1 = 2610.0f32 / 16384.0;
    let m2 = 2523.0f32 / 32.0;
    let c1 = 107.0f32 / 128.0;
    let c2 = 2413.0f32 / 128.0;
    let c3 = 2392.0f32 / 128.0;
    let rgb = rgb.map(|v| spowf(v, m1));
    rgb.map(|v| spowf((c1 + c2 * v) / (1.0 + c3 * v), m2))
}

/* Functions for OpenDRT */

fn compress_hyperbolic_power(x: f32, s: f32, p: f32) -> f32 {
    spowf(x / (x + s), p)
}

fn compress_toe_quadratic(x: f32, toe: f32, inv: bool) -> f32 {
    if toe == 0.0 {
        return x;
    }
    if !inv {
        spowf(x, 2.0) / (x + toe)
    } else {
        (x + (x * (4.0 * toe + x)).sqrt()) / 2.0
    }
}

fn compress_toe_cubic(x: f32, m: f32, w: f32, inv: bool) -> f32 {
    if m == 1.0 {
        return x;
    }
    let x2 = x * x;
    if !inv {
        x * (x2 + m * w) / (x2 + w)
    } else {
        let p0 = x2 - 3.0 * m * w;
        let p1 = 2.0 * x2 + 27.0 * w - 9.0 * m * w;
        let p2 = ((x2 * p1 * p1 - 4.0 * p0 * p0 * p0).sqrt() / 2.0 + x * p1 / 2.0).powf(1.0 / 3.0);
        p0 / (3.0 * p2) + p2 / 3.0 + x / 3.0
    }
}

/// "High exposure adjustment with linear extension" (forward only).
fn contrast_high(x: f32, p: f32, pv: f32, pv_lx: f32) -> f32 {
    let x0 = 0.18 * 2.0f32.powf(pv);
    if x < x0 || p == 1.0 {
        return x;
    }
    let o = x0 - x0 / p;
    let s0 = x0.powf(1.0 - p) / p;
    let x1 = x0 * 2.0f32.powf(pv_lx);
    let k1 = p * s0 * x1.powf(p) / x1;
    let y1 = s0 * x1.powf(p) + o;
    if x > x1 {
        k1 * (x - x1) + y1
    } else {
        s0 * x.powf(p) + o
    }
}

fn softplus(x: f32, s: f32) -> f32 {
    if x > 10.0 * s || s < 1e-4 {
        return x;
    }
    s * (1.0 + (x / s).exp()).max(0.0).ln()
}

fn gauss_window(x: f32, w: f32) -> f32 {
    (-x * x / w).exp()
}

/// "Simple Cyan-Yellow / Green-Magenta opponent space".
fn opponent(rgb: F3) -> (f32, f32) {
    (rgb.x - rgb.z, rgb.y - (rgb.x + rgb.z) / 2.0)
}

fn hue_offset(h: f32, o: f32) -> f32 {
    // `2.0*PI` is a double in the DCTL; doubling is exact, so 2·PI in f32 is identical.
    (h - o + PI) % (2.0 * PI) - PI
}

/// Display gamut and creative whitepoint conversion (`display_gamut_whitepoint`).
fn display_gamut_whitepoint(rgb: F3, tsn: f32, cwp_lm: f32, display_gamut: DisplayGamut, cwp: i32) -> F3 {
    use DisplayGamut as G;
    // "First, convert from P3D65 to XYZ D65"
    let mut rgb = vdot(&MATRIX_P3D65_TO_XYZ, rgb);
    let mut cwp_neutral = rgb;
    let cwp_f = tsn.powf(2.0 * cwp_lm);

    match display_gamut {
        G::Rec709 | G::P3D65 | G::Rec2020P3Limited => match cwp {
            0 => rgb = vdot(&MATRIX_CAT_D65_TO_D93, rgb),
            1 => rgb = vdot(&MATRIX_CAT_D65_TO_D75, rgb),
            3 => rgb = vdot(&MATRIX_CAT_D65_TO_D60, rgb),
            4 => rgb = vdot(&MATRIX_CAT_D65_TO_D55, rgb),
            5 => rgb = vdot(&MATRIX_CAT_D65_TO_D50, rgb),
            _ => {}
        },
        G::P3D60 => match cwp {
            0 => rgb = vdot(&MATRIX_CAT_D60_TO_D93, rgb),
            1 => rgb = vdot(&MATRIX_CAT_D60_TO_D75, rgb),
            2 => rgb = vdot(&MATRIX_CAT_D60_TO_D65, rgb),
            4 => rgb = vdot(&MATRIX_CAT_D60_TO_D55, rgb),
            5 => rgb = vdot(&MATRIX_CAT_D60_TO_D50, rgb),
            _ => {}
        },
        G::P3Dci | G::Xyz => {
            // "Keep "Neutral" axis as D65, don't want green midtones in P3-DCI container."
            cwp_neutral = vdot(&MATRIX_CAT_DCI_TO_D65, rgb);
            match cwp {
                0 => rgb = vdot(&MATRIX_CAT_DCI_TO_D93, rgb),
                1 => rgb = vdot(&MATRIX_CAT_DCI_TO_D75, rgb),
                2 => rgb = cwp_neutral,
                3 => rgb = vdot(&MATRIX_CAT_DCI_TO_D60, rgb),
                4 => rgb = vdot(&MATRIX_CAT_DCI_TO_D55, rgb),
                5 => rgb = vdot(&MATRIX_CAT_DCI_TO_D50, rgb),
                _ => {}
            }
        }
    }

    // "Mix between Creative Whitepoint and "neutral" axis with Creative White Range control."
    rgb = rgb * cwp_f + cwp_neutral * (1.0 - cwp_f);

    rgb = match display_gamut {
        G::Rec709 => vdot(&MATRIX_XYZ_TO_REC709, rgb),
        G::Xyz => vdot(&MATRIX_CAT_D65_TO_DCI, rgb),
        _ => vdot(&MATRIX_XYZ_TO_P3D65, rgb),
    };

    // "Post creative whitepoint normalization so that peak luminance does not exceed display maximum."
        let cwp_norm: f32 = match (display_gamut, cwp) {
        (G::Rec709, 0) => 0.744192699063,
        (G::Rec709, 1) => 0.873470832146,
        (G::Rec709, 3) => 0.955936992163,
        (G::Rec709, 4) => 0.905671332781,
        (G::Rec709, 5) => 0.850004385027,
        (G::P3D65 | G::Rec2020P3Limited, 0) => 0.762687057298,
        (G::P3D65 | G::Rec2020P3Limited, 1) => 0.884054083328,
        (G::P3D65 | G::Rec2020P3Limited, 3) => 0.964320186739,
        (G::P3D65 | G::Rec2020P3Limited, 4) => 0.923076518860,
        (G::P3D65 | G::Rec2020P3Limited, 5) => 0.876572837784,
        (G::P3D60, 0) => 0.704956321013,
        (G::P3D60, 1) => 0.816715709816,
        (G::P3D60, 2) => 0.923382193663,
        (G::P3D60, 4) => 0.956138500287,
        (G::P3D60, 5) => 0.906801453023,
        (G::P3Dci, 0) => 0.665336141225,
        (G::P3Dci, 1) => 0.770397131382,
        (G::P3Dci, 2) => 0.870572343302,
        (G::P3Dci, 3) => 0.891354547503,
        (G::P3Dci, 4) => 0.855327825187,
        (G::P3Dci, 5) => 0.814566436117,
        (G::Xyz, 0) => 0.707142784007,
        (G::Xyz, 1) => 0.815561082617,
        (G::Xyz, c) if c >= 2 => 0.916555279740,
        _ => 1.0,
    };
    // "only normalize values affected by range control"
    rgb *= cwp_norm * cwp_f + 1.0 - cwp_f;
    rgb
}

/// OpenDRT for one [`Config`], with the per-frame constants precomputed.
#[derive(Debug, Clone)]
pub struct OpenDrt {
    cfg: Config,
    p: LookParams,
    cwp: i32,
    in_to_xyz: M3,
    ts_x0: f32,
    ts_s: f32,
    ts_p: f32,
    ts_m2: f32,
    ts_dsc: f32,
    s_lp100: f32,
    ts_s1: f32,
}

fn on(v: f32) -> bool {
    v != 0.0
}

impl OpenDrt {
    pub fn new(cfg: &Config) -> OpenDrt {
        let p = cfg.look;
        let in_to_xyz = match cfg.in_gamut {
            InputGamut::Xyz => IDENTITY,
            InputGamut::Aces2065_1 => MATRIX_AP0_TO_XYZ,
            InputGamut::AcesCg => MATRIX_AP1_TO_XYZ,
            InputGamut::P3D65 => MATRIX_P3D65_TO_XYZ,
            InputGamut::Rec2020 => MATRIX_REC2020_TO_XYZ,
            InputGamut::Rec709 => MATRIX_REC709_TO_XYZ,
            InputGamut::ArriWideGamut3 => MATRIX_ARRIWG3_TO_XYZ,
            InputGamut::ArriWideGamut4 => MATRIX_ARRIWG4_TO_XYZ,
            InputGamut::RedWideGamutRgb => MATRIX_REDWG_TO_XYZ,
            InputGamut::SGamut3 => MATRIX_SONYSGAMUT3_TO_XYZ,
            InputGamut::SGamut3Cine => MATRIX_SONYSGAMUT3CINE_TO_XYZ,
            InputGamut::VGamut => MATRIX_VGAMUT_TO_XYZ,
            InputGamut::EGamut => MATRIX_EGAMUT_TO_XYZ,
            InputGamut::EGamut2 => MATRIX_EGAMUT2_TO_XYZ,
            InputGamut::DaVinciWideGamut => MATRIX_DAVINCIWG_TO_XYZ,
        };

        // "Tonescale Constraint Calculations" https://www.desmos.com/calculator/1c4fhzy3bw
        let tn_lp = cfg.peak_nits;
        let tn_lg = cfg.grey_nits * cfg.grey_scale;
        let ts_x1 = 2.0f32.powf(6.0 * p.tn_sh + 4.0);
        let ts_y1 = tn_lp / 100.0;
        let ts_x0 = 0.18 + p.tn_off;
        let ts_y0 = tn_lg / 100.0 * (1.0 + cfg.hdr_grey_boost * ts_y1.log2());
        let ts_s0 = compress_toe_quadratic(ts_y0, p.tn_toe, true);
        let ts_p = p.tn_con / (1.0 + cfg.surround as f32 * 0.05);
        let ts_s10 = ts_x0 * (ts_s0.powf(-1.0 / p.tn_con) - 1.0);
        let ts_m1 = ts_y1 / (ts_x1 / (ts_x1 + ts_s10)).powf(p.tn_con);
        let ts_m2 = compress_toe_quadratic(ts_m1, p.tn_toe, true);
        let ts_s = ts_x0 * ((ts_s0 / ts_m2).powf(-1.0 / p.tn_con) - 1.0);
        // A double ternary in the DCTL, converted to float.
        let ts_dsc = match cfg.eotf {
            Eotf::Pq => 0.01f64 as f32,
            Eotf::Hlg => 0.1f64 as f32,
            _ => 100.0 / tn_lp,
        };
        // "Lerp from pt_cmp at 100 nits to pt_cmp_hdr at 1000 nits"
        let pt_cmp_lf = cfg.hdr_purity * 1.0f32.min((tn_lp - 100.0) / 900.0);
        // "Approximate scene-linear scale at Lp=100 nits"
        let s_lp100 = ts_x0 * ((tn_lg / 100.0).powf(-1.0 / p.tn_con) - 1.0);
        let ts_s1 = ts_s * pt_cmp_lf + s_lp100 * (1.0 - pt_cmp_lf);

        OpenDrt {
            cfg: *cfg,
            p,
            cwp: p.cwp.round() as i32,
            in_to_xyz,
            ts_x0,
            ts_s,
            ts_p,
            ts_m2,
            ts_dsc,
            s_lp100,
            ts_s1,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Transform one pixel (input encoded per `config().in_oetf` / `in_gamut`) to display
    /// code values.
    pub fn apply(&self, input: [f32; 3]) -> [f32; 3] {
        let p = &self.p;
        let cfg = &self.cfg;
        let mut rgb = f3(input[0], input[1], input[2]);

        // "Linearize if a non-linear input oetf / transfer function is selected"
        rgb = linearize(rgb, cfg.in_oetf);

        // "Convert from input gamut into P3-D65"
        rgb = vdot(&self.in_to_xyz, rgb);
        rgb = vdot(&MATRIX_XYZ_TO_P3D65, rgb);

        // "Rendering Space: "Desaturate" to control scale of the color volume in the rgb ratios."
        let rs_w = f3(p.rs_rw, 1.0 - p.rs_rw - p.rs_bw, p.rs_bw);
        let mut sat_l = rgb.x * rs_w.x + rgb.y * rs_w.y + rgb.z * rs_w.z;
        rgb = f3(sat_l * p.rs_sa, sat_l * p.rs_sa, sat_l * p.rs_sa) + rgb * (1.0 - p.rs_sa);

        // "Offset"
        rgb += p.tn_off;

        // "Tonescale Norm"
        let mut tsn = hypotf3(rgb) / SQRT3;
        // "RGB Ratios"
        rgb = rgb.map(|v| sdivf(v, tsn));

        let opp = opponent(rgb);
        let mut ach_d = hypotf2(opp.0, opp.1) / 2.0;
        // "Smooth ach_d, normalized so 1.0 doesn't change"
        ach_d = 1.25 * compress_toe_quadratic(ach_d, 0.25, false);

        // "Hue angle, rotated so that red = 0.0"
        let hue = (opp.0.atan2(opp.1) + PI + 1.10714931) % (2.0 * PI);

        // "RGB Hue Angles"
        let ha_rgb = f3(
            gauss_window(hue_offset(hue, 0.1), 0.66),
            gauss_window(hue_offset(hue, 4.3), 0.66),
            gauss_window(hue_offset(hue, 2.3), 0.66),
        );
        // "RGB Hue Angles for hue shift: red shifted more orange"
        let ha_rgb_hs = f3(gauss_window(hue_offset(hue, -0.4), 0.66), ha_rgb.y, gauss_window(hue_offset(hue, 2.5), 0.66));
        // "CMY Hue Angles"
        let ha_cmy = f3(
            gauss_window(hue_offset(hue, 3.3), 0.5),
            gauss_window(hue_offset(hue, 1.3), 0.5),
            gauss_window(hue_offset(hue, -1.15), 0.5),
        );

        // "Brilliance"
        if on(p.brl_enable) {
            let brl_tsf = (tsn / (tsn + 1.0)).powf(1.0 - p.brl_rng);
            let brl_exf = (p.brl + p.brl_r * ha_rgb.x + p.brl_g * ha_rgb.y + p.brl_b * ha_rgb.z) * ach_d.powf(1.0 / p.brl_st);
            let brl_ex = 2.0f32.powf(brl_exf * (if brl_exf < 0.0 { brl_tsf } else { 1.0 - brl_tsf }));
            tsn *= brl_ex;
        }

        // "Contrast Low"
        if on(p.tn_lcon_enable) {
            let lcon_m = 2.0f32.powf(-p.tn_lcon);
            let mut lcon_w = p.tn_lcon_w / 4.0;
            lcon_w *= lcon_w;
            // "Normalize for ts_x0 intersection constraint"
            let lcon_cnst_sc = compress_toe_cubic(self.ts_x0, lcon_m, lcon_w, true) / self.ts_x0;
            tsn *= lcon_cnst_sc;
            tsn = compress_toe_cubic(tsn, lcon_m, lcon_w, false);
        }

        // "Contrast High"
        if on(p.tn_hcon_enable) {
            let hcon_p = 2.0f32.powf(p.tn_hcon);
            tsn = contrast_high(tsn, hcon_p, p.tn_hcon_pv, p.tn_hcon_st);
        }

        // "Hyperbolic Compression"
        let tsn_pt = compress_hyperbolic_power(tsn, self.ts_s1, self.ts_p);
        let tsn_const = compress_hyperbolic_power(tsn, self.s_lp100, self.ts_p);
        tsn = compress_hyperbolic_power(tsn, self.ts_s, self.ts_p);

        // "Hue Contrast R"
        if on(p.hc_enable) {
            let mut hc_ts = 1.0 - tsn_const;
            let mut hc_c = hc_ts * (1.0 - ach_d) + ach_d * (1.0 - hc_ts);
            hc_c *= ach_d * ha_rgb.x;
            hc_ts = hc_ts.powf(1.0 / p.hc_r_rng);
            let hc_f = p.hc_r * (hc_c - 2.0 * hc_c * hc_ts) + 1.0;
            rgb = f3(rgb.x, rgb.y * hc_f, rgb.z * hc_f);
        }

        // "Hue Shift RGB by purity compress tonescale, shifting more as intensity increases"
        if on(p.hs_rgb_enable) {
            let hs_rgb = f3(
                ha_rgb_hs.x * ach_d * tsn_pt.powf(1.0 / p.hs_r_rng),
                ha_rgb_hs.y * ach_d * tsn_pt.powf(1.0 / p.hs_g_rng),
                ha_rgb_hs.z * ach_d * tsn_pt.powf(1.0 / p.hs_b_rng),
            );
            let hsf = f3(hs_rgb.x * p.hs_r, hs_rgb.y * -p.hs_g, hs_rgb.z * -p.hs_b);
            let hsf = f3(hsf.z - hsf.y, hsf.x - hsf.z, hsf.y - hsf.x);
            rgb += hsf;
        }

        // "Hue Shift CMY by tonescale, shifting less as intensity increases"
        if on(p.hs_cmy_enable) {
            let tsn_pt_compl = 1.0 - tsn_pt;
            let hs_cmy = f3(
                ha_cmy.x * ach_d * tsn_pt_compl.powf(1.0 / p.hs_c_rng),
                ha_cmy.y * ach_d * tsn_pt_compl.powf(1.0 / p.hs_m_rng),
                ha_cmy.z * ach_d * tsn_pt_compl.powf(1.0 / p.hs_y_rng),
            );
            let hsf = f3(hs_cmy.x * -p.hs_c, hs_cmy.y * p.hs_m, hs_cmy.z * p.hs_y);
            let hsf = f3(hsf.z - hsf.y, hsf.x - hsf.z, hsf.y - hsf.x);
            rgb += hsf;
        }

        // "Purity Limit Low" (applied whatever pt_enable says, as in the DCTL)
        let pt_lml_p = 1.0
            + 4.0 * (1.0 - tsn_pt) * (p.pt_lml + p.pt_lml_r * ha_rgb_hs.x + p.pt_lml_g * ha_rgb_hs.y + p.pt_lml_b * ha_rgb_hs.z);
        let mut ptf = 1.0 - tsn_pt.powf(pt_lml_p);

        // "Purity Limit High"
        let pt_lmh_p = (1.0 - ach_d * (p.pt_lmh_r * ha_rgb_hs.x + p.pt_lmh_b * ha_rgb_hs.z)) * (1.0 - p.pt_lmh * ach_d);
        ptf = ptf.powf(pt_lmh_p);

        // "Mid-Range Purity"
        if on(p.ptm_enable) {
            let ptm_low_f = if p.ptm_low_st == 0.0 || p.ptm_low_rng == 0.0 {
                1.0
            } else {
                1.0 + p.ptm_low * (-2.0 * ach_d * ach_d / p.ptm_low_st).exp() * (1.0 - tsn_const).powf(1.0 / p.ptm_low_rng)
            };
            let ptm_high_f = if p.ptm_high_st == 0.0 || p.ptm_high_rng == 0.0 {
                1.0
            } else {
                1.0 + p.ptm_high
                    * (-2.0 * ach_d * ach_d / p.ptm_high_st).exp()
                    * tsn_pt.powf(1.0 / (4.0 * p.ptm_high_rng))
            };
            ptf *= ptm_low_f * ptm_high_f;
        }

        // "Lerp to peak achromatic by ptf in rgb ratios"
        rgb = rgb * ptf + 1.0 - ptf;

        // "Inverse Rendering Space"
        sat_l = rgb.x * rs_w.x + rgb.y * rs_w.y + rgb.z * rs_w.z;
        rgb = f3(sat_l * p.rs_sa - rgb.x, sat_l * p.rs_sa - rgb.y, sat_l * p.rs_sa - rgb.z) / (p.rs_sa - 1.0);

        // "Convert to final display gamut and set whitepoint"
        rgb = display_gamut_whitepoint(rgb, tsn_const, p.cwp_lm, cfg.display_gamut, self.cwp);

        // "Post Brilliance"
        if on(p.brlp_enable) {
            let brlp_opp = opponent(rgb);
            let mut brlp_ach_d = hypotf2(brlp_opp.0, brlp_opp.1) / 4.0;
            brlp_ach_d = 1.1 * (brlp_ach_d * brlp_ach_d / (brlp_ach_d + 0.1));
            let brlp_ha_rgb = ha_rgb * ach_d;
            let brlp_m = p.brlp + p.brlp_r * brlp_ha_rgb.x + p.brlp_g * brlp_ha_rgb.y + p.brlp_b * brlp_ha_rgb.z;
            let brlp_ex = 2.0f32.powf(brlp_m * brlp_ach_d * tsn);
            rgb *= brlp_ex;
        }

        // "Purity Compress Low"
        if on(p.ptl_enable) {
            rgb = f3(softplus(rgb.x, p.ptl_c), softplus(rgb.y, p.ptl_m), softplus(rgb.z, p.ptl_y));
        }

        // "Final tonescale adjustments"
        tsn *= self.ts_m2; // "scale for inverse toe"
        tsn = compress_toe_quadratic(tsn, p.tn_toe, false);
        tsn *= self.ts_dsc; // "scale for display encoding"

        // "Return from RGB ratios"
        rgb *= tsn;

        // "Rec.2020 (P3 Limited)"
        if cfg.display_gamut == DisplayGamut::Rec2020P3Limited {
            rgb = rgb.map(|v| v.max(0.0)); // "Limit to P3 gamut"
            rgb = vdot(&MATRIX_P3_TO_REC2020, rgb);
        }

        if cfg.clamp {
            rgb = rgb.map(|v| clampf(v, 0.0, 1.0));
        }

        // "Apply inverse Display EOTF"
        let eotf = cfg.eotf.index() as i32;
        let eotf_p = 2.0 + eotf as f32 * 0.2;
        rgb = match cfg.eotf {
            Eotf::Power22 | Eotf::Power24 | Eotf::Power26 => rgb.map(|v| spowf(v, 1.0 / eotf_p)),
            Eotf::Pq => eotf_pq_inverse(rgb),
            Eotf::Hlg => eotf_hlg_inverse(rgb),
            Eotf::Linear => rgb,
        };
        [rgb.x, rgb.y, rgb.z]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Presets};

    #[test]
    fn neutral_stays_neutral_and_grey_lands_on_grey_nits() {
        // DWG/DI 18% grey through the Standard look at Rec.1886: 10 nits of 100.
        let drt = OpenDrt::new(&Config::default());
        let di_gray = 0.336_043_7; // DaVinci Intermediate code of 0.18
        let o = drt.apply([di_gray; 3]);
        assert!((o[0] - o[1]).abs() < 1e-6 && (o[1] - o[2]).abs() < 1e-6, "{o:?}");
        let nits = 100.0 * o[1].powf(2.4);
        assert!((nits - 10.0).abs() < 0.05, "grey at {nits} nits");
        let _ = Presets::default();
    }
}
