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
  type ChartAnalysis,
  type ChartCatalog,
  type ChartFitSettings,
  type ChartPreview,
  type ChartSettings,
  type LoadedStill,
  type OdrtChartFit,
  type OdrtChartSettings,
  defaultChartFit,
  defaultOdrtChart,
  type InputSpace,
  type LoadedLut,
  type MatchSettings,
  type OpenDrtMatch,
  type OutputSpace,
  type PresetList,
  type Settings,
} from "./api";
import { dirName } from "./platform";

export type View = "matrix" | "opendrt" | "chart" | "batch" | "presets";

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

  /* Chart matching. */
  chartCatalog = $state<ChartCatalog | null>(null);
  still = $state<LoadedStill | null>(null);
  loadingStill = $state(false);
  /** Per-still choices (not remembered): encoding override, orientation, corners, excluded patches. */
  chart = $state<ChartSettings>(freshChartSettings(structuredClone(defaultChartFit)));
  /** The target: null = the published chart, else another still. */
  chartTarget = $state<LoadedStill | null>(null);
  chartAnalysis = $state<ChartAnalysis | null>(null);
  chartAnalyzing = $state(false);
  chartError = $state<string | null>(null);
  chartPreview = $state<ChartPreview | null>(null);
  odrtChart = $state<OdrtChartSettings>(structuredClone(defaultOdrtChart));
  odrtChartFit = $state<OdrtChartFit | null>(null);
  odrtChartRunning = $state(false);
  odrtChartError = $state<string | null>(null);

  async openStill(path: string) {
    this.loadingStill = true;
    try {
      const s = await api.chartLoad(path);
      this.still = s;
      // Fresh per-still choices, same fit settings.
      this.chart = freshChartSettings($state.snapshot(this.chart.fit), this.chart.edition);
      this.chartAnalysis = null;
      this.chartError = null;
      this.chartPreview = null;
      this.odrtChartFit = null;
      this.odrtChartError = null;
      this.lastDir = dirName(path);
      this.view = "chart";
      if (!s.found) this.notify("No chart found automatically: click its four corner patches to place it", "error");
    } catch (e) {
      this.notify(String(e), "error");
    } finally {
      this.loadingStill = false;
    }
  }

  async openTargetStill(path: string) {
    try {
      this.chartTarget = await api.chartLoad(path);
    } catch (e) {
      this.notify(String(e), "error");
    }
  }

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

  async addToBatch(paths: string[], stage: "negative" | "print" | "full" | null = null) {
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
      chartFit: $state.snapshot(this.chart.fit),
      chartEdition: this.chart.edition,
      odrtChart: $state.snapshot(this.odrtChart),
      through: this.through ? $state.snapshot(this.through) : null,
      lastDir: this.lastDir,
    };
  }

  applyPrefs(p: unknown) {
    if (!p || typeof p !== "object") return;
    const prefs = p as Record<string, unknown>;
    if (["matrix", "opendrt", "chart", "batch", "presets"].includes(prefs.view as string)) this.view = prefs.view as View;
    if (prefs.chartFit && typeof prefs.chartFit === "object")
      this.chart.fit = { ...structuredClone(defaultChartFit), ...(prefs.chartFit as Partial<ChartFitSettings>) };
    if (prefs.chartEdition === "after2014" || prefs.chartEdition === "before2014") this.chart.edition = prefs.chartEdition;
    if (prefs.odrtChart && typeof prefs.odrtChart === "object")
      this.odrtChart = { ...structuredClone(defaultOdrtChart), ...(prefs.odrtChart as Partial<OdrtChartSettings>) };
    if (prefs.settings && typeof prefs.settings === "object")
      this.settings = { ...structuredClone(defaultSettings), ...(prefs.settings as Partial<Settings>) };
    if (prefs.match && typeof prefs.match === "object")
      this.match = { ...structuredClone(defaultMatchSettings), ...(prefs.match as Partial<MatchSettings>) };
    if (prefs.through && typeof prefs.through === "object") this.through = prefs.through as Through;
    if (typeof prefs.lastDir === "string") this.lastDir = prefs.lastDir;
  }
}

function freshChartSettings(fit: ChartFitSettings, edition: ChartSettings["edition"] = "after2014"): ChartSettings {
  return { edition, space: null, orientation: null, corners: null, sample_size: 0.4, exclude: [], clip_fraction: 0.02, mirrors: false, fit };
}

export const app = new AppState();

export function curveKey(c: { curve: string; ei?: number }): string {
  return c.curve + (c.ei ? `:${c.ei}` : "");
}

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
