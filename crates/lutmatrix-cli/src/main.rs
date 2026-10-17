//! `lutmatrix` command-line front end.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use lutmatrix_core::analysis::{analyze_with, Analysis, Reference, Severity};
use lutmatrix_core::dctl;
use lutmatrix_core::detect::{detect, Confidence, Guess};
use lutmatrix_core::difference::Metric;
use lutmatrix_core::lut::Lut;
use lutmatrix_core::math::{Cat, Mat3};
use lutmatrix_core::sample::{GrayAnchor, Settings, WhiteMode};
use lutmatrix_core::spaces::{CameraSpaceId, DisplayGamut, DisplayTransfer, InputSpace, OutputSpace};

#[derive(Parser)]
#[command(name = "lutmatrix", version = lutmatrix_core::VERSION, about = "Extract a 3x3 XYZ matrix from a camera-log → display LUT")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

// Parsed once at start-up; the size difference between variants is irrelevant.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Cmd {
    /// Parse LUT file(s) and report the detected input/output spaces.
    Inspect {
        files: Vec<PathBuf>,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Fit the matrix and write a DCTL plus JSON and text matrix files.
    Export {
        file: PathBuf,
        #[command(flatten)]
        opts: FitOpts,
        /// Output folder (default: next to the LUT).
        #[arg(long, short)]
        out_dir: Option<PathBuf>,
    },
    /// Measure the LUT around middle gray and fit the 3x3 XYZ matrix.
    Fit {
        file: PathBuf,
        #[command(flatten)]
        opts: FitOpts,
        /// Print the full analysis as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args)]
struct FitOpts {
    /// Override input space id, e.g. arri_log_c3_awg3, arri_log_c4_awg4, sony_s_log3_s_gamut3_cine.
    #[arg(long)]
    input: Option<String>,
    /// ARRI LogC3 exposure index.
    #[arg(long)]
    ei: Option<u16>,
    /// Override output gamut: rec709, p3_d65, p3_dci, rec2020.
    #[arg(long)]
    gamut: Option<String>,
    /// Override output transfer: 2.2, 2.4, 2.6 or pq.
    #[arg(long)]
    transfer: Option<String>,
    /// PQ peak luminance (cd/m²).
    #[arg(long)]
    peak: Option<f64>,
    /// White mode: d65 (default) or preserve.
    #[arg(long, default_value = "d65")]
    white: String,
    /// Chromatic adaptation for D65 mode: bradford (default) or cat02.
    #[arg(long, default_value = "bradford")]
    cat: String,
    /// Middle-gray anchor: scene (A, default) or display (B).
    #[arg(long, default_value = "scene")]
    anchor: String,
    /// Display middle gray in cd/m² (default: BT.2408-based or the LUT's DW hint).
    #[arg(long)]
    mid_gray_nits: Option<f64>,
    /// Exposure bracket ±stops (0 = off).
    #[arg(long, default_value_t = 0.0)]
    bracket: f64,
    /// Number of hues in the sample ring.
    #[arg(long, default_value_t = 36)]
    hues: usize,
    /// Disable exposure normalisation.
    #[arg(long)]
    no_normalize: bool,
    /// Least squares only (no perceptual refinement).
    #[arg(long)]
    ls_only: bool,
    /// Error metric: auto (default), de2000 or itp.
    #[arg(long, default_value = "auto")]
    metric: String,
    /// Preserve mode: constrain the matrix to reproduce the LUT's white exactly.
    #[arg(long)]
    lock_neutral: bool,
    /// Fit THROUGH this display transform: a LUT of the node that follows the DCTL
    /// (e.g. a CST or OpenDRT exported from Resolve as a 65-point .cube).
    #[arg(long, value_name = "LUT")]
    through: Option<PathBuf>,
    /// Input encoding of the --through LUT (default: da_vinci_intermediate_dwg).
    #[arg(long, default_value = "da_vinci_intermediate_dwg")]
    through_input: String,
    /// Output gamut of the --through LUT (default: same as the LUT being matched).
    #[arg(long)]
    through_gamut: Option<String>,
    /// Output transfer of the --through LUT: 2.2, 2.4, 2.6 or pq (default: same as the LUT).
    #[arg(long)]
    through_transfer: Option<String>,
}

fn parse_transfer(t: &str, peak: Option<f64>) -> Result<DisplayTransfer, String> {
    Ok(match t.to_ascii_lowercase().as_str() {
        "pq" | "st2084" => DisplayTransfer::Pq { peak_nits: peak.unwrap_or(1000.0) },
        g => DisplayTransfer::Gamma { gamma: g.parse().map_err(|_| format!("bad transfer '{t}'"))? },
    })
}

/// Load the --through reference (if any). Its output space defaults to the LUT's.
fn load_reference(o: &FitOpts, lut_output: OutputSpace) -> Result<Option<(Lut, InputSpace, OutputSpace, String)>, String> {
    let Some(path) = &o.through else { return Ok(None) };
    let lut = Lut::load(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let input = InputSpace::new(parse_id::<CameraSpaceId>(&o.through_input, "input space")?);
    let mut out = lut_output;
    if let Some(g) = &o.through_gamut {
        out.gamut = parse_id::<DisplayGamut>(g, "gamut")?;
    }
    if let Some(t) = &o.through_transfer {
        out.transfer = parse_transfer(t, o.peak)?;
    }
    let name = lut.filename.clone().unwrap_or_else(|| path.display().to_string());
    Ok(Some((lut, input, out, name)))
}

fn run_analysis(lut: &Lut, o: &FitOpts) -> Result<Analysis, String> {
    let (input, output, settings, hint) = build(lut, o)?;
    let reference = load_reference(o, output)?;
    let r = reference.as_ref().map(|(l, i, out, name)| Reference { transform: l, name: name.clone(), input: *i, output: *out });
    analyze_with(lut, input, output, &settings, hint, r.as_ref()).map_err(|e| e.to_string())
}

fn parse_id<T: serde::de::DeserializeOwned>(s: &str, what: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|_| format!("unknown {what} '{s}'"))
}

fn build(lut: &Lut, o: &FitOpts) -> Result<(InputSpace, OutputSpace, Settings, Option<f64>), String> {
    let d = detect(lut);
    let mut input = d.input.value;
    if let Some(id) = &o.input {
        input = InputSpace::new(parse_id::<CameraSpaceId>(id, "input space")?);
    }
    if let Some(ei) = o.ei {
        input.arri_ei = ei;
    }
    let mut output = d.output_space();
    if let Some(g) = &o.gamut {
        output.gamut = parse_id::<DisplayGamut>(g, "gamut")?;
    }
    if let Some(t) = &o.transfer {
        output.transfer = parse_transfer(t, None)?;
    }
    if let (Some(p), DisplayTransfer::Pq { peak_nits }) = (o.peak, &mut output.transfer) {
        *peak_nits = p;
    }
    let s = Settings {
        white_mode: match o.white.as_str() {
            "d65" => WhiteMode::D65,
            "preserve" => WhiteMode::PreserveLut,
            w => return Err(format!("bad white mode '{w}'")),
        },
        cat: match o.cat.as_str() {
            "bradford" => Cat::Bradford,
            "cat02" => Cat::Cat02,
            c => return Err(format!("bad CAT '{c}'")),
        },
        anchor: match o.anchor.as_str() {
            "scene" | "a" => GrayAnchor::SceneLinear,
            "display" | "b" => GrayAnchor::DisplayTarget,
            a => return Err(format!("bad anchor '{a}'")),
        },
        display_middle_gray_nits: o.mid_gray_nits,
        bracket_stops: o.bracket,
        hues: o.hues,
        normalize_exposure: !o.no_normalize,
        refine: !o.ls_only,
        lock_neutral: o.lock_neutral,
        metric: match o.metric.as_str() {
            "auto" => Metric::Auto,
            "de2000" => Metric::DeltaE2000,
            "itp" => Metric::DeltaEItp,
            m => return Err(format!("bad metric '{m}'")),
        },
        ..Settings::default()
    };
    Ok((input, output, s, d.middle_gray_hint.map(|h| h.0)))
}

fn print_matrix(name: &str, m: &Mat3) {
    println!("  {name}");
    for r in 0..3 {
        println!("    [{:>13.10} {:>13.10} {:>13.10}]", m[(r, 0)], m[(r, 1)], m[(r, 2)]);
    }
}

fn print_analysis(file: &str, a: &Analysis) {
    println!("{file}");
    println!("  Input           {}", a.input_name);
    println!("  Output          {}", a.output_name);
    println!(
        "  White mode      {}",
        match a.settings.white_mode {
            WhiteMode::PreserveLut if a.settings.lock_neutral => "preserve LUT white (neutral locked)".to_string(),
            WhiteMode::PreserveLut => "preserve LUT white".to_string(),
            WhiteMode::D65 => format!("D65 ({:?} CAT + neutral constraint)", a.settings.cat),
        }
    );
    println!(
        "  Samples         {} used{}",
        a.samples_used,
        a.samples_skipped
            .iter()
            .map(|(s, n)| format!(", {n} {}", s.label()))
            .collect::<String>()
    );
    if let Some(r) = &a.reference {
        println!("  Fitted through  {} ({} → {})", r.name, r.input_name, r.output_name);
    }
    let ls_name = if a.reference.is_some() { "M_XYZ (colorimetric least squares, for comparison)" } else { "M_XYZ (least squares)" };
    print_matrix(ls_name, &a.m_xyz_ls);
    if let Some(m) = &a.m_xyz_refined {
        let what = if a.reference.is_some() { "fitted through reference" } else { "refined" };
        print_matrix(&format!("M_XYZ ({what}, min {})", a.metric.label()), m);
    }
    let row = |name: &str, s: &lutmatrix_core::analysis::FitStats| {
        println!(
            "  {name:<15} {}: mean {:.3}  rms {:.3}  p95 {:.3}  max {:.3}  | neutral {:.3}, Δu′v′ {:.5}",
            s.metric.label(), s.mean, s.rms, s.p95, s.max, s.neutral, s.neutral_duv
        );
    };
    if let Some(r) = &a.reference {
        row("Ref alone", &r.stats_alone);
        row("Colorimetric", &r.stats_colorimetric);
    } else {
        row("Error (LS)", &a.stats_ls);
    }
    if let Some(s) = &a.stats_refined {
        row("Error (refined)", s);
    }
    let s = a.stats_refined.as_ref().unwrap_or(&a.stats_ls);
    println!(
        "  Worst hues      {}",
        s.worst_hues
            .iter()
            .map(|h| format!("{} {:.0}° ({:.2})", h.name, h.hue_deg, h.mean))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for d in &a.diagnostics {
        let tag = match d.severity {
            Severity::Info => "Info",
            Severity::Warning => "WARNING",
        };
        println!("  {tag:<15} {}", d.message);
    }
    println!();
}

fn conf(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
        Confidence::None => "none",
    }
}

fn line<T>(label: &str, value: String, g: &Guess<T>) {
    println!(
        "  {label:<15} {value}   [{} confidence, {:.0}%]",
        conf(g.confidence),
        g.certainty * 100.0
    );
    for r in &g.reasons {
        println!("  {:<15}   · {r}", "");
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Inspect { files, json } => {
            let mut ok = true;
            let mut out = Vec::new();
            for f in files {
                match Lut::load(&f) {
                    Ok(lut) => {
                        let d = detect(&lut);
                        if json {
                            out.push(serde_json::json!({
                                "file": f.display().to_string(),
                                "lut": lut.describe(),
                                "title": lut.title,
                                "detection": d,
                            }));
                            continue;
                        }
                        println!("{}", lut.filename.as_deref().unwrap_or("?"));
                        println!("  {:<15} {}", "LUT", lut.describe());
                        if let Some(t) = &lut.title {
                            println!("  {:<15} {t}", "Title");
                        }
                        let (lo, hi) = lut.output_extent();
                        println!("  {:<15} {lo:.4} … {hi:.4}", "Output range");
                        line("Input", d.input.value.name(), &d.input);
                        for (alt, s) in d.input.alternatives.iter().take(3) {
                            println!("  {:<15}   alt: {} (score {s:.1})", "", alt.name());
                        }
                        line("Output gamut", d.output_gamut.value.name().into(), &d.output_gamut);
                        line("Output EOTF", d.output_transfer.value.name(), &d.output_transfer);
                        if let Some(p) = &d.probe {
                            println!(
                                "  {:<15} 18% gray: in {:.4} → out ({:.4}, {:.4}, {:.4}) = {:.2} nits; display middle gray {:.2} nits ({:+.2} stops)",
                                "Gray probe",
                                p.input_code,
                                p.output_code[0],
                                p.output_code[1],
                                p.output_code[2],
                                p.output_nits,
                                p.expected_nits,
                                p.stops_from_expected
                            );
                        }
                        if let Some((_, why)) = &d.middle_gray_hint {
                            println!("  {:<15} {why}", "Middle gray");
                        }
                        for n in &d.notes {
                            println!("  {:<15} {n}", "Note");
                        }
                        println!();
                    }
                    Err(e) => {
                        ok = false;
                        eprintln!("{}: {e}", f.display());
                    }
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            }
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Cmd::Export { file, opts, out_dir } => {
            let lut = match Lut::load(&file) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}: {e}", file.display());
                    return ExitCode::FAILURE;
                }
            };
            let a = match run_analysis(&lut, &opts) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("{}: {e}", file.display());
                    return ExitCode::FAILURE;
                }
            };
            let name = lut.filename.clone().unwrap_or_else(|| "LUT.cube".into());
            let meta = dctl::ExportMeta::from_analysis(&a, &name, &dctl::today_utc());
            let dir = out_dir.unwrap_or_else(|| file.parent().map(|p| p.to_path_buf()).unwrap_or_default());
            let dctl_path = dir.join(dctl::dctl_file_name(&name));
            let json_path = dctl_path.with_extension("json");
            let txt_path = dctl_path.with_extension("txt");
            let writes = [
                (&dctl_path, dctl::generate_dctl(&a.m_xyz, &meta)),
                (&json_path, dctl::export_json(&a, &meta)),
                (&txt_path, dctl::export_text(&a, &meta)),
            ];
            for (p, body) in writes {
                if let Err(e) = std::fs::write(p, body) {
                    eprintln!("could not write {}: {e}", p.display());
                    return ExitCode::FAILURE;
                }
                println!("wrote {}", p.display());
            }
            println!("{}", meta.fit_error);
            ExitCode::SUCCESS
        }
        Cmd::Fit { file, opts, json } => {
            let lut = match Lut::load(&file) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}: {e}", file.display());
                    return ExitCode::FAILURE;
                }
            };
            match run_analysis(&lut, &opts) {
                Ok(a) if json => {
                    println!("{}", serde_json::to_string_pretty(&a).unwrap());
                    ExitCode::SUCCESS
                }
                Ok(a) => {
                    print_analysis(lut.filename.as_deref().unwrap_or("?"), &a);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}: {e}", file.display());
                    ExitCode::FAILURE
                }
            }
        }
    }
}
