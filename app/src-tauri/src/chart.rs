//! Chart matching commands: load a still, analyse its chart, preview, export the DCTL,
//! fit and save an OpenDRT preset.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lutmatrix_core::chart::detect::{find_lattice, Lattice, Orientation};
use lutmatrix_core::chart::encoding::{still_curves, StillGamut, StillSpace};
use lutmatrix_core::chart::image::{ChartImage, ImageInfo};
use lutmatrix_core::chart::odrt::{fit_opendrt_chart, OdrtChartFit, OdrtChartSettings, DISPLAYS};
use lutmatrix_core::chart::reference::{ChartEdition, PATCH_NAMES};
use lutmatrix_core::chart::{analyze, base64, preview_jpeg, ChartAnalysis, ChartSettings};
use lutmatrix_core::dctl::today_utc;
use lutmatrix_core::preset_store::{PresetStore, StoredPreset};
use lutmatrix_core::spaces::curves::LogCurve;
use lutmatrix_opendrt::tables::{LOOK_PRESET_LABELS, TONESCALE_PRESET_LABELS};
use serde::{Deserialize, Serialize};
use tauri::State;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// A loaded still and its detected lattice (detection depends only on the pixels).
struct Still {
    img: ChartImage,
    lattice: Option<Lattice>,
}

/// Managed state; cheap to clone into worker threads.
#[derive(Default, Clone)]
pub struct ChartState(Arc<ChartInner>);

#[derive(Default)]
pub struct ChartInner {
    stills: Mutex<HashMap<String, Arc<Still>>>,
    /// The analysis on screen (what Export / Copy / OpenDRT use).
    analysis: Mutex<Option<ChartAnalysis>>,
    /// The OpenDRT fit on screen (what Save uses).
    odrt: Mutex<Option<OdrtChartFit>>,
}

fn still(state: &ChartInner, path: &str) -> CmdResult<Arc<Still>> {
    if let Some(s) = state.stills.lock().unwrap().get(path) {
        return Ok(s.clone());
    }
    let img = ChartImage::load(Path::new(path)).map_err(err)?;
    let lattice = find_lattice(&img);
    let s = Arc::new(Still { img, lattice });
    let mut map = state.stills.lock().unwrap();
    // Keep at most a few stills in memory (a 6K float still is ~300 MB).
    if map.len() >= 3 {
        map.clear();
    }
    map.insert(path.to_string(), s.clone());
    Ok(s)
}

#[derive(Serialize)]
pub struct CurveEntry {
    curve: LogCurve,
    name: String,
}

#[derive(Serialize)]
pub struct ChartCatalog {
    curves: Vec<CurveEntry>,
    gamuts: Vec<(StillGamut, &'static str)>,
    orientations: Vec<(Orientation, &'static str)>,
    editions: Vec<(ChartEdition, &'static str)>,
    patch_names: Vec<&'static str>,
    looks: Vec<&'static str>,
    tonescales: Vec<&'static str>,
    displays: Vec<(usize, &'static str)>,
    extensions: Vec<&'static str>,
}

#[tauri::command]
pub fn chart_catalog() -> ChartCatalog {
    ChartCatalog {
        curves: still_curves().into_iter().map(|c| CurveEntry { curve: c, name: c.name() }).collect(),
        gamuts: StillGamut::ALL.iter().map(|g| (*g, g.label())).collect(),
        orientations: Orientation::ALL.iter().map(|o| (*o, o.label())).collect(),
        editions: ChartEdition::ALL.iter().map(|e| (*e, e.label())).collect(),
        patch_names: PATCH_NAMES.to_vec(),
        looks: LOOK_PRESET_LABELS.to_vec(),
        tonescales: TONESCALE_PRESET_LABELS.to_vec(),
        displays: DISPLAYS.iter().map(|d| (d.0, d.1)).collect(),
        extensions: lutmatrix_core::chart::image::IMAGE_EXTENSIONS.to_vec(),
    }
}

#[derive(Serialize)]
pub struct LoadedStill {
    path: String,
    file_name: String,
    info: ImageInfo,
    /// Patches found by detection (0 = no chart found).
    found: usize,
}

#[tauri::command]
pub async fn chart_load(path: String, state: State<'_, ChartState>) -> CmdResult<LoadedStill> {
    // An explicit load re-reads the file (it may have changed).
    state.0.stills.lock().unwrap().remove(&path);
    let st = state.0.clone();
    let p = path.clone();
    let s = tauri::async_runtime::spawn_blocking(move || still(&st, &p))
    .await
    .map_err(err)??;
    Ok(LoadedStill {
        file_name: file_name(&path),
        info: s.img.info.clone(),
        found: s.lattice.as_ref().map_or(0, |l| l.members.len()),
        path,
    })
}

#[derive(Deserialize)]
pub struct TargetRequest {
    path: String,
    space: Option<StillSpace>,
}

#[derive(Deserialize)]
pub struct ChartRequest {
    path: String,
    settings: ChartSettings,
    /// Another still whose chart is the target (else the published values).
    target: Option<TargetRequest>,
}

#[tauri::command]
pub async fn chart_analyze(req: ChartRequest, state: State<'_, ChartState>) -> CmdResult<ChartAnalysis> {
    let s = still(&state.0, &req.path)?;
    let target = match &req.target {
        Some(t) => Some((still(&state.0, &t.path)?, t.space)),
        None => None,
    };
    let mut settings = req.settings;
    let a = tauri::async_runtime::spawn_blocking(move || -> CmdResult<ChartAnalysis> {
        if let Some((ts, space)) = target {
            let tset = ChartSettings { edition: settings.edition, space, ..ChartSettings::default() };
            let ta = analyze(&ts.img, ts.lattice.as_ref(), &tset).map_err(|e| format!("target chart: {e}"))?;
            settings.target = ta.as_target();
        }
        analyze(&s.img, s.lattice.as_ref(), &settings).map_err(err)
    })
    .await
    .map_err(err)??;
    *state.0.analysis.lock().unwrap() = Some(a.clone());
    Ok(a)
}

#[derive(Serialize)]
pub struct Preview {
    data_url: String,
    width: usize,
    height: usize,
    /// Full-resolution pixels per preview pixel.
    scale: f64,
}

#[tauri::command]
pub async fn chart_preview(path: String, space: StillSpace, exposure_ratio: f64, state: State<'_, ChartState>) -> CmdResult<Preview> {
    let s = still(&state.0, &path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (jpg, w, h) = preview_jpeg(&s.img, &space, exposure_ratio, 1800);
        Preview {
            data_url: format!("data:image/jpeg;base64,{}", base64(&jpg)),
            width: w,
            height: h,
            scale: s.img.width as f64 / w.max(1) as f64,
        }
    })
    .await
    .map_err(err)
}

#[tauri::command]
pub fn chart_default_dctl_name(path: String) -> String {
    lutmatrix_core::chart::dctl::chart_dctl_file_name(&file_name(&path))
}

#[tauri::command]
pub fn chart_export_dctl(dctl_path: String, state: State<ChartState>) -> CmdResult<String> {
    let guard = state.0.analysis.lock().unwrap();
    let a = guard.as_ref().ok_or("nothing to export yet")?;
    let src = a.dctl(&today_utc()).ok_or("no correction to export (the fit failed)")?;
    let p = PathBuf::from(&dctl_path);
    let p = if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dctl")) { p } else { p.with_extension("dctl") };
    std::fs::write(&p, src).map_err(|e| format!("could not write {}: {e}", p.display()))?;
    Ok(p.display().to_string())
}

/// The on-screen matrix (XYZ) as text, plus the tone curve if any.
#[tauri::command]
pub fn chart_matrix_text(state: State<ChartState>) -> CmdResult<String> {
    let guard = state.0.analysis.lock().unwrap();
    let f = guard.as_ref().and_then(|a| a.fit.as_ref()).ok_or("no correction yet")?;
    let m = &f.m_xyz;
    let mut s = (0..3).map(|r| format!("{:.10} {:.10} {:.10}", m[(r, 0)], m[(r, 1)], m[(r, 2)])).collect::<Vec<_>>().join("\n");
    if let Some(t) = &f.tone {
        s.push_str(&format!(
            "\n# tone (linear DWG, before the matrix): power {:.6} {:.6} {:.6}, black {:.8} {:.8} {:.8}",
            t.power[0], t.power[1], t.power[2], t.black[0], t.black[1], t.black[2]
        ));
    }
    Ok(s)
}

#[tauri::command]
pub async fn chart_opendrt_fit(settings: OdrtChartSettings, state: State<'_, ChartState>) -> CmdResult<OdrtChartFit> {
    let a = state.0.analysis.lock().unwrap().clone().ok_or("analyse a chart first")?;
    let f = tauri::async_runtime::spawn_blocking(move || fit_opendrt_chart(&a, &settings)).await.map_err(err)??;
    *state.0.odrt.lock().unwrap() = Some(f.clone());
    Ok(f)
}

#[tauri::command]
pub fn chart_default_preset_name(path: String) -> String {
    let stem = Path::new(&path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!("Chart {stem}")
}

#[tauri::command]
pub fn chart_preset_save(name: String, replace: bool, state: State<ChartState>) -> CmdResult<String> {
    let guard = state.0.odrt.lock().unwrap();
    let f = guard.as_ref().ok_or("no OpenDRT fit to save")?;
    let p = StoredPreset::from_chart(f, Some(&name)).map_err(err)?;
    PresetStore::open_default().map_err(err)?.save(&p, replace).map_err(err)?;
    Ok(p.name)
}
