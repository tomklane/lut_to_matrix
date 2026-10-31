//! `lutmatrix chart`: find a colour chart in a still and fit a correction to it.

use std::path::{Path, PathBuf};

use clap::Args;
use lutmatrix_core::chart::detect::{find_lattice, Orientation};
use lutmatrix_core::chart::encoding::{curve_from_id, curve_id, StillGamut, StillSpace};
use lutmatrix_core::chart::fit::{Exposure, Model};
use lutmatrix_core::chart::image::ChartImage;
use lutmatrix_core::chart::odrt::{fit_opendrt_chart, OdrtChartSettings};
use lutmatrix_core::chart::reference::ChartEdition;
use lutmatrix_core::chart::{analyze, preview_jpeg, ChartAnalysis, ChartSettings};
use lutmatrix_core::dctl::today_utc;
use lutmatrix_core::detect::Confidence;
use lutmatrix_core::preset_store::{PresetStore, StoredPreset};

#[derive(Args)]
pub struct ChartArgs {
    /// The still with the chart (TIFF, OpenEXR, PNG, JPEG, DPX).
    image: PathBuf,
    /// The still's curve: di, linear, acescct, logc3, logc4, slog3, vlog, log3g10, clog2,
    /// clog3, gen5, flog, flog2 (default: detected).
    #[arg(long)]
    curve: Option<String>,
    /// The still's gamut, e.g. da_vinci_wide_gamut, xyz, aces_ap0, aces_ap1,
    /// arri_wide_gamut4, rec709 (default: detected).
    #[arg(long)]
    gamut: Option<String>,
    /// Chart orientation: rot0, rot90, rot180, rot270, mirror0, … (default: best fit).
    #[arg(long)]
    orientation: Option<String>,
    /// Place the chart by hand: centres of dark skin, bluish green, black, white as
    /// x,y,x,y,x,y,x,y (image pixels).
    #[arg(long, value_delimiter = ',', num_args = 8)]
    corners: Option<Vec<f64>>,
    /// after2014 or before2014.
    #[arg(long, default_value = "after2014")]
    edition: String,
    /// matrix, tone (matrix + per-channel tone curve) tps (TPS3D, colour only, no matrix) or matrix-tps (matrix, then TPS3D).
    #[arg(long, default_value = "matrix")]
    model: String,
    /// match (correct the exposure too) or keep.
    #[arg(long, default_value = "match")]
    exposure: String,
    /// Pull the matrix towards a plain white balance, 0 (none) … 1 (strong).
    #[arg(long, default_value_t = 0.0)]
    restraint: f64,
    /// Don't lock the neutral patches to the target's gray.
    #[arg(long)]
    no_lock_neutral: bool,
    /// Equal R, G, B stay equal (clipped white stays white); replaces the neutral lock,
    /// so the grays keep what is left of the camera's cast.
    #[arg(long)]
    preserve_neutrals: bool,
    /// TPS3D only: fit the six primaries (and the grays) instead of all colour patches.
    #[arg(long)]
    tps_primaries: bool,
    /// TPS3D only: the spline's smoothing (0 = exact through every patch).
    #[arg(long, default_value_t = lutmatrix_core::chart::fit::DEFAULT_TPS_SMOOTHING)]
    tps_smoothing: f64,
    /// Highlight protection in the DCTL: near the clip, clipped areas go to white.
    #[arg(long)]
    protect_highlights: bool,
    /// Where the footage clips, DaVinci Intermediate code (default: measured from the
    /// still).
    #[arg(long)]
    clip_di: Option<f64>,
    /// Highlight protection transition below the clip, stops.
    #[arg(long, default_value_t = 1.0)]
    highlight_softness: f64,
    /// The DCTL's white balance checkbox starts off (footage white-balanced in the raw
    /// decode).
    #[arg(long)]
    no_white_balance: bool,
    /// Patches to leave out, 1–24 (chart numbering).
    #[arg(long, value_delimiter = ',')]
    exclude: Vec<usize>,
    /// Correct to the chart in this still instead of the published values.
    #[arg(long)]
    target: Option<PathBuf>,
    /// Curve / gamut of the target still (default: detected).
    #[arg(long)]
    target_curve: Option<String>,
    #[arg(long)]
    target_gamut: Option<String>,
    /// Write the DCTL here.
    #[arg(long)]
    dctl: Option<PathBuf>,
    /// Write a JPEG preview here.
    #[arg(long)]
    preview: Option<PathBuf>,
    /// Also fit an OpenDRT look preset.
    #[arg(long)]
    opendrt: bool,
    /// OpenDRT look / tonescale / display encoding preset indices to start from.
    #[arg(long, default_value_t = 0)]
    look: usize,
    #[arg(long, default_value_t = 0)]
    tonescale: usize,
    #[arg(long, default_value_t = 0)]
    display: usize,
    /// Keep OpenDRT's tonescale as it is (fit colour only).
    #[arg(long)]
    no_fit_tone: bool,
    /// Fit the OpenDRT preset to the uncorrected chart (default: after the matrix, for
    /// matrix DCTL → OpenDRT).
    #[arg(long)]
    without_matrix: bool,
    /// OpenDRT regularisation (pull towards the starting look).
    #[arg(long, default_value_t = 6.0)]
    regularization: f64,
    /// Save the OpenDRT preset to the library under this name.
    #[arg(long)]
    save: Option<String>,
    /// Preset library folder (default: the app's).
    #[arg(long)]
    store: Option<PathBuf>,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

fn parse<T: serde::de::DeserializeOwned>(s: &str, what: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|_| format!("unknown {what} '{s}'"))
}

fn space(curve: &Option<String>, gamut: &Option<String>) -> Result<Option<(Option<StillSpace>, bool)>, String> {
    if curve.is_none() && gamut.is_none() {
        return Ok(None);
    }
    let c = curve.as_deref().map(|c| curve_from_id(c).ok_or(format!("unknown curve '{c}'"))).transpose()?;
    let g = gamut.as_deref().map(|g| parse::<StillGamut>(g, "gamut")).transpose()?;
    Ok(Some((
        Some(StillSpace {
            curve: c.unwrap_or(lutmatrix_core::spaces::LogCurve::DaVinciIntermediate),
            gamut: g.unwrap_or(StillGamut::DaVinciWideGamut),
        }),
        c.is_some() && g.is_some(),
    )))
}

fn run_one(path: &Path, s: &ChartSettings, partial: Option<(Option<String>, Option<String>)>) -> Result<(ChartImage, ChartAnalysis), String> {
    let img = ChartImage::load(path).map_err(|e| e.to_string())?;
    let lat = if s.corners.is_none() { find_lattice(&img) } else { None };
    let mut a = analyze(&img, lat.as_ref(), s).map_err(|e| e.to_string())?;
    // Only one of curve/gamut given: keep the detected other half.
    if let Some((c, g)) = partial {
        let mut sp = a.guess.space;
        if let Some(c) = c {
            sp.curve = curve_from_id(&c).ok_or(format!("unknown curve '{c}'"))?;
        }
        if let Some(g) = g {
            sp.gamut = parse(&g, "gamut")?;
        }
        let s2 = ChartSettings { space: Some(sp), ..s.clone() };
        a = analyze(&img, lat.as_ref(), &s2).map_err(|e| e.to_string())?;
    }
    Ok((img, a))
}

fn conf(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
        Confidence::None => "none",
    }
}

pub fn run(args: ChartArgs) -> Result<(), String> {
    let mut s = ChartSettings {
        edition: parse::<ChartEdition>(&args.edition, "edition")?,
        orientation: args.orientation.as_deref().map(|o| parse::<Orientation>(o, "orientation")).transpose()?,
        corners: args.corners.as_ref().map(|v| [[v[0], v[1]], [v[2], v[3]], [v[4], v[5]], [v[6], v[7]]]),
        exclude: args.exclude.iter().filter(|&&i| (1..=24).contains(&i)).map(|i| i - 1).collect(),
        ..ChartSettings::default()
    };
    s.fit.model = match args.model.as_str() {
        "matrix" => Model::Matrix,
        "tone" => Model::MatrixTone,
        "tps" | "tps3d" => Model::Tps,
        "matrix-tps" => Model::MatrixTps,
        m => return Err(format!("unknown model '{m}'")),
    };
    s.fit.exposure = match args.exposure.as_str() {
        "match" => Exposure::Match,
        "keep" => Exposure::Keep,
        e => return Err(format!("unknown exposure '{e}'")),
    };
    s.fit.lock_neutral = !args.no_lock_neutral;
    s.fit.restraint = args.restraint;
    s.fit.preserve_neutrals = args.preserve_neutrals;
    if args.tps_primaries {
        s.fit.tps_patches = lutmatrix_core::chart::fit::TpsPatches::Primaries;
    }
    s.fit.tps_smoothing = args.tps_smoothing;
    s.highlights.enabled = args.protect_highlights;
    s.highlights.clip_di = args.clip_di;
    s.highlights.softness = args.highlight_softness;
    s.white_balance = !args.no_white_balance;
    if let Some(t) = &args.target {
        let ts = ChartSettings { space: None, corners: None, orientation: None, exclude: Vec::new(), ..s.clone() };
        let partial = space(&args.target_curve, &args.target_gamut)?;
        let (ts, partial) = match partial {
            Some((sp, true)) => (ChartSettings { space: sp, ..ts }, None),
            Some((_, false)) => (ts, Some((args.target_curve.clone(), args.target_gamut.clone()))),
            None => (ts, None),
        };
        let (_, ta) = run_one(t, &ts, partial)?;
        eprintln!("target chart: {} ({}), {} usable patches", ta.file_name, ta.space.name(), ta.patches.iter().filter(|p| p.used).count());
        s.target = ta.as_target();
    }
    let partial = space(&args.curve, &args.gamut)?;
    let partial = match partial {
        Some((sp, true)) => {
            s.space = sp;
            None
        }
        Some((_, false)) => Some((args.curve.clone(), args.gamut.clone())),
        None => None,
    };
    let (img, a) = run_one(&args.image, &s, partial)?;
    let odrt = if args.opendrt {
        let os = OdrtChartSettings {
            look: args.look,
            tonescale: args.tonescale,
            display_encoding: args.display,
            fit_tone: !args.no_fit_tone,
            after_matrix: !args.without_matrix,
            regularization: args.regularization,
            ..OdrtChartSettings::default()
        };
        Some(fit_opendrt_chart(&a, &os)?)
    } else {
        None
    };
    if let Some(p) = &args.preview {
        let ratio = a.fit.as_ref().map_or(1.0, |f| f.exposure_ratio);
        let (jpg, _, _) = preview_jpeg(&img, &a.space, ratio, 1600);
        std::fs::write(p, jpg).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    if let Some(p) = &args.dctl {
        let src = a.dctl(&today_utc()).ok_or("no fit, nothing to export")?;
        std::fs::write(p, src).map_err(|e| format!("{}: {e}", p.display()))?;
        eprintln!("wrote {}", p.display());
    }
    if let (Some(name), Some(f)) = (&args.save, &odrt) {
        let store = match &args.store {
            Some(d) => PresetStore::at(d),
            None => PresetStore::open_default().map_err(|e| e.to_string())?,
        };
        let p = StoredPreset::from_chart(f, Some(name)).map_err(|e| e.to_string())?;
        let path = store.save(&p, false).map_err(|e| e.to_string())?;
        eprintln!("saved preset '{}' to {}", p.name, path.display());
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "chart": a, "opendrt": odrt })).unwrap());
        return Ok(());
    }
    println!("{}  ({} {}, {}×{})", a.file_name, a.image.format, a.image.sample_type, a.image.width, a.image.height);
    println!(
        "  Chart       {} ({}, {} patches found, pitch {:.0} px)",
        a.placement.orientation.label(),
        a.placement_source,
        a.detected_patches,
        a.pitch_px
    );
    for alt in a.alternatives.iter().skip(1).take(2) {
        println!("              alt: {} (score {:.1} vs {:.1})", alt.orientation.label(), alt.score, a.alternatives[0].score);
    }
    println!(
        "  Encoding    {} [{}; curve {}, gamut {}]",
        a.space.name(),
        if a.space_detected { "detected" } else { "chosen" },
        conf(a.guess.curve_confidence),
        conf(a.guess.gamut_confidence)
    );
    for r in &a.guess.reasons {
        println!("              · {r}");
    }
    println!("  Curves      {}", a.guess.curves.iter().take(4).map(|c| format!("{} {:.2}", curve_id(c.curve), c.score)).collect::<Vec<_>>().join(", "));
    println!("  Target      {}", a.target_name);
    println!("\n   #  patch           code (R G B)               used   ΔE before → after");
    for p in &a.patches {
        println!(
            "  {:>2}  {:<14}  {:.4} {:.4} {:.4}   {:<5}  {:>6.2} → {:>5.2}  {}",
            p.index + 1,
            p.name,
            p.sample.mean[0],
            p.sample.mean[1],
            p.sample.mean[2],
            if p.used { "yes" } else { "no" },
            p.de_before,
            p.de_after,
            p.reason.as_deref().unwrap_or("")
        );
    }
    match (&a.fit, &a.fit_error) {
        (Some(f), _) => {
            println!(
                "\n  Fit         {:?}, exposure {:?} (still {:+.2} stops from target), neutral gain ×{:.3}",
                f.settings.model,
                f.settings.exposure,
                f.exposure_ratio.log2(),
                f.neutral_gain
            );
            println!("  ΔE2000      mean {:.2} → {:.2}, max {:.2}, neutrals {:.2}; {} patches", f.mean_before, f.mean_after, f.max_after, f.neutral_after, f.used);
            let [gr, _, gb] = f.gray_dwg;
            let ratio = |m: &lutmatrix_core::math::Mat3| {
                let w = m * lutmatrix_core::math::Vec3::repeat(1.0);
                format!("{:.3} : {:.3} : {:.3}", w.x / w.max(), w.y / w.max(), w.z / w.max())
            };
            println!(
                "  Neutrals    {}; camera gray R {gr:.3} : G 1 : B {gb:.3} (DWG); equal RGB → {} (white balance off: {})",
                if f.settings.preserve_neutrals { "preserved" } else if f.settings.lock_neutral { "locked" } else { "free" },
                ratio(&f.m_dwg),
                ratio(&f.m_dwg_after_wb)
            );
            println!("  White bal.  R ×{:.4}, G ×1, B ×{:.4} (linear DWG)", f.wb_gains[0], f.wb_gains[2]);
            if let (Some(t), Some(c)) = (&f.tps, f.tps_check) {
                println!(
                    "  TPS3D       smoothing {:.4}: chart mean {:.2}, cross-check {:.2}; the matrix model: {:.2}, cross-check {:.2}",
                    t.smoothing,
                    f.mean_after,
                    c,
                    f.mean_matrix,
                    f.matrix_check.unwrap_or(f64::NAN)
                );
            }
            println!("  M_XYZ       {:?}", [f.m_xyz.row(0), f.m_xyz.row(1), f.m_xyz.row(2)].map(|r| [r[0], r[1], r[2]].map(|v| (v * 1e5).round() / 1e5)));
            if let Some(t) = &f.tone {
                println!("  Tone        power {:?}, black {:?}", t.power.map(|v| (v * 1e4).round() / 1e4), t.black.map(|v| (v * 1e5).round() / 1e5));
            }
        }
        (None, Some(e)) => println!("\n  Fit         failed: {e}"),
        _ => {}
    }
    let h = &a.highlights;
    println!(
        "  Highlights  clip {:.4} DI ({:?}{}){}",
        h.clip_di,
        h.source,
        if h.source == lutmatrix_core::chart::highlights::ClipSource::Set { format!("; still {:.4} {:?}", h.measured_di, h.measured_source) } else { String::new() },
        if h.enabled { format!("; protection on, softness {:.2} stops", h.softness) } else { "; protection off".into() }
    );
    for n in &a.notes {
        println!("  Note        {n}");
    }
    if let Some(o) = &odrt {
        println!("\n  OpenDRT     {} → preset; grey {:.2} nits; {:.1} s", o.base, o.config.grey_nits, o.seconds);
        println!("  ΔE2000      stock look {:.2} → preset {:.2} (max {:.2}, neutrals {:.2})", o.before.mean, o.after.mean, o.after.max, o.after.neutral);
        println!("  Safety      {}", if o.safety.ok() { "ok" } else { "see notes" });
        for n in &o.notes {
            println!("  Note        {n}");
        }
    }
    Ok(())
}
