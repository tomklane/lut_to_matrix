//! LUT Matrix desktop app: Tauri commands over `lutmatrix-core`.
//!
//! The web front end (app/src) does layout and state; everything colour-related runs here.
//! Long jobs (analysis, OpenDRT fitting) run on a blocking thread so the window stays live.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lutmatrix_core::analysis::{analyze_with, Analysis, Reference};
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
use tauri::State;

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running LUT Matrix");
}
