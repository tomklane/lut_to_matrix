//! `lutmatrix` command-line front end.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lutmatrix_core::detect::{detect, Confidence, Guess};
use lutmatrix_core::lut::Lut;

#[derive(Parser)]
#[command(name = "lutmatrix", version = lutmatrix_core::VERSION, about = "Extract a 3x3 XYZ matrix from a camera-log → display LUT")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Parse LUT file(s) and report the detected input/output spaces.
    Inspect {
        files: Vec<PathBuf>,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
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
    }
}
