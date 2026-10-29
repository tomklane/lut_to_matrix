//! `lutmatrix` command-line front end.

mod chart;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use lutmatrix_core::analysis::{analyze_with, Analysis, Reference, Severity};
use lutmatrix_core::dctl;
use lutmatrix_core::detect::{detect, Confidence, Detection, Guess};
use lutmatrix_core::difference::Metric;
use lutmatrix_core::gray_axis::{NeutralGray, DIFFUSE_WHITE_STOPS};
use lutmatrix_core::lut::Lut;
use lutmatrix_core::math::{Cat, Mat3};
use lutmatrix_core::opendrt_match::{match_opendrt, MatchSettings, OpenDrtMatch};
use lutmatrix_core::preset_store::{import_file, PresetStore, StoredPreset};
use lutmatrix_core::sample::{GrayAnchor, Settings, WhiteMode};
use lutmatrix_core::spaces::{CameraSpaceId, DisplayGamut, DisplayTransfer, InputSpace, OutputSpace};
use lutmatrix_opendrt::writer::lut_match_dctl;

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
    /// Match OpenDRT to LUTs, keep the results in your local preset library, and build
    /// OpenDRT DCTLs with them as "LUT Match" look presets.
    Opendrt(OdrtArgs),
    /// Stack every negative LUT with every print LUT and match OpenDRT to each pair.
    /// Pairs are named `negative_print` from film stocks: the negative's stock number
    /// (+ pull/push) from its file name, the print's from your stock-names table.
    Batch(BatchArgs),
    /// Find a colour chart in a still, fit a correction to it (DWG/DI DCTL), and
    /// optionally an OpenDRT look preset.
    Chart(chart::ChartArgs),
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

#[derive(Clone, Copy, ValueEnum)]
enum NeutralArg {
    Off,
    MiddleGray,
    EveryLevel,
    WhiteGain,
}

impl From<NeutralArg> for NeutralGray {
    fn from(a: NeutralArg) -> Self {
        match a {
            NeutralArg::Off => NeutralGray::Off,
            NeutralArg::MiddleGray => NeutralGray::MiddleGray,
            NeutralArg::EveryLevel => NeutralGray::Exposure,
            NeutralArg::WhiteGain => NeutralGray::WhiteGain,
        }
    }
}

/// The LUT's input and output spaces (detected unless overridden).
#[derive(Args)]
struct SpaceOpts {
    /// Override input space id, e.g. arri_log_c3_awg3, arri_log_c4_awg4, aces2065_ap0.
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
}

impl SpaceOpts {
    fn resolve(&self, lut: &Lut) -> Result<(InputSpace, OutputSpace, Detection), String> {
        let d = detect(lut);
        let mut input = d.input.value;
        if let Some(id) = &self.input {
            input = InputSpace::new(parse_id::<CameraSpaceId>(id, "input space")?);
        }
        if let Some(ei) = self.ei {
            input.arri_ei = ei;
        }
        let mut output = d.output_space();
        if let Some(g) = &self.gamut {
            output.gamut = parse_id::<DisplayGamut>(g, "gamut")?;
        }
        if let Some(t) = &self.transfer {
            output.transfer = parse_transfer(t, None)?;
        }
        if let (Some(p), DisplayTransfer::Pq { peak_nits }) = (self.peak, &mut output.transfer) {
            *peak_nits = p;
        }
        Ok((input, output, d))
    }
}

#[derive(Args)]
struct BatchArgs {
    /// LUTs of any stage: each is measured and sorted into negatives, prints or full LUTs.
    #[arg(num_args = 0..)]
    luts: Vec<PathBuf>,
    /// Negative LUTs (camera log → film negative), whatever they measure as.
    #[arg(long = "negative", short = 'n', num_args = 1..)]
    negatives: Vec<PathBuf>,
    /// Print LUTs (film negative → display), whatever they measure as.
    #[arg(long = "print", short = 'p', num_args = 1..)]
    prints: Vec<PathBuf>,
    /// Full LUTs (camera log → display): matched on their own, not stacked.
    #[arg(long = "full", short = 'f', num_args = 1..)]
    fulls: Vec<PathBuf>,
    /// Stock-names table (default: stock-names.json in your data folder).
    #[arg(long)]
    names: Option<PathBuf>,
    /// Override the negatives' camera space, e.g. arri_log_c4_awg4 (default: detected).
    #[arg(long)]
    input: Option<String>,
    /// ARRI LogC3 exposure index for the negatives.
    #[arg(long)]
    ei: Option<u16>,
    /// Override the prints' display gamut: rec709, p3_d65, p3_dci, rec2020.
    #[arg(long)]
    gamut: Option<String>,
    /// Override the prints' display transfer: 2.2, 2.4, 2.6 or pq.
    #[arg(long)]
    transfer: Option<String>,
    /// Only name and check the pairs (no fitting).
    #[arg(long)]
    check_only: bool,
    /// Fit pairs whose check failed too (default: skip them).
    #[arg(long)]
    fit_mismatched: bool,
    /// quick, standard or thorough.
    #[arg(long, default_value = "standard")]
    effort: String,
    /// See `opendrt fit --neutral-gray`.
    #[arg(long, value_enum, default_value_t = NeutralArg::Off)]
    neutral_gray: NeutralArg,
    #[arg(long, default_value_t = 1.0)]
    neutral_amount: f64,
    #[arg(long, default_value_t = DIFFUSE_WHITE_STOPS)]
    white_stops: f64,
    /// Keep brilliance at the start preset.
    #[arg(long)]
    freeze_brilliance: bool,
    /// Pull towards the start preset (0 = off).
    #[arg(long, default_value_t = 0.0)]
    reg: f64,
    /// Folder for batch.csv and one JSON report per pair.
    #[arg(long, short, default_value = "lutmatrix-batch")]
    out: PathBuf,
    /// Save every fitted pair to your preset library (named like the pair).
    #[arg(long)]
    save: bool,
    /// With --save: replace presets of the same name.
    #[arg(long)]
    replace: bool,
    /// Preset library folder (default: your per-user app data folder).
    #[arg(long)]
    store: Option<PathBuf>,
}

#[derive(Args)]
struct OdrtArgs {
    /// Preset library folder (default: your per-user app data folder; see `presets path`).
    #[arg(long, global = true, value_name = "DIR")]
    store: Option<PathBuf>,
    #[command(subcommand)]
    cmd: OdrtCmd,
}

#[derive(Subcommand)]
enum OdrtCmd {
    /// Fit OpenDRT's look parameters to a LUT and save the result in your preset library.
    Fit {
        file: PathBuf,
        #[command(flatten)]
        spaces: SpaceOpts,
        /// Preset name (default: "LUT Match <LUT name>").
        #[arg(long)]
        name: Option<String>,
        /// Overwrite a preset of the same name.
        #[arg(long)]
        replace: bool,
        /// Do not save to the library (report only).
        #[arg(long)]
        no_save: bool,
        /// Keep these parameters at the start preset (name prefixes, comma separated;
        /// "brl" = brilliance and post brilliance, "grey" = grey level).
        #[arg(long, value_delimiter = ',')]
        freeze: Vec<String>,
        /// Pull towards the start preset (0 = off).
        #[arg(long, default_value_t = 0.0)]
        reg: f64,
        /// Optimiser iterations per start.
        #[arg(long, default_value_t = 150)]
        iterations: usize,
        /// Number of preset starts to refine.
        #[arg(long, default_value_t = 3)]
        starts: usize,
        /// Do not refit with brilliance frozen when luminance falls with exposure.
        #[arg(long)]
        no_auto_freeze: bool,
        /// Remove the LUT's gray-axis cast before matching: "every-level" white-balances
        /// each luminance level to D65, "middle-gray" applies the 18% gray's balance to
        /// everything, "white-gain" balances the LUT's white with one linear gain after the
        /// LUT (like a Gain node; see --white-stops).
        #[arg(long, value_enum, default_value_t = NeutralArg::Off)]
        neutral_gray: NeutralArg,
        /// Strength of --neutral-gray, 0–1 (0.5 keeps half the cast).
        #[arg(long, default_value_t = 1.0)]
        neutral_amount: f64,
        /// White for --neutral-gray white-gain, stops over 18% gray (default 2.32 = diffuse
        /// white, a 90% reflector).
        #[arg(long, default_value_t = DIFFUSE_WHITE_STOPS)]
        white_stops: f64,
        /// Also write the full fit report to this file.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Manage your local preset library.
    #[command(subcommand)]
    Presets(PresetCmd),
    /// Write OpenDRT (the bundled v1.1.0.1 DCTL) with presets from your library added as
    /// Look Presets, plus a P3-D65 2.6 power Display Encoding Preset.
    Build {
        /// Presets to include, in menu order (default: all, oldest first).
        #[arg(long = "preset", value_name = "NAME")]
        presets: Vec<String>,
        /// Output DCTL (default: OpenDRT_v1.1.0.1_LUTmatch.dctl in the current folder).
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Extra line for the DCTL header.
        #[arg(long, default_value = "")]
        note: String,
    },
}

#[derive(Subcommand)]
enum PresetCmd {
    /// List the presets in your library.
    List,
    /// Print where the library lives.
    Path,
    /// Show one preset's fit summary.
    Show { name: String },
    /// Delete a preset.
    Remove { name: String },
    /// Add presets from files (a saved preset, an `opendrt fit --out` report, or an
    /// older Python-tool fit).
    Import {
        files: Vec<PathBuf>,
        /// Name for each file, in order (default: the name in the file or "LUT Match <LUT>").
        #[arg(long = "name")]
        names: Vec<String>,
        /// Overwrite presets of the same name.
        #[arg(long)]
        replace: bool,
    },
    /// Copy a preset out of the library to a file.
    Export {
        name: String,
        #[arg(long, short)]
        out: PathBuf,
    },
}

#[derive(Args)]
struct FitOpts {
    #[command(flatten)]
    spaces: SpaceOpts,
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
    let Some(path) = &o.through else {
        return Ok(None);
    };
    let lut = Lut::load(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let input = InputSpace::new(parse_id::<CameraSpaceId>(&o.through_input, "input space")?);
    let mut out = lut_output;
    if let Some(g) = &o.through_gamut {
        out.gamut = parse_id::<DisplayGamut>(g, "gamut")?;
    }
    if let Some(t) = &o.through_transfer {
        out.transfer = parse_transfer(t, o.spaces.peak)?;
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
    let (input, output, d) = o.spaces.resolve(lut)?;
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
        a.samples_skipped.iter().map(|(s, n)| format!(", {n} {}", s.label())).collect::<String>()
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
            s.metric.label(),
            s.mean,
            s.rms,
            s.p95,
            s.max,
            s.neutral,
            s.neutral_duv
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
        s.worst_hues.iter().map(|h| format!("{} {:.0}° ({:.2})", h.name, h.hue_deg, h.mean)).collect::<Vec<_>>().join(", ")
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

fn print_match(m: &OpenDrtMatch) {
    println!("{}", m.lut_name.as_deref().unwrap_or("?"));
    println!("  {:<15} {}", "Input", m.input.name());
    println!("  {:<15} {}", "Output", m.output.name());
    println!("  {:<15} {} of {} inside the LUT's input range", "Samples", m.samples_used, m.samples_total);
    println!("  {:<15} {} (weighted {:.2})", "Start", m.start.description, m.start.score);
    println!("  {:<15} {} mean / p95 / max   gray axis", "Error", m.metric.label());
    for (name, s) in [("stock", &m.stock), ("best preset", &m.best_preset), ("fitted", &m.fitted)] {
        println!("  {:<15}   {name:<12} {:.2} / {:.2} / {:.2}   {:.2}", "", s.mean, s.p95, s.max, s.gray_mean);
    }
    println!("  {:<15} (stock = Standard look, ACES-2.0 tonescale, 10 nits grey)", "");
    println!("  {:<15} {:.2} nits (Display Grey Luminance)", "Grey", m.config.grey_nits);
    if !m.frozen.is_empty() {
        println!("  {:<15} {}", "Frozen", m.frozen.join(", "));
    }
    if !m.gray_axis.is_empty() {
        let fix = match m.neutral_gray {
            NeutralGray::Off => "neutralize gray off".to_string(),
            g => format!("neutralize gray {}, {:.0}%", g.label(), 100.0 * m.neutral_amount),
        };
        println!("  {:<15} LUT's cast from D65 by exposure, Δu′v′ ({fix})", "Gray axis");
        let row = |f: &dyn Fn(&lutmatrix_core::gray_axis::GrayPoint) -> String| m.gray_axis.iter().map(f).collect::<Vec<_>>().join(" ");
        println!("  {:<15}   stops {}", "", row(&|p| format!("{:>6}", format!("{:+}", p.stops))));
        println!("  {:<15}   Δu′v′ {}", "", row(&|p| format!("{:>6.4}", p.duv)));
        println!("  {:<15}   cast  {}", "", row(&|p| format!("{:>6}", short_cast(p.cast()))));
    }
    let s = &m.safety;
    println!(
        "  {:<15} {} (finite/in range: {}, reversals: {}, roughness {:.4} vs stock {:.4})",
        "Safety",
        if s.ok() { "ok" } else { "CHECK" },
        s.finite_in_range,
        s.reversals.len(),
        s.roughness,
        s.stock_roughness
    );
    for r in s.reversals.iter().take(5) {
        println!("  {:<15}   {}: luminance falls {:.1}% at {:+.2} stops", "", r.colour, r.drop_percent, r.at_stops);
    }
    for n in &m.notes {
        println!("  {:<15} {n}", "Note");
    }
    println!("  {:<15} {:.0} s", "Time", m.seconds);
}

fn short_cast(c: &str) -> &'static str {
    match c {
        "neutral" => "-",
        "red-orange" => "R/O",
        "yellow" => "Y",
        "yellow-green" => "Y/G",
        "green" => "G",
        "cyan" => "C",
        "blue" => "B",
        "magenta" => "M",
        _ => "R/M",
    }
}

fn run_batch(a: BatchArgs) -> Result<(), String> {
    use lutmatrix_core::batch::{self, BatchOptions, Full, Negative, Print};
    use lutmatrix_core::film::{dedupe, measure_stage, Stage, StockNames};
    use std::sync::atomic::AtomicBool;

    let names_path = a.names.clone().or_else(StockNames::default_path);
    let names = match &names_path {
        Some(p) => StockNames::load(p)?,
        None => StockNames::default(),
    };
    let spaces = SpaceOpts { input: a.input.clone(), ei: a.ei, gamut: a.gamut.clone(), transfer: a.transfer.clone(), peak: None };
    let file = |p: &PathBuf| p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // Every LUT is measured; given lists keep the stage they were given as.
    let mut loaded: Vec<(PathBuf, Stage, Lut)> = Vec::new();
    for (list, forced) in
        [(&a.luts, None), (&a.negatives, Some(Stage::Negative)), (&a.prints, Some(Stage::Print)), (&a.fulls, Some(Stage::Full))]
    {
        for p in list {
            let lut = Lut::load(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let m = measure_stage(&lut);
            let stage = match forced {
                Some(f) => {
                    if f != m.stage {
                        eprintln!("note: {} measures as {} ({}); using it as given", file(p), stage_word(m.stage), m.reason);
                    }
                    f
                }
                None => {
                    println!("{:<52} {:<8} {}", file(p), stage_word(m.stage), m.reason);
                    m.stage
                }
            };
            loaded.push((p.clone(), stage, lut));
        }
    }
    // Names per list, repeats lettered a, b, c….
    let mut lists: [Vec<String>; 3] = Default::default();
    for (p, stage, _) in &loaded {
        lists[*stage as usize].push(names.name_for(*stage, &file(p)));
    }
    lists.iter_mut().for_each(|l| dedupe(l));
    let mut lists = lists.map(|l| l.into_iter());
    let (mut negatives, mut prints, mut fulls) = (Vec::new(), Vec::new(), Vec::new());
    for (p, stage, lut) in loaded {
        let name = lists[stage as usize].next().expect("one name per LUT");
        let (input, output, _) = spaces.resolve(&lut)?;
        match stage {
            Stage::Negative => negatives.push(Negative { name, file: file(&p), lut, input }),
            Stage::Print => prints.push(Print { name, file: file(&p), lut, output }),
            Stage::Full => fulls.push(Full { name, file: file(&p), lut, input, output }),
        }
    }
    if negatives.len() * prints.len() == 0 && fulls.is_empty() {
        return Err(format!(
            "nothing to run: {} negative(s), {} print(s), {} full LUT(s); a pair needs at least one negative and one print",
            negatives.len(),
            prints.len(),
            fulls.len()
        ));
    }
    let (iterations, starts) = match a.effort.as_str() {
        "quick" => (40, 2),
        "standard" => (150, 3),
        "thorough" => (300, 4),
        e => return Err(format!("--effort must be quick, standard or thorough (got '{e}')")),
    };
    let settings = MatchSettings {
        max_iterations: iterations,
        starts,
        regularization: a.reg,
        freeze: if a.freeze_brilliance { vec!["brl".into()] } else { Vec::new() },
        auto_freeze_brilliance: !a.freeze_brilliance,
        neutral_gray: a.neutral_gray.into(),
        neutral_amount: a.neutral_amount,
        white_stops: a.white_stops,
        ..MatchSettings::default()
    };
    let store = if a.save {
        Some(match &a.store {
            Some(d) => PresetStore::at(d),
            None => PresetStore::open_default().map_err(|e| e.to_string())?,
        })
    } else {
        None
    };
    std::fs::create_dir_all(&a.out).map_err(|e| format!("{}: {e}", a.out.display()))?;
    let opts = BatchOptions { settings, fit_mismatched: a.fit_mismatched, check_only: a.check_only };
    println!(
        "{} negative(s) × {} print(s) = {} pairs, + {} full LUT(s){}",
        negatives.len(),
        prints.len(),
        negatives.len() * prints.len(),
        fulls.len(),
        if a.check_only { " (check only)" } else { "" }
    );
    let cancel = AtomicBool::new(false);
    let out = a.out.clone();
    let mut write_err = None;
    let results = batch::run(&negatives, &prints, &fulls, &opts, &cancel, |done, total, r| {
        let what = match (&r.result, &r.error) {
            (Some(m), _) => format!(
                "{} {:.2} → {:.2} (gray {:.2}), grey {:.2} nits{}",
                m.metric.label(),
                m.stock.mean,
                m.fitted.mean,
                m.fitted.gray_mean,
                m.config.grey_nits,
                if m.safety.ok() { "" } else { ", CHECK safety" }
            ),
            (None, Some(e)) => e.clone(),
            (None, None) => format!(
                "18% gray → {:.2} nits{}",
                r.check.gray_nits,
                if r.check.ok { String::new() } else { format!("  MISMATCH: {}", r.check.problems.join("; ")) }
            ),
        };
        println!("[{done}/{total}] {:<44} {what}", r.name);
        let path = out.join(format!("{}.json", r.name.replace(['/', '\\', ':'], "-")));
        if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(r).unwrap() + "\n") {
            write_err.get_or_insert(format!("{}: {e}", path.display()));
        }
        if let (Some(store), Some(m)) = (&store, &r.result) {
            match StoredPreset::from_match(m, Some(&r.name))
                .map_err(|e| e.to_string())
                .and_then(|p| store.save(&p, a.replace).map_err(|e| e.to_string()))
            {
                Ok(_) => {}
                Err(e) => eprintln!("  not saved: {e}"),
            }
        }
    });
    if let Some(e) = write_err {
        return Err(e);
    }
    let csv_path = a.out.join("batch.csv");
    std::fs::write(&csv_path, batch::csv(&results)).map_err(|e| format!("{}: {e}", csv_path.display()))?;
    println!("wrote {} and {} pair report(s) to {}", csv_path.display(), results.len(), a.out.display());
    Ok(())
}

fn stage_word(s: lutmatrix_core::film::Stage) -> &'static str {
    match s {
        lutmatrix_core::film::Stage::Negative => "negative",
        lutmatrix_core::film::Stage::Print => "print",
        lutmatrix_core::film::Stage::Full => "full",
    }
}

fn run_opendrt(args: OdrtArgs) -> Result<(), String> {
    let store = match args.store {
        Some(d) => PresetStore::at(d),
        None => PresetStore::open_default().map_err(|e| e.to_string())?,
    };
    match args.cmd {
        OdrtCmd::Fit {
            file,
            spaces,
            name,
            replace,
            no_save,
            freeze,
            reg,
            iterations,
            starts,
            no_auto_freeze,
            neutral_gray,
            neutral_amount,
            white_stops,
            out,
            json,
        } => {
            if !(0.0..=1.0).contains(&neutral_amount) {
                return Err("--neutral-amount must be between 0 and 1".into());
            }
            let lut = Lut::load(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let (input, output, _) = spaces.resolve(&lut)?;
            let preset_name = name.clone().unwrap_or_else(|| {
                let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                format!("LUT Match {stem}")
            });
            if !no_save && !replace && store.get(&preset_name).is_ok() {
                return Err(format!("a preset called '{preset_name}' is already in your library; use --replace or --name"));
            }
            let settings = MatchSettings {
                freeze,
                regularization: reg,
                max_iterations: iterations,
                starts,
                auto_freeze_brilliance: !no_auto_freeze,
                neutral_gray: neutral_gray.into(),
                neutral_amount,
                white_stops,
                ..MatchSettings::default()
            };
            let m = match_opendrt(&lut, lut.filename.clone(), input, output, &settings).map_err(|e| e.to_string())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&m).unwrap());
            } else {
                print_match(&m);
            }
            if let Some(path) = out {
                let body = serde_json::to_string_pretty(&m).unwrap() + "\n";
                std::fs::write(&path, body).map_err(|e| format!("could not write {}: {e}", path.display()))?;
                eprintln!("wrote {}", path.display());
            }
            if !no_save {
                let p = StoredPreset::from_match(&m, Some(&preset_name)).map_err(|e| e.to_string())?;
                let path = store.save(&p, replace).map_err(|e| e.to_string())?;
                eprintln!("saved preset '{}' to {}", p.name, path.display());
            }
            Ok(())
        }
        OdrtCmd::Presets(cmd) => run_presets(&store, cmd),
        OdrtCmd::Build { presets, out, note } => {
            let (all, bad) = store.list().map_err(|e| e.to_string())?;
            for e in bad {
                eprintln!("skipped: {e}");
            }
            let chosen: Vec<StoredPreset> = if presets.is_empty() {
                all
            } else {
                presets.iter().map(|n| store.get(n).map_err(|e| e.to_string())).collect::<Result<_, _>>()?
            };
            if chosen.is_empty() {
                return Err(format!("your preset library ({}) is empty; run `lutmatrix opendrt fit` first", store.dir().display()));
            }
            let writer: Vec<_> = chosen.iter().map(|p| p.to_writer()).collect();
            let dctl = lut_match_dctl(&writer, &note).map_err(|e| e.to_string())?;
            let path = out.unwrap_or_else(|| PathBuf::from("OpenDRT_v1.1.0.1_LUTmatch.dctl"));
            std::fs::write(&path, dctl).map_err(|e| format!("could not write {}: {e}", path.display()))?;
            println!("wrote {} with {} LUT Match preset(s):", path.display(), chosen.len());
            for p in &chosen {
                println!("  {:<28} grey {:>5.2} nits  ({})", p.name, p.grey_nits, p.source_lut);
            }
            Ok(())
        }
    }
}

fn run_presets(store: &PresetStore, cmd: PresetCmd) -> Result<(), String> {
    match cmd {
        PresetCmd::Path => println!("{}", store.dir().display()),
        PresetCmd::List => {
            let (all, bad) = store.list().map_err(|e| e.to_string())?;
            if all.is_empty() {
                println!("no presets in {}", store.dir().display());
            }
            for p in &all {
                let err = p.report.as_ref().map(|r| format!("  {} {:.2}", r.metric.label(), r.fitted.mean)).unwrap_or_default();
                println!("{:<28} {}  grey {:>5.2} nits{err}  ({})", p.name, p.created_date(), p.grey_nits, p.source_lut);
            }
            for e in bad {
                eprintln!("skipped: {e}");
            }
        }
        PresetCmd::Show { name } => {
            let p = store.get(&name).map_err(|e| e.to_string())?;
            println!("{}  (saved {}, grey {:.2} nits)", p.name, p.created_date(), p.grey_nits);
            match &p.report {
                Some(m) => print_match(m),
                None => println!("  imported from {} (no fit report)", p.source_lut),
            }
        }
        PresetCmd::Remove { name } => {
            store.remove(&name).map_err(|e| e.to_string())?;
            println!("removed '{name}'");
        }
        PresetCmd::Import { files, names, replace } => {
            if names.len() > files.len() {
                return Err("more --name values than files".into());
            }
            for (i, f) in files.iter().enumerate() {
                let p = import_file(f, names.get(i).map(|s| s.as_str())).map_err(|e| e.to_string())?;
                let path = store.save(&p, replace).map_err(|e| e.to_string())?;
                println!("imported '{}' → {}", p.name, path.display());
            }
        }
        PresetCmd::Export { name, out } => {
            let p = store.get(&name).map_err(|e| e.to_string())?;
            let body = serde_json::to_string_pretty(&p).unwrap() + "\n";
            std::fs::write(&out, body).map_err(|e| format!("could not write {}: {e}", out.display()))?;
            println!("wrote {}", out.display());
        }
    }
    Ok(())
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
    println!("  {label:<15} {value}   [{} confidence, {:.0}%]", conf(g.confidence), g.certainty * 100.0);
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
                        let stage = lutmatrix_core::film::measure_stage(&lut);
                        if json {
                            out.push(serde_json::json!({
                                "file": f.display().to_string(),
                                "lut": lut.describe(),
                                "title": lut.title,
                                "detection": d,
                                "stage": stage,
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
                        println!("  {:<15} {} ({})", "Stage", stage_word(stage.stage), stage.reason);
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
        Cmd::Batch(args) => match run_batch(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Cmd::Chart(args) => match chart::run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Cmd::Opendrt(args) => match run_opendrt(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
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
