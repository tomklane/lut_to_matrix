// App-wide state (Svelte 5 runes). Views read and write `app`; the remembered parts are
// saved to prefs.json in the user's app data folder (see `prefsSnapshot`).

import {
  api,
  defaultMatchSettings,
  defaultSettings,
  onBatchPair,
  type Analysis,
  type AppInfo,
  type BatchView,
  type Catalog,
  type InputSpace,
  type LoadedLut,
  type MatchSettings,
  type OpenDrtMatch,
  type OutputSpace,
  type PresetList,
  type Settings,
} from "./api";
import { dirName } from "./platform";

export type View = "matrix" | "opendrt" | "batch" | "presets";

export interface Through {
  path: string;
  fileName: string;
  input: InputSpace;
}

export const DI_INPUT: InputSpace = { id: "da_vinci_intermediate_dwg", arri_ei: 800, white_cat: "Bradford" };

class AppState {
  info = $state<AppInfo | null>(null);
  catalog = $state<Catalog | null>(null);
  view = $state<View>("matrix");

  lut = $state<LoadedLut | null>(null);
  loadingLut = $state(false);
  input = $state<InputSpace | null>(null);
  output = $state<OutputSpace | null>(null);
  settings = $state<Settings>(structuredClone(defaultSettings));
  through = $state<Through | null>(null);
  lastDir = $state<string>("");

  analysis = $state<Analysis | null>(null);
  analyzing = $state(false);
  analysisError = $state<string | null>(null);

  match = $state<MatchSettings>(structuredClone(defaultMatchSettings));
  odrt = $state<OpenDrtMatch | null>(null);
  odrtForLut = $state<string | null>(null);
  odrtRunning = $state(false);
  odrtError = $state<string | null>(null);

  presets = $state<PresetList | null>(null);

  /** The batch (negatives × prints) as the backend has it. */
  batch = $state<BatchView | null>(null);
  /** The run in progress: pairs done / total, when it started, the latest pair. */
  batchProgress = $state<{ done: number; total: number; started: number; last: string } | null>(null);
  batchFitMismatched = $state(false);

  toast = $state<{ text: string; kind: "ok" | "error" } | null>(null);
  showAbout = $state(false);
  dragging = $state(false);

  private toastTimer: ReturnType<typeof setTimeout> | undefined;

  notify(text: string, kind: "ok" | "error" = "ok") {
    this.toast = { text, kind };
    clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => (this.toast = null), kind === "error" ? 6000 : 3000);
  }

  async openLut(path: string) {
    this.loadingLut = true;
    try {
      const l = await api.loadLut(path);
      this.lut = l;
      this.input = l.input;
      this.output = l.output;
      this.analysis = null;
      this.analysisError = null;
      this.lastDir = dirName(path);
      if (this.odrtForLut !== path) {
        this.odrt = null;
        this.odrtError = null;
      }
    } catch (e) {
      this.notify(String(e), "error");
    } finally {
      this.loadingLut = false;
    }
  }

  /** Load the batch and follow its progress events (once, at start-up). */
  async initBatch() {
    try {
      this.batch = await api.batchState();
    } catch (e) {
      this.notify(String(e), "error");
    }
    await onBatchPair((p) => {
      if (this.batch) this.batch.results = [...this.batch.results.filter((r) => r.name !== p.pair.name), p.pair];
      if (this.batchProgress) this.batchProgress = { ...this.batchProgress, done: p.done, total: p.total, last: p.pair.name };
    });
  }

  async addToBatch(paths: string[], stage: "negative" | "print" | null = null) {
    const luts = paths.filter((p) => /\.(cube|3dl|dat)$/i.test(p));
    if (!luts.length) {
      this.notify("Drop .cube, .3dl or .dat LUTs", "error");
      return;
    }
    try {
      this.batch = await api.batchAdd(luts, stage);
      this.lastDir = dirName(luts[0]);
    } catch (e) {
      this.notify(String(e), "error");
      this.batch = await api.batchState();
    }
  }

  async refreshPresets() {
    try {
      this.presets = await api.presetsList();
    } catch (e) {
      this.notify(String(e), "error");
    }
  }

  /** What is remembered between sessions. */
  prefsSnapshot() {
    return {
      version: 1,
      view: this.view,
      settings: $state.snapshot(this.settings),
      match: $state.snapshot(this.match),
      through: this.through ? $state.snapshot(this.through) : null,
      lastDir: this.lastDir,
    };
  }

  applyPrefs(p: unknown) {
    if (!p || typeof p !== "object") return;
    const prefs = p as Record<string, unknown>;
    if (["matrix", "opendrt", "batch", "presets"].includes(prefs.view as string)) this.view = prefs.view as View;
    if (prefs.settings && typeof prefs.settings === "object")
      this.settings = { ...structuredClone(defaultSettings), ...(prefs.settings as Partial<Settings>) };
    if (prefs.match && typeof prefs.match === "object")
      this.match = { ...structuredClone(defaultMatchSettings), ...(prefs.match as Partial<MatchSettings>) };
    if (prefs.through && typeof prefs.through === "object") this.through = prefs.through as Through;
    if (typeof prefs.lastDir === "string") this.lastDir = prefs.lastDir;
  }
}

export const app = new AppState();

/* ---------- display helpers ---------- */

export function metricLabel(m: string): string {
  return m === "delta_e_itp" ? "ΔE ITP" : "ΔE2000";
}

export function transferLabel(o: OutputSpace): string {
  return o.transfer.kind === "pq" ? `PQ ${o.transfer.peak_nits} nits` : `Gamma ${o.transfer.gamma.toFixed(1)}`;
}

export function gamutLabel(g: string, catalog: Catalog | null): string {
  return catalog?.gamuts.find(([id]) => id === g)?.[1] ?? g;
}

export function cameraName(i: InputSpace, catalog: Catalog | null): string {
  const c = catalog?.cameras.find((c) => c.id === i.id);
  if (!c) return i.id;
  return c.has_ei ? `${c.name} (EI ${i.arri_ei})` : c.name;
}

export function fmt(n: number, d = 2): string {
  return Number.isFinite(n) ? n.toFixed(d) : "—";
}
