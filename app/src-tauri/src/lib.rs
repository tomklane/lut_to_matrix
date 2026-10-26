//! LUT Matrix desktop app: Tauri commands over `lutmatrix-core`.
//!
//! The web front end (app/src) does layout and state; everything colour-related runs here.
//! Long jobs (analysis, OpenDRT fitting) run on a blocking thread so the window stays live.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use lutmatrix_core::analysis::{analyze_with, Analysis, Reference};
use lutmatrix_core::batch::{self, BatchOptions, Negative, PairCheck, PairResult, Print};
use lutmatrix_core::film::{guess_stage, negative_name, Stage, StockNames};
use lutmatrix_core::dctl;
use lutmatrix_core::detect::{detect, Detection};
use lutmatrix_core::lut::Lut;
use lutmatrix_core::opendrt_match::{match_opendrt, MatchSettings, OpenDrtMatch};
use lutmatrix_core::preset_store::{data_dir, import_file, PresetStore, StoredPreset};
use lutmatrix_core::sample::Settings;
use lutmatrix_core::spaces::camera::CAMERA_SPACES;
use lutmatrix_core::spaces::curves::{LogCurve, ARRI_LOGC3_EIS};
use lutmatrix_core::spaces::display::PQ_PEAK_PRESETS;
use lutmatrix_core::spaces::{DisplayGamut, InputSpace, OutputSpace};
use lutmatrix_opendrt::writer::lut_match_dctl;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

#[derive(Default)]
struct AppState {
    luts: Mutex<HashMap<String, Arc<Lut>>>,
    /// The analysis on screen (what Export / Copy use) and the LUT file it came from.
    analysis: Mutex<Option<(String, Analysis)>>,
    /// The OpenDRT fit on screen (what Save as Preset uses).
    odrt: Mutex<Option<OpenDrtMatch>>,
    /// A LUT passed on the command line / via "Open With".
    initial_file: Mutex<Option<String>>,
    /// The batch (negatives × prints).
    batch: Mutex<BatchState>,
    batch_cancel: Arc<AtomicBool>,
}

/// LUTs in the batch and the latest results.
#[derive(Default)]
struct BatchState {
    next_id: u32,
    items: Vec<BatchItem>,
    results: Vec<PairResult>,
    running: bool,
    /// Where the current/last run is written.
    folder: Option<PathBuf>,
    /// The last run on disk was looked for (once, at first use).
    restored: bool,
}

struct BatchItem {
    id: u32,
    stage: Stage,
    path: String,
    file: String,
    name: String,
    /// The user typed this name (keep it when the stock names change).
    name_edited: bool,
    lut: Arc<Lut>,
    input: InputSpace,
    output: OutputSpace,
    include: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn load_cached(state: &AppState, path: &str) -> CmdResult<Arc<Lut>> {
    if let Some(l) = state.luts.lock().unwrap().get(path) {
        return Ok(l.clone());
    }
    let lut = Arc::new(Lut::load(Path::new(path)).map_err(|e| format!("{}: {e}", file_name(path)))?);
    state.luts.lock().unwrap().insert(path.to_string(), lut.clone());
    Ok(lut)
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

fn stem(path: &str) -> String {
    Path::new(path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/* ---------- app info and catalog ---------- */

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    os: &'static str,
    data_dir: Option<String>,
    presets_dir: Option<String>,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: lutmatrix_core::VERSION,
        os: std::env::consts::OS,
        data_dir: data_dir().ok().map(|d| d.display().to_string()),
        presets_dir: PresetStore::default_dir().ok().map(|d| d.display().to_string()),
    }
}

#[derive(Serialize)]
struct CameraEntry {
    id: serde_json::Value,
    name: &'static str,
    gamut_name: &'static str,
    has_ei: bool,
}

#[derive(Serialize)]
struct Catalog {
    cameras: Vec<CameraEntry>,
    gamuts: Vec<(DisplayGamut, &'static str)>,
    arri_eis: Vec<u16>,
    pq_peaks: Vec<f64>,
}

#[tauri::command]
fn catalog() -> Catalog {
    Catalog {
        cameras: CAMERA_SPACES
            .iter()
            .map(|c| CameraEntry {
                id: serde_json::to_value(c.id).unwrap(),
                name: c.name,
                gamut_name: c.gamut_name,
                has_ei: matches!(c.curve, LogCurve::ArriLogC3 { .. }),
            })
            .collect(),
        gamuts: DisplayGamut::ALL.iter().map(|g| (*g, g.name())).collect(),
        arri_eis: ARRI_LOGC3_EIS.to_vec(),
        pq_peaks: PQ_PEAK_PRESETS.to_vec(),
    }
}

/* ---------- LUT loading ---------- */

#[derive(Serialize)]
struct LoadedLut {
    path: String,
    file_name: String,
    describe: String,
    title: Option<String>,
    output_range: (f64, f64),
    warnings: Vec<String>,
    detection: Detection,
    /// Detected input and output with names, ready to use.
    input: InputSpace,
    input_name: String,
    output: OutputSpace,
}

#[tauri::command]
fn load_lut(path: String, state: State<AppState>) -> CmdResult<LoadedLut> {
    // Always re-read from disk on an explicit load (the file may have changed).
    state.luts.lock().unwrap().remove(&path);
    let lut = load_cached(&state, &path)?;
    if !lut.is_3d() && lut.shaper.is_none() {
        return Err(format!("{} has no LUT data", file_name(&path)));
    }
    let d = detect(&lut);
    Ok(LoadedLut {
        file_name: file_name(&path),
        describe: lut.describe(),
        title: lut.title.clone(),
        output_range: lut.output_extent(),
        warnings: lut.warnings.clone(),
        input: d.input.value,
        input_name: d.input.value.name(),
        output: d.output_space(),
        detection: d,
        path,
    })
}

#[tauri::command]
fn initial_file(state: State<AppState>) -> Option<String> {
    state.initial_file.lock().unwrap().take()
}

/* ---------- matrix analysis ---------- */

#[derive(Deserialize)]
struct ThroughRequest {
    path: String,
    input: InputSpace,
    output: OutputSpace,
}

#[derive(Deserialize)]
struct AnalyzeRequest {
    path: String,
    input: InputSpace,
    output: OutputSpace,
    settings: Settings,
    middle_gray_hint: Option<f64>,
    through: Option<ThroughRequest>,
}

#[tauri::command]
async fn analyze(req: AnalyzeRequest, state: State<'_, AppState>) -> CmdResult<Analysis> {
    let lut = load_cached(&state, &req.path)?;
    let through = match &req.through {
        Some(t) => Some((load_cached(&state, &t.path)?, t)),
        None => None,
    };
    let through = through.map(|(l, t)| (l, file_name(&t.path), t.input, t.output));
    let (input, output, settings, hint) = (req.input, req.output, req.settings, req.middle_gray_hint);
    let a = tauri::async_runtime::spawn_blocking(move || {
        let r = through.as_ref().map(|(l, name, i, o)| Reference {
            transform: l.as_ref(),
            name: name.clone(),
            input: *i,
            output: *o,
        });
        analyze_with(lut.as_ref(), input, output, &settings, hint, r.as_ref())
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    *state.analysis.lock().unwrap() = Some((req.path.clone(), a.clone()));
    Ok(a)
}

#[derive(Serialize)]
struct Exported {
    dctl: String,
    json: String,
    txt: String,
}

#[tauri::command]
fn default_dctl_name(path: String) -> String {
    dctl::dctl_file_name(&file_name(&path))
}

/// Write the on-screen analysis as `dctl_path` plus .json and .txt beside it.
#[tauri::command]
fn export_dctl(dctl_path: String, state: State<AppState>) -> CmdResult<Exported> {
    let guard = state.analysis.lock().unwrap();
    let (lut_path, a) = guard.as_ref().ok_or("nothing to export yet")?;
    let meta = dctl::ExportMeta::from_analysis(a, &file_name(lut_path), &dctl::today_utc());
    let p = PathBuf::from(&dctl_path);
    let p = if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dctl")) { p } else { p.with_extension("dctl") };
    let json = p.with_extension("json");
    let txt = p.with_extension("txt");
    for (path, body) in [
        (&p, dctl::generate_dctl(&a.m_xyz, &meta)),
        (&json, dctl::export_json(a, &meta)),
        (&txt, dctl::export_text(a, &meta)),
    ] {
        std::fs::write(path, body).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    let s = |p: &PathBuf| p.display().to_string();
    Ok(Exported { dctl: s(&p), json: s(&json), txt: s(&txt) })
}

/// The on-screen matrix as text for the clipboard.
#[tauri::command]
fn matrix_text(state: State<AppState>) -> CmdResult<String> {
    let guard = state.analysis.lock().unwrap();
    let (_, a) = guard.as_ref().ok_or("no matrix yet")?;
    let m = &a.m_xyz;
    Ok((0..3)
        .map(|r| format!("{:.10} {:.10} {:.10}", m[(r, 0)], m[(r, 1)], m[(r, 2)]))
        .collect::<Vec<_>>()
        .join("\n"))
}

/* ---------- preferences ---------- */

fn prefs_path() -> CmdResult<PathBuf> {
    Ok(data_dir().map_err(err)?.join("prefs.json"))
}

#[tauri::command]
fn load_prefs() -> serde_json::Value {
    prefs_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null)
}

#[tauri::command]
fn save_prefs(prefs: serde_json::Value) -> CmdResult<()> {
    let p = prefs_path()?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(err)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&prefs).map_err(err)?).map_err(err)?;
    std::fs::rename(&tmp, &p).map_err(err)
}

/* ---------- OpenDRT matching and presets ---------- */

#[derive(Deserialize)]
struct OdrtRequest {
    path: String,
    input: InputSpace,
    output: OutputSpace,
    settings: MatchSettings,
}

#[tauri::command]
async fn opendrt_fit(req: OdrtRequest, state: State<'_, AppState>) -> CmdResult<OpenDrtMatch> {
    let lut = load_cached(&state, &req.path)?;
    let name = Some(file_name(&req.path));
    let m = tauri::async_runtime::spawn_blocking(move || {
        match_opendrt(lut.as_ref(), name, req.input, req.output, &req.settings)
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    *state.odrt.lock().unwrap() = Some(m.clone());
    Ok(m)
}

fn store() -> CmdResult<PresetStore> {
    PresetStore::open_default().map_err(err)
}

#[derive(Serialize)]
struct PresetRow {
    name: String,
    created: String,
    source_lut: String,
    grey_nits: f32,
    /// (metric label, fitted mean) when the preset has a fit report.
    error: Option<(String, f64)>,
    stock_error: Option<f64>,
    notes: Vec<String>,
}

#[derive(Serialize)]
struct PresetList {
    dir: String,
    presets: Vec<PresetRow>,
    problems: Vec<String>,
}

#[tauri::command]
fn presets_list() -> CmdResult<PresetList> {
    let s = store()?;
    let (all, bad) = s.list().map_err(err)?;
    Ok(PresetList {
        dir: s.dir().display().to_string(),
        presets: all
            .into_iter()
            .map(|p| PresetRow {
                created: p.created_date(),
                error: p.report.as_ref().map(|r| (r.metric.label().to_string(), r.fitted.mean)),
                stock_error: p.report.as_ref().map(|r| r.stock.mean),
                notes: p.report.as_ref().map(|r| r.notes.clone()).unwrap_or_default(),
                name: p.name,
                source_lut: p.source_lut,
                grey_nits: p.grey_nits,
            })
            .collect(),
        problems: bad.into_iter().map(|e| e.to_string()).collect(),
    })
}

#[tauri::command]
fn default_preset_name(path: String) -> String {
    format!("LUT Match {}", stem(&path))
}

#[tauri::command]
fn preset_exists(name: String) -> CmdResult<bool> {
    Ok(store()?.get(&name).is_ok())
}

/// Save the on-screen OpenDRT fit to the library.
#[tauri::command]
fn preset_save(name: String, replace: bool, state: State<AppState>) -> CmdResult<String> {
    let guard = state.odrt.lock().unwrap();
    let m = guard.as_ref().ok_or("no OpenDRT fit to save")?;
    let p = StoredPreset::from_match(m, Some(&name)).map_err(err)?;
    store()?.save(&p, replace).map_err(err)?;
    Ok(p.name)
}

#[tauri::command]
fn preset_remove(name: String) -> CmdResult<()> {
    store()?.remove(&name).map_err(err)
}

#[tauri::command]
fn preset_rename(name: String, new_name: String) -> CmdResult<String> {
    let s = store()?;
    let mut p = s.get(&name).map_err(err)?;
    let clean = lutmatrix_opendrt::writer::sanitize_name(&new_name);
    if clean.is_empty() {
        return Err("that name has nothing a DCTL menu can show".into());
    }
    if clean.to_lowercase() != p.name.to_lowercase() && s.get(&clean).is_ok() {
        return Err(format!("a preset called '{clean}' already exists"));
    }
    s.remove(&p.name).map_err(err)?;
    p.name = clean.clone();
    s.save(&p, true).map_err(err)?;
    Ok(clean)
}

#[tauri::command]
fn preset_import(paths: Vec<String>, replace: bool) -> CmdResult<Vec<String>> {
    let s = store()?;
    let mut names = Vec::new();
    for path in paths {
        let p = import_file(Path::new(&path), None).map_err(err)?;
        s.save(&p, replace).map_err(err)?;
        names.push(p.name);
    }
    Ok(names)
}

#[tauri::command]
fn preset_export(name: String, path: String) -> CmdResult<()> {
    let p = store()?.get(&name).map_err(err)?;
    std::fs::write(&path, serde_json::to_string_pretty(&p).map_err(err)? + "\n").map_err(err)
}

/// Write OpenDRT with the named presets (menu order as given) to `path`.
#[tauri::command]
fn opendrt_build(names: Vec<String>, path: String) -> CmdResult<String> {
    let s = store()?;
    let presets = names.iter().map(|n| s.get(n).map(|p| p.to_writer()).map_err(err)).collect::<CmdResult<Vec<_>>>()?;
    if presets.is_empty() {
        return Err("choose at least one preset".into());
    }
    let src = lut_match_dctl(&presets, "").map_err(err)?;
    std::fs::write(&path, src).map_err(|e| format!("could not write {path}: {e}"))?;
    Ok(path)
}

/* ---------- batch: every negative × every print ---------- */

#[derive(Serialize)]
struct BatchItemView {
    id: u32,
    stage: Stage,
    path: String,
    file: String,
    name: String,
    name_edited: bool,
    input: InputSpace,
    output: OutputSpace,
    include: bool,
    /// Print variant code / flags / revision from the stock-names table (for filtering).
    variant: Option<String>,
    variant_word: Option<String>,
    flags: Vec<String>,
}

/// One pair's result, without the full fit (fetch that with `batch_result`).
#[derive(Serialize, Clone)]
struct PairSummary {
    name: String,
    negative: String,
    print: String,
    check: PairCheck,
    error: Option<String>,
    metric: Option<String>,
    stock: Option<f64>,
    best_preset: Option<f64>,
    fitted: Option<f64>,
    p95: Option<f64>,
    gray: Option<f64>,
    grey_nits: Option<f32>,
    start: Option<String>,
    safety_ok: Option<bool>,
    notes: Vec<String>,
}

impl From<&PairResult> for PairSummary {
    fn from(r: &PairResult) -> Self {
        let m = r.result.as_ref();
        PairSummary {
            name: r.name.clone(),
            negative: r.negative.clone(),
            print: r.print.clone(),
            check: r.check.clone(),
            error: r.error.clone(),
            metric: m.map(|m| m.metric.label().to_string()),
            stock: m.map(|m| m.stock.mean),
            best_preset: m.map(|m| m.best_preset.mean),
            fitted: m.map(|m| m.fitted.mean),
            p95: m.map(|m| m.fitted.p95),
            gray: m.map(|m| m.fitted.gray_mean),
            grey_nits: m.map(|m| m.config.grey_nits),
            start: m.map(|m| m.start.description.clone()),
            safety_ok: m.map(|m| m.safety.ok()),
            notes: m.map(|m| m.notes.clone()).unwrap_or_default(),
        }
    }
}

#[derive(Serialize)]
struct BatchView {
    items: Vec<BatchItemView>,
    results: Vec<PairSummary>,
    running: bool,
    folder: Option<String>,
    names_path: Option<String>,
}

fn stock_names() -> StockNames {
    StockNames::default_path().map(|p| StockNames::load(&p).unwrap_or_default()).unwrap_or_default()
}

fn batch_root() -> CmdResult<PathBuf> {
    Ok(data_dir().map_err(err)?.join("batches"))
}

/// Results of the latest run saved on disk (so a closed app keeps its batch).
fn restore_last(b: &mut BatchState) {
    if b.restored {
        return;
    }
    b.restored = true;
    let Ok(root) = batch_root() else { return };
    let Ok(rd) = std::fs::read_dir(&root) else { return };
    let Some(last) = rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).max() else { return };
    let mut results: Vec<PairResult> = std::fs::read_dir(&last)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    results.sort_by(|a, b| a.name.cmp(&b.name));
    if !results.is_empty() {
        b.results = results;
        b.folder = Some(last);
    }
}

fn batch_view(b: &BatchState) -> BatchView {
    let names = stock_names();
    BatchView {
        items: b
            .items
            .iter()
            .map(|i| {
                let pn = if i.stage == Stage::Print { names.print_name(&i.file) } else { None };
                BatchItemView {
                    id: i.id,
                    stage: i.stage,
                    path: i.path.clone(),
                    file: i.file.clone(),
                    name: i.name.clone(),
                    name_edited: i.name_edited,
                    input: i.input,
                    output: i.output,
                    include: i.include,
                    variant_word: pn.as_ref().and_then(|p| p.variant.as_ref().and_then(|v| names.variants.get(v).cloned())),
                    variant: pn.as_ref().and_then(|p| p.variant.clone()),
                    flags: pn.map(|p| p.flags).unwrap_or_default(),
                }
            })
            .collect(),
        results: b.results.iter().map(PairSummary::from).collect(),
        running: b.running,
        folder: b.folder.as_ref().map(|f| f.display().to_string()),
        names_path: StockNames::default_path().map(|p| p.display().to_string()),
    }
}

fn auto_name(stage: Stage, file: &str, names: &StockNames) -> String {
    let fallback = || stem(file);
    match stage {
        Stage::Negative => negative_name(file).unwrap_or_else(fallback),
        Stage::Print => names.print_name(file).map(|p| p.name).unwrap_or_else(fallback),
    }
}

#[tauri::command]
fn batch_state(state: State<AppState>) -> BatchView {
    let mut b = state.batch.lock().unwrap();
    restore_last(&mut b);
    batch_view(&b)
}

/// Add LUT files; each goes to the list its name suggests (`stage` forces one).
#[tauri::command]
async fn batch_add(paths: Vec<String>, stage: Option<Stage>, state: State<'_, AppState>) -> CmdResult<BatchView> {
    let names = stock_names();
    let mut problems = Vec::new();
    let mut loaded = Vec::new();
    for path in paths {
        if state.batch.lock().unwrap().items.iter().any(|i| i.path == path) {
            continue;
        }
        let p2 = path.clone();
        match tauri::async_runtime::spawn_blocking(move || Lut::load(Path::new(&p2))).await.map_err(err)? {
            Ok(lut) => {
                let d = detect(&lut);
                let file = file_name(&path);
                let stage = stage.unwrap_or_else(|| guess_stage(&file, &names));
                loaded.push((path, file, stage, d.input.value, d.output_space(), lut));
            }
            Err(e) => problems.push(format!("{}: {e}", file_name(&path))),
        }
    }
    let mut b = state.batch.lock().unwrap();
    for (path, file, stage, input, output, lut) in loaded {
        b.next_id += 1;
        let name = auto_name(stage, &file, &names);
        let id = b.next_id;
        b.items.push(BatchItem { id, stage, path, file, name, name_edited: false, lut: Arc::new(lut), input, output, include: true });
    }
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    Ok(batch_view(&b))
}

#[derive(Deserialize)]
struct BatchItemUpdate {
    id: u32,
    name: Option<String>,
    stage: Option<Stage>,
    input: Option<InputSpace>,
    output: Option<OutputSpace>,
    include: Option<bool>,
}

#[tauri::command]
fn batch_update(updates: Vec<BatchItemUpdate>, state: State<AppState>) -> CmdResult<BatchView> {
    let names = stock_names();
    let mut b = state.batch.lock().unwrap();
    for u in updates {
        let Some(i) = b.items.iter_mut().find(|i| i.id == u.id) else { continue };
        if let Some(st) = u.stage {
            if st != i.stage {
                i.stage = st;
                if !i.name_edited {
                    i.name = auto_name(st, &i.file, &names);
                }
            }
        }
        if let Some(n) = u.name {
            let n = n.trim().to_string();
            if n.is_empty() {
                i.name = auto_name(i.stage, &i.file, &names);
                i.name_edited = false;
            } else {
                i.name = n;
                i.name_edited = true;
            }
        }
        if let Some(x) = u.input {
            i.input = x;
        }
        if let Some(x) = u.output {
            i.output = x;
        }
        if let Some(x) = u.include {
            i.include = x;
        }
    }
    Ok(batch_view(&b))
}

#[tauri::command]
fn batch_remove(ids: Vec<u32>, state: State<AppState>) -> BatchView {
    let mut b = state.batch.lock().unwrap();
    b.items.retain(|i| !ids.contains(&i.id));
    batch_view(&b)
}

#[tauri::command]
fn stock_names_get() -> StockNames {
    stock_names()
}

/// Save the stock-names table and rename the LUTs whose names were not typed by hand.
#[tauri::command]
fn stock_names_set(names: StockNames, state: State<AppState>) -> CmdResult<BatchView> {
    let path = StockNames::default_path().ok_or("no data folder")?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(err)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&names).map_err(err)? + "\n").map_err(err)?;
    let mut b = state.batch.lock().unwrap();
    for i in b.items.iter_mut().filter(|i| !i.name_edited) {
        i.name = auto_name(i.stage, &i.file, &names);
    }
    Ok(batch_view(&b))
}

#[tauri::command]
fn stock_names_import(path: String) -> CmdResult<StockNames> {
    let t = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&t).map_err(|e| format!("{}: not a stock-names file ({e})", file_name(&path)))
}

#[derive(Deserialize)]
struct BatchRunRequest {
    settings: MatchSettings,
    check_only: bool,
    fit_mismatched: bool,
}

#[derive(Serialize, Clone)]
struct BatchProgress {
    done: usize,
    total: usize,
    pair: PairSummary,
}

/// Run the batch on the included LUTs. Emits `batch-pair` as each pair finishes; the
/// results also go to a new folder under `batches/` in the data folder.
#[tauri::command]
async fn batch_run(req: BatchRunRequest, app: AppHandle, state: State<'_, AppState>) -> CmdResult<BatchView> {
    let (negatives, prints, folder) = {
        let mut b = state.batch.lock().unwrap();
        if b.running {
            return Err("a batch is already running".into());
        }
        let negatives: Vec<Negative> = b
            .items
            .iter()
            .filter(|i| i.include && i.stage == Stage::Negative)
            .map(|i| Negative { name: i.name.clone(), file: i.file.clone(), lut: (*i.lut).clone(), input: i.input })
            .collect();
        let prints: Vec<Print> = b
            .items
            .iter()
            .filter(|i| i.include && i.stage == Stage::Print)
            .map(|i| Print { name: i.name.clone(), file: i.file.clone(), lut: (*i.lut).clone(), output: i.output })
            .collect();
        if negatives.is_empty() || prints.is_empty() {
            return Err("add at least one negative and one print, and tick them".into());
        }
        let mut seen = std::collections::HashMap::new();
        for n in &negatives {
            for p in &prints {
                let name = lutmatrix_core::film::pair_name(&n.name, &p.name);
                if let Some((a, c)) = seen.insert(name.clone(), (n.file.clone(), p.file.clone())) {
                    return Err(format!(
                        "two pairs would both be called '{name}' ({a} + {c}, and {} + {}); rename one",
                        n.file, p.file
                    ));
                }
            }
        }
        let folder = if req.check_only {
            None
        } else {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let f = batch_root()?.join(format!("batch-{secs}"));
            std::fs::create_dir_all(&f).map_err(err)?;
            Some(f)
        };
        b.running = true;
        b.results.clear();
        if folder.is_some() {
            b.folder = folder.clone();
        }
        (negatives, prints, folder)
    };
    state.batch_cancel.store(false, Ordering::Relaxed);
    let cancel = state.batch_cancel.clone();
    let opts = BatchOptions { settings: req.settings, fit_mismatched: req.fit_mismatched, check_only: req.check_only };
    let app2 = app.clone();
    let folder2 = folder.clone();
    let results = tauri::async_runtime::spawn_blocking(move || {
        batch::run(&negatives, &prints, &opts, &cancel, |done, total, r| {
            if let Some(f) = &folder2 {
                let path = f.join(format!("{}.json", r.name.replace(['/', '\\', ':'], "-")));
                let _ = std::fs::write(path, serde_json::to_string_pretty(r).unwrap_or_default() + "\n");
            }
            let st = app2.state::<AppState>();
            st.batch.lock().unwrap().results.push(r.clone());
            let _ = app2.emit("batch-pair", BatchProgress { done, total, pair: PairSummary::from(r) });
        })
    })
    .await;
    let mut b = state.batch.lock().unwrap();
    b.running = false;
    let results = results.map_err(err)?;
    if let Some(f) = &folder {
        let _ = std::fs::write(f.join("batch.csv"), batch::csv(&results));
    }
    Ok(batch_view(&b))
}

#[tauri::command]
fn batch_cancel(state: State<AppState>) {
    state.batch_cancel.store(true, Ordering::Relaxed);
}

/// The full fit of one pair; it also becomes the on-screen fit (so OpenDRT Match can
/// show and save it).
#[tauri::command]
fn batch_result(name: String, state: State<AppState>) -> CmdResult<OpenDrtMatch> {
    let b = state.batch.lock().unwrap();
    let r = b.results.iter().find(|r| r.name == name).ok_or("no such pair")?;
    let m = r.result.clone().ok_or_else(|| r.error.clone().unwrap_or_else(|| "not fitted".into()))?;
    *state.odrt.lock().unwrap() = Some(m.clone());
    Ok(m)
}

/// Save fitted pairs to the preset library under their pair names.
#[tauri::command]
fn batch_save(names: Vec<String>, replace: bool, state: State<AppState>) -> CmdResult<Vec<String>> {
    let s = store()?;
    let b = state.batch.lock().unwrap();
    let mut saved = Vec::new();
    let mut existing = Vec::new();
    for n in &names {
        let Some(m) = b.results.iter().find(|r| &r.name == n).and_then(|r| r.result.as_ref()) else { continue };
        let p = StoredPreset::from_match(m, Some(n)).map_err(err)?;
        if !replace && s.get(&p.name).is_ok() {
            existing.push(p.name);
            continue;
        }
        s.save(&p, replace).map_err(err)?;
        saved.push(p.name);
    }
    if !existing.is_empty() {
        return Err(format!("already in your library (not replaced): {}", existing.join(", ")));
    }
    Ok(saved)
}

#[tauri::command]
fn batch_existing(names: Vec<String>) -> CmdResult<Vec<String>> {
    let s = store()?;
    Ok(names.into_iter().filter(|n| s.get(n).is_ok()).collect())
}

#[tauri::command]
fn batch_export_csv(path: String, state: State<AppState>) -> CmdResult<()> {
    let b = state.batch.lock().unwrap();
    std::fs::write(&path, batch::csv(&b.results)).map_err(|e| format!("could not write {path}: {e}"))
}

pub fn run() {
    let state = AppState::default();
    // A LUT given on the command line (or by "Open With").
    if let Some(arg) = std::env::args().skip(1).find(|a| !a.starts_with('-')) {
        *state.initial_file.lock().unwrap() = Some(arg);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            app_info,
            catalog,
            load_lut,
            initial_file,
            analyze,
            default_dctl_name,
            export_dctl,
            matrix_text,
            load_prefs,
            save_prefs,
            opendrt_fit,
            presets_list,
            default_preset_name,
            preset_exists,
            preset_save,
            preset_remove,
            preset_rename,
            preset_import,
            preset_export,
            opendrt_build,
            batch_state,
            batch_add,
            batch_update,
            batch_remove,
            stock_names_get,
            stock_names_set,
            stock_names_import,
            batch_run,
            batch_cancel,
            batch_result,
            batch_save,
            batch_existing,
            batch_export_csv,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LUT Matrix");
}
