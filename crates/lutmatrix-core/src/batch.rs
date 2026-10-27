//! Batch OpenDRT matching of every negative × every print (DECISIONS.md D36).
//!
//! Each pair is stacked ([`Stack`]: negative, then print, as two LUT nodes), checked for a
//! plausible pairing, and matched with [`match_opendrt`] using the shared settings. The
//! pair is named `negative_print` from film stocks only ([`crate::film`]). Full LUTs
//! (camera log → display) are matched on their own, not stacked (D37). Result names
//! that would repeat get a, b, c….

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::film::{dedupe, pair_name, Stack};
use crate::lut::{Interpolation, Lut, Transform};
use crate::math::whites;
use crate::opendrt_match::{match_opendrt, MatchSettings, OpenDrtMatch};
use crate::spaces::{InputSpace, OutputSpace};

/// A negative LUT: camera log in, film negative out.
#[derive(Debug, Clone)]
pub struct Negative {
    pub name: String,
    pub file: String,
    pub lut: Lut,
    /// The camera space the negative LUT expects (the stack's input).
    pub input: InputSpace,
}

/// A print LUT: film negative in, display out.
#[derive(Debug, Clone)]
pub struct Print {
    pub name: String,
    pub file: String,
    pub lut: Lut,
    /// The display the print LUT renders to (the stack's output).
    pub output: OutputSpace,
}

/// A full LUT: camera log in, display out; matched on its own.
#[derive(Debug, Clone)]
pub struct Full {
    pub name: String,
    pub file: String,
    pub lut: Lut,
    pub input: InputSpace,
    pub output: OutputSpace,
}

/// 18% gray through the stack lands far from a normal display level: 3–40% of the
/// display's peak (SDR) or 3–60 nits (PQ). Typical film pairs land near 10%.
const GRAY_MIN_FRACTION: f64 = 0.03;
const GRAY_MAX_FRACTION: f64 = 0.40;

/// Is this negative/print pairing plausible?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCheck {
    /// 18% scene gray through negative then print, cd/m².
    pub gray_nits: f64,
    /// The negative's output range over realistic scene colours (gray −8…+8 stops, hues
    /// inside both the camera and the display gamut at −4…+6 stops) and the print's input
    /// range. Colours beyond the display gamut are left out: negatives often push those
    /// below 0, which the print clips, as Resolve does.
    pub negative_out: (f64, f64),
    pub print_in: (f64, f64),
    /// Share of those colours (%) that leave the print's input range (and get clipped).
    pub outside_percent: f64,
    pub ok: bool,
    pub problems: Vec<String>,
}

/// One pair's outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResult {
    pub name: String,
    pub negative: String,
    pub print: String,
    pub negative_file: String,
    pub print_file: String,
    pub check: PairCheck,
    /// The fit, unless skipped or failed.
    pub result: Option<OpenDrtMatch>,
    /// Why there is no fit.
    pub error: Option<String>,
    /// A full LUT matched on its own (`negative` is empty, `print` is its name).
    #[serde(default)]
    pub full: bool,
}

/// Run each LUT of a pair in order.
pub fn stack(negative: &Negative, print: &Print) -> Stack {
    Stack { stages: vec![negative.lut.clone(), print.lut.clone()] }
}

/// Checks a pairing before fitting it.
pub fn check_pair(negative: &Negative, print: &Print, interp: Interpolation) -> PairCheck {
    let s = stack(negative, print);
    let gray = whites::D65.to_xyz() * 0.18;
    let code = negative.input.encode(&(negative.input.xyz_d65_to_rgb() * gray));
    let gray_nits = (print.output.rgb_to_xyz() * print.output.decode(&s.apply_rgb(&code, interp))).y;
    let problems = gray_problem(gray_nits, &print.output).into_iter().collect::<Vec<_>>();
    let (lo, hi) = print.lut.input_domain();
    let print_in = (lo.iter().cloned().fold(f64::INFINITY, f64::min), hi.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
    // Realistic scene colours, not the table's corners (out-of-gamut codes no camera makes).
    let wn = whites::D65.to_xyz();
    let mut scene: Vec<crate::math::Vec3> = (0..=32).map(|k| wn * (0.18 * 2f64.powf(-8.0 + 0.5 * k as f64))).collect();
    let rings = crate::sample::Settings {
        hues: 24,
        saturations: vec![0.5, 0.9],
        ring_limit: crate::sample::RingLimit::CameraAndDisplay,
        ..crate::sample::Settings::default()
    };
    for st in -4..=6 {
        scene.extend(crate::sample::hue_ring(&rings, &negative.input, &print.output, 0.18 * 2f64.powi(st)).into_iter().map(|(_, x)| x));
    }
    let to_cam = negative.input.xyz_d65_to_rgb();
    let mut negative_out = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut used, mut outside) = (0usize, 0usize);
    for x in &scene {
        let cam_code = negative.input.encode(&(to_cam * x));
        if !negative.lut.in_domain(&cam_code) {
            continue;
        }
        let o = negative.lut.apply(&cam_code, interp);
        used += 1;
        negative_out = (negative_out.0.min(o.min()), negative_out.1.max(o.max()));
        if (0..3).any(|c| o[c] < lo[c] - 0.01 || o[c] > hi[c] + 0.01) {
            outside += 1;
        }
    }
    let outside_percent = 100.0 * outside as f64 / used.max(1) as f64;
    let mut problems = problems;
    if outside_percent > 5.0 {
        problems.push(format!(
            "{outside_percent:.0}% of realistic colours leave the print's input range ({:.3}–{:.3}; the negative \
             gives {:.3}–{:.3}) and get clipped",
            print_in.0, print_in.1, negative_out.0, negative_out.1
        ));
    }
    PairCheck { gray_nits, negative_out, print_in, outside_percent, ok: problems.is_empty(), problems }
}

/// A note if 18% gray lands far from a normal display level.
fn gray_problem(gray_nits: f64, output: &OutputSpace) -> Option<String> {
    let peak = match output.transfer {
        crate::spaces::DisplayTransfer::Pq { .. } => 150.0,
        _ => output.peak_nits(),
    };
    (!(GRAY_MIN_FRACTION * peak..=GRAY_MAX_FRACTION * peak).contains(&gray_nits)).then(|| {
        format!(
            "18% gray comes out at {gray_nits:.2} nits (expected about {:.0}–{:.0}); the stages may not share \
             the same in-between encoding, or the input space is wrong",
            GRAY_MIN_FRACTION * peak,
            GRAY_MAX_FRACTION * peak
        )
    })
}

/// The same check for a full LUT on its own (where its 18% gray lands).
pub fn check_full(full: &Full, interp: Interpolation) -> PairCheck {
    let gray = whites::D65.to_xyz() * 0.18;
    let code = full.input.encode(&(full.input.xyz_d65_to_rgb() * gray));
    let gray_nits = (full.output.rgb_to_xyz() * full.output.decode(&full.lut.apply(&code, interp))).y;
    let problems: Vec<String> = gray_problem(gray_nits, &full.output).into_iter().collect();
    PairCheck { gray_nits, negative_out: (0.0, 0.0), print_in: (0.0, 0.0), outside_percent: 0.0, ok: problems.is_empty(), problems }
}

/// Options for [`run`].
#[derive(Debug, Clone, Default)]
pub struct BatchOptions {
    pub settings: MatchSettings,
    /// Fit pairs whose check failed anyway (default: skip them).
    pub fit_mismatched: bool,
    /// Only check the pairs; no fitting.
    pub check_only: bool,
}

/// The result names of a batch, in run order: every negative × every print, then the
/// full LUTs; names that would repeat get a, b, c….
pub fn job_names(negatives: &[Negative], prints: &[Print], fulls: &[Full]) -> Vec<String> {
    let mut names: Vec<String> = negatives.iter().flat_map(|n| prints.iter().map(move |p| pair_name(&n.name, &p.name))).collect();
    names.extend(fulls.iter().map(|f| f.name.clone()));
    dedupe(&mut names);
    names
}

/// Every negative × every print, in that order (all prints for the first negative,
/// then the next), then every full LUT on its own. `on_pair(done, total, result)` is
/// called as each finishes; setting `cancel` stops before the next.
pub fn run(
    negatives: &[Negative],
    prints: &[Print],
    fulls: &[Full],
    options: &BatchOptions,
    cancel: &AtomicBool,
    mut on_pair: impl FnMut(usize, usize, &PairResult),
) -> Vec<PairResult> {
    let names = job_names(negatives, prints, fulls);
    let total = names.len();
    let mut out = Vec::with_capacity(total);
    let mut names = names.into_iter();
    for n in negatives {
        for p in prints {
            if cancel.load(Ordering::Relaxed) {
                return out;
            }
            let name = names.next().expect("one name per job");
            let check = check_pair(n, p, options.settings.interpolation);
            let (result, error) = if options.check_only {
                (None, None)
            } else if !check.ok && !options.fit_mismatched {
                (None, Some(format!("skipped: {}", check.problems.join("; "))))
            } else {
                match match_opendrt(&stack(n, p), Some(name.clone()), n.input, p.output, &options.settings) {
                    Ok(m) => (Some(m), None),
                    Err(e) => (None, Some(e.to_string())),
                }
            };
            let r = PairResult {
                name,
                negative: n.name.clone(),
                print: p.name.clone(),
                negative_file: n.file.clone(),
                print_file: p.file.clone(),
                check,
                result,
                error,
                full: false,
            };
            on_pair(out.len() + 1, total, &r);
            out.push(r);
        }
    }
    for f in fulls {
        if cancel.load(Ordering::Relaxed) {
            return out;
        }
        let name = names.next().expect("one name per job");
        let check = check_full(f, options.settings.interpolation);
        let (result, error) = if options.check_only {
            (None, None)
        } else if !check.ok && !options.fit_mismatched {
            (None, Some(format!("skipped: {}", check.problems.join("; "))))
        } else {
            match match_opendrt(&f.lut, Some(name.clone()), f.input, f.output, &options.settings) {
                Ok(m) => (Some(m), None),
                Err(e) => (None, Some(e.to_string())),
            }
        };
        let r = PairResult {
            name,
            negative: String::new(),
            print: f.name.clone(),
            negative_file: String::new(),
            print_file: f.file.clone(),
            check,
            result,
            error,
            full: true,
        };
        on_pair(out.len() + 1, total, &r);
        out.push(r);
    }
    out
}

/// One CSV row per pair (header first): what a colorist sorts and compares.
pub fn csv(results: &[PairResult]) -> String {
    let mut s = String::from(
        "name,negative,print,gray_nits,check,stock_mean,best_preset_mean,fitted_mean,fitted_p95,fitted_gray,\
         grey_nits,start,safety_ok,neutral_gray,error\n",
    );
    let q = |t: &str| format!("\"{}\"", t.replace('"', "\"\""));
    for r in results {
        let m = r.result.as_ref();
        let f = |v: Option<f64>| v.map_or(String::new(), |v| format!("{v:.3}"));
        s.push_str(&format!(
            "{},{},{},{:.2},{},{},{},{},{},{},{},{},{},{},{}\n",
            q(&r.name),
            q(&r.negative),
            q(&r.print),
            r.check.gray_nits,
            if r.check.ok { "ok" } else { "mismatch" },
            f(m.map(|m| m.stock.mean)),
            f(m.map(|m| m.best_preset.mean)),
            f(m.map(|m| m.fitted.mean)),
            f(m.map(|m| m.fitted.p95)),
            f(m.map(|m| m.fitted.gray_mean)),
            f(m.map(|m| m.config.grey_nits as f64)),
            q(m.map_or("", |m| m.start.description.as_str())),
            m.map_or(String::new(), |m| m.safety.ok().to_string()),
            m.map_or(String::new(), |m| m.neutral_gray.label().to_string()),
            q(r.error.as_deref().unwrap_or("")),
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lut::Lut3D;
    use crate::spaces::curves::di_decode;
    use crate::spaces::{CameraSpaceId, DisplayGamut, DisplayTransfer};

    /// A toy negative (DI in → "density" = DI code) and print (density → Rec.709 2.4
    /// with a filmic curve): together a plausible pair; a print fed a dark negative is
    /// not.
    fn pair(neg_scale: f64) -> (Negative, Print) {
        let input = InputSpace::new(CameraSpaceId::DaVinciIntermediateDwg);
        let out = OutputSpace::new(DisplayGamut::Rec709, DisplayTransfer::Gamma { gamma: 2.4 });
        let neg = Lut::from_3d(Lut3D::from_fn(17, move |c| c * neg_scale));
        let print = Lut::from_3d(Lut3D::from_fn(17, move |c| {
            c.map(|v| {
                let lin = di_decode(v).max(0.0);
                let y = lin / (lin + 0.6);
                (y.min(1.0)).powf(1.0 / 2.4)
            })
        }));
        (
            Negative { name: "5219".into(), file: "neg.cube".into(), lut: neg, input },
            Print { name: "2383v1_Native".into(), file: "print.dat".into(), lut: print, output: out },
        )
    }

    #[test]
    fn checks_and_names_pairs() {
        let (n, p) = pair(1.0);
        let c = check_pair(&n, &p, Interpolation::Tetrahedral);
        assert!(c.ok, "{c:?}");
        let (dark, p2) = pair(0.3);
        let c2 = check_pair(&dark, &p2, Interpolation::Tetrahedral);
        assert!(!c2.ok && c2.problems[0].contains("18% gray"), "{c2:?}");

        let cancel = AtomicBool::new(false);
        let opts = BatchOptions { check_only: true, ..BatchOptions::default() };
        let mut seen = Vec::new();
        let r = run(&[n.clone(), dark], std::slice::from_ref(&p), &[], &opts, &cancel, |d, t, r| seen.push((d, t, r.name.clone())));
        assert_eq!(
            seen,
            vec![(1, 2, "5219_2383v1_Nativea".to_string()), (2, 2, "5219_2383v1_Nativeb".to_string())],
            "same name twice → a, b"
        );
        assert!(r.iter().all(|r| r.result.is_none() && r.error.is_none()));
        assert!(csv(&r).lines().count() == 3);

        // Mismatched pairs are skipped (not fitted) unless asked.
        let opts = BatchOptions::default();
        let (dark, _) = pair(0.3);
        let r = run(&[dark], std::slice::from_ref(&p), &[], &opts, &cancel, |_, _, _| {});
        assert!(r[0].error.as_deref().unwrap().starts_with("skipped"));

        cancel.store(true, Ordering::Relaxed);
        // A full LUT runs on its own, after the pairs, named as given.
        cancel.store(false, Ordering::Relaxed);
        let full = Full {
            name: "FilmLog_2383v1_Native".into(),
            file: "LogC34_x.dat".into(),
            lut: p.lut.clone(),
            input: n.input,
            output: p.output,
        };
        let co = BatchOptions { check_only: true, ..BatchOptions::default() };
        let r = run(std::slice::from_ref(&n), std::slice::from_ref(&p), std::slice::from_ref(&full), &co, &cancel, |_, _, _| {});
        assert_eq!(r.len(), 2);
        assert!(r[1].full && r[1].name == "FilmLog_2383v1_Native" && r[1].negative.is_empty());
        cancel.store(true, Ordering::Relaxed);
        assert!(run(&[n], &[p2], &[], &opts, &cancel, |_, _, _| {}).is_empty(), "cancelled before the first pair");
    }
}
