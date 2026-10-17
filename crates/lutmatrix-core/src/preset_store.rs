//! The user's local library of OpenDRT "LUT Match" presets.
//!
//! Presets are fitted from the user's own LUTs and are never part of the app or the repo
//! (DECISIONS.md D29). Each one is a JSON file in a per-user data folder:
//!
//! - macOS: `~/Library/Application Support/LUT Matrix/opendrt-presets/`
//! - Windows: `%APPDATA%\LUT Matrix\opendrt-presets\`
//! - Linux: `$XDG_DATA_HOME/lut-matrix/opendrt-presets/` (default `~/.local/share/…`)
//!
//! `LUTMATRIX_DATA_DIR` replaces the platform folder (the presets go in its
//! `opendrt-presets/`).

use std::path::{Path, PathBuf};

use lutmatrix_opendrt::writer::{sanitize_name, LutMatchPreset};
use lutmatrix_opendrt::{Config, LookParams};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::opendrt_match::OpenDrtMatch;

const FORMAT: &str = "lutmatrix-opendrt-preset";
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("no folder for app data (HOME / APPDATA not set); set LUTMATRIX_DATA_DIR")]
    NoDataDir,
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("{0}: not an OpenDRT preset or fit file ({1})")]
    BadFile(String, String),
    #[error("no preset called '{0}'")]
    NotFound(String),
    #[error("a preset called '{0}' already exists (replace it explicitly)")]
    Exists(String),
    #[error("preset name '{0}' has nothing a DCTL menu can show")]
    BadName(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io { path: path.display().to_string(), source }
}

/// One stored preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPreset {
    pub format: String,
    pub version: u32,
    /// Menu name (DCTL-safe).
    pub name: String,
    /// Milliseconds since 1970 (UTC); orders the presets in the Look Preset menu.
    pub created_unix_ms: u64,
    /// The LUT it was fitted to (file name).
    pub source_lut: String,
    pub look: LookParams,
    /// Fitted Display Grey Luminance, nits.
    pub grey_nits: f32,
    /// The full fit report, when fitted by this app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<OpenDrtMatch>,
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl StoredPreset {
    pub fn new(name: &str, source_lut: &str, look: LookParams, grey_nits: f32) -> Result<Self, StoreError> {
        let clean = sanitize_name(name);
        if clean.is_empty() {
            return Err(StoreError::BadName(name.into()));
        }
        Ok(StoredPreset {
            format: FORMAT.into(),
            version: FORMAT_VERSION,
            name: clean,
            created_unix_ms: now_unix(),
            source_lut: source_lut.into(),
            look,
            grey_nits,
            report: None,
        })
    }

    /// From an `opendrt fit` result; `name` defaults to "LUT Match <LUT name>".
    pub fn from_match(m: &OpenDrtMatch, name: Option<&str>) -> Result<Self, StoreError> {
        let lut = m.lut_name.clone().unwrap_or_default();
        let default = format!("LUT Match {}", stem(&lut));
        let mut p = StoredPreset::new(name.unwrap_or(&default), &lut, m.config.look, m.config.grey_nits * m.config.grey_scale)?;
        p.report = Some(m.clone());
        Ok(p)
    }

    pub fn created_date(&self) -> String {
        crate::dctl::date_utc((self.created_unix_ms / 1000) as i64)
    }

    /// For the DCTL writer.
    pub fn to_writer(&self) -> LutMatchPreset {
        LutMatchPreset { name: self.name.clone(), look: self.look, grey_nits: self.grey_nits, source: self.source_lut.clone() }
    }
}

fn stem(file: &str) -> String {
    Path::new(file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Read a preset from a file: a stored preset, an `opendrt fit` report, or a fit from the
/// earlier Python tool. `name` overrides the stored/default name.
pub fn import_file(path: &Path, name: Option<&str>) -> Result<StoredPreset, StoreError> {
    let text = std::fs::read_to_string(path).map_err(io(path))?;
    let bad = |why: String| StoreError::BadFile(path.display().to_string(), why);
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| bad(e.to_string()))?;
    if v.get("format").and_then(|f| f.as_str()) == Some(FORMAT) {
        let mut p: StoredPreset = serde_json::from_value(v).map_err(|e| bad(e.to_string()))?;
        if let Some(n) = name {
            p.name = sanitize_name(n);
            if p.name.is_empty() {
                return Err(StoreError::BadName(n.into()));
            }
        }
        return Ok(p);
    }
    if v.get("config").is_some() && v.get("fitted").is_some() {
        let m: OpenDrtMatch = serde_json::from_value(v).map_err(|e| bad(e.to_string()))?;
        return StoredPreset::from_match(&m, name);
    }
    if let Some(params) = v.get("params").and_then(|p| p.as_object()) {
        let mut look = Config::default().look;
        let mut grey = None;
        for (k, x) in params {
            let x = x.as_f64().ok_or_else(|| bad(format!("{k} is not a number")))? as f32;
            if k == "tn_Lg" {
                grey = Some(x);
            } else if !look.set(k.trim_start_matches('_'), x) {
                return Err(bad(format!("unknown OpenDRT parameter {k}")));
            }
        }
        let lut = v["lut"].as_str().unwrap_or("").to_string();
        let default = format!("LUT Match {}", stem(&lut));
        return StoredPreset::new(name.unwrap_or(&default), &lut, look, grey.ok_or_else(|| bad("no tn_Lg".into()))?);
    }
    Err(bad("unrecognised contents".into()))
}

/// The app's per-user data folder (presets, preferences): see the module docs.
pub fn data_dir() -> Result<PathBuf, StoreError> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    Ok(if let Some(d) = var("LUTMATRIX_DATA_DIR") {
        d
    } else if cfg!(target_os = "macos") {
        var("HOME").ok_or(StoreError::NoDataDir)?.join("Library/Application Support/LUT Matrix")
    } else if cfg!(windows) {
        var("APPDATA").ok_or(StoreError::NoDataDir)?.join("LUT Matrix")
    } else {
        var("XDG_DATA_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".local/share")))
            .ok_or(StoreError::NoDataDir)?
            .join("lut-matrix")
    })
}

/// The preset folder.
pub struct PresetStore {
    dir: PathBuf,
}

impl PresetStore {
    /// The per-user folder (see the module docs).
    pub fn default_dir() -> Result<PathBuf, StoreError> {
        Ok(data_dir()?.join("opendrt-presets"))
    }

    pub fn open_default() -> Result<Self, StoreError> {
        Ok(PresetStore { dir: Self::default_dir()? })
    }

    /// A store in `dir` (the `opendrt-presets` folder itself).
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        PresetStore { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every preset, in menu order (oldest first). Files that are not presets are
    /// reported in the second list instead of failing the whole listing.
    pub fn list(&self) -> Result<(Vec<StoredPreset>, Vec<StoreError>), StoreError> {
        let mut out = Vec::new();
        let mut bad = Vec::new();
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((out, bad)),
            Err(e) => return Err(io(&self.dir)(e)),
        };
        for entry in rd {
            let path = entry.map_err(io(&self.dir))?.path();
            if path.extension().is_some_and(|e| e == "json") {
                match import_file(&path, None) {
                    Ok(p) if p.format == FORMAT => out.push(p),
                    Ok(_) => bad.push(StoreError::BadFile(path.display().to_string(), "not a stored preset".into())),
                    Err(e) => bad.push(e),
                }
            }
        }
        out.sort_by(|a, b| a.created_unix_ms.cmp(&b.created_unix_ms).then_with(|| a.name.cmp(&b.name)));
        Ok((out, bad))
    }

    fn path_for(&self, name: &str) -> PathBuf {
        let slug: String = sanitize_name(name)
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect();
        let slug = slug.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
        self.dir.join(format!("{slug}.json"))
    }

    pub fn get(&self, name: &str) -> Result<StoredPreset, StoreError> {
        let want = sanitize_name(name).to_lowercase();
        self.list()?
            .0
            .into_iter()
            .find(|p| p.name.to_lowercase() == want)
            .ok_or_else(|| StoreError::NotFound(name.into()))
    }

    /// Save `p`. With `replace`, an existing preset of the same name is overwritten (and
    /// keeps its place in the menu); otherwise that is an error.
    pub fn save(&self, p: &StoredPreset, replace: bool) -> Result<PathBuf, StoreError> {
        let mut p = p.clone();
        if let Ok(old) = self.get(&p.name) {
            if !replace {
                return Err(StoreError::Exists(p.name));
            }
            p.created_unix_ms = old.created_unix_ms;
            self.remove(&old.name)?;
        }
        std::fs::create_dir_all(&self.dir).map_err(io(&self.dir))?;
        let path = self.path_for(&p.name);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(&p).expect("serialisable") + "\n";
        std::fs::write(&tmp, body).map_err(io(&tmp))?;
        std::fs::rename(&tmp, &path).map_err(io(&path))?;
        Ok(path)
    }

    pub fn remove(&self, name: &str) -> Result<(), StoreError> {
        let want = sanitize_name(name).to_lowercase();
        let rd = std::fs::read_dir(&self.dir).map_err(|_| StoreError::NotFound(name.into()))?;
        for entry in rd {
            let path = entry.map_err(io(&self.dir))?.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(p) = import_file(&path, None) {
                    if p.format == FORMAT && p.name.to_lowercase() == want {
                        return std::fs::remove_file(&path).map_err(io(&path));
                    }
                }
            }
        }
        Err(StoreError::NotFound(name.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lutmatrix-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn save_list_get_replace_remove() {
        let dir = tmp("crud");
        let s = PresetStore::at(&dir);
        assert!(s.list().unwrap().0.is_empty(), "missing folder = empty library");
        let look = LookParams::preset(2).unwrap();
        let mut a = StoredPreset::new("LUT Match {A}", "a.cube", look, 8.0).unwrap();
        a.created_unix_ms = 10;
        let mut b = StoredPreset::new("LUT Match B", "b.cube", look, 12.0).unwrap();
        b.created_unix_ms = 5;
        s.save(&a, false).unwrap();
        s.save(&b, false).unwrap();
        let names: Vec<String> = s.list().unwrap().0.into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["LUT Match B", "LUT Match A"]);
        assert!(matches!(s.save(&a, false), Err(StoreError::Exists(_))));
        let mut a2 = a.clone();
        a2.grey_nits = 9.0;
        a2.created_unix_ms = 99;
        s.save(&a2, true).unwrap();
        let got = s.get("lut match a").unwrap();
        assert_eq!((got.grey_nits, got.created_unix_ms), (9.0, 10), "replaced in place");
        s.remove("LUT Match B").unwrap();
        assert_eq!(s.list().unwrap().0.len(), 1);
        assert!(matches!(s.remove("nope"), Err(StoreError::NotFound(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn imports_python_fits_and_round_trips() {
        let dir = tmp("import");
        std::fs::create_dir_all(&dir).unwrap();
        let py = dir.join("fit.json");
        std::fs::write(&py, r#"{"lut": "x.cube", "params": {"_tn_con": 1.2, "_cwp": 3, "tn_Lg": 7.5}}"#).unwrap();
        let p = import_file(&py, None).unwrap();
        assert_eq!((p.name.as_str(), p.look.tn_con, p.look.cwp, p.grey_nits), ("LUT Match x", 1.2, 3.0, 7.5));
        let s = PresetStore::at(dir.join("store"));
        let path = s.save(&p, false).unwrap();
        let back = import_file(&path, Some("Renamed")).unwrap();
        assert_eq!((back.name.as_str(), back.look, back.grey_nits), ("Renamed", p.look, 7.5));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
