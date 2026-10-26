// Typed wrappers around the Rust commands (app/src-tauri/src/lib.rs).
// In a plain browser (`npm run dev` without Tauri) a development mock answers instead.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let mockInvoke: ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null = null;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return tauriInvoke<T>(cmd, args);
  if (import.meta.env.DEV) {
    mockInvoke ??= (await import("./mock")).mockInvoke;
    return mockInvoke(cmd, args) as Promise<T>;
  }
  throw new Error("LUT Matrix must run inside its app window");
}

/* ---------- types (mirror the Rust serde output) ---------- */

export type CameraId = string;
export type Cat = "Bradford" | "Cat02" | "None";
export type Gamut = "rec709" | "p3_d65" | "p3_dci" | "rec2020";
export type Transfer = { kind: "gamma"; gamma: number } | { kind: "pq"; peak_nits: number };
export type Confidence = "high" | "medium" | "low" | "none";
export type Metric = "auto" | "delta_e2000" | "delta_e_itp";

export interface InputSpace {
  id: CameraId;
  arri_ei: number;
  white_cat: Cat;
}
export interface OutputSpace {
  gamut: Gamut;
  transfer: Transfer;
  sdr_peak_nits: number;
}
export interface Guess<T> {
  value: T;
  confidence: Confidence;
  certainty: number;
  reasons: string[];
  alternatives: [T, number][];
}
export interface Detection {
  input: Guess<InputSpace>;
  output_gamut: Guess<Gamut>;
  output_transfer: Guess<Transfer>;
  probe: {
    input_code: number;
    output_code: [number, number, number];
    output_nits: number;
    expected_nits: number;
    stops_from_expected: number;
  } | null;
  middle_gray_hint: [number, string] | null;
  notes: string[];
}
export interface LoadedLut {
  path: string;
  file_name: string;
  describe: string;
  title: string | null;
  output_range: [number, number];
  warnings: string[];
  detection: Detection;
  input: InputSpace;
  input_name: string;
  output: OutputSpace;
}

export interface Settings {
  interpolation: "tetrahedral" | "trilinear";
  anchor: "scene_linear" | "display_target";
  scene_middle_gray: number;
  display_middle_gray_nits: number | null;
  hues: number;
  saturations: number[];
  ring_limit: "camera_and_display" | "camera";
  bracket_stops: number;
  normalize_exposure: boolean;
  white_mode: "d65" | "preserve_lut";
  cat: Cat;
  metric: Metric;
  refine: boolean;
  neutral_weight: number | null;
  lock_neutral: boolean;
  clip_low_code: number;
  clip_high_margin: number;
}

export const defaultSettings: Settings = {
  interpolation: "tetrahedral",
  anchor: "scene_linear",
  scene_middle_gray: 0.18,
  display_middle_gray_nits: null,
  hues: 36,
  saturations: [0.25, 0.5, 0.75, 0.95],
  ring_limit: "camera_and_display",
  bracket_stops: 0,
  normalize_exposure: true,
  white_mode: "d65",
  cat: "Bradford",
  metric: "auto",
  refine: true,
  neutral_weight: null,
  lock_neutral: false,
  clip_low_code: 0.0005,
  clip_high_margin: 0.001,
};

export interface HueError {
  hue_deg: number;
  name: string;
  mean: number;
  max: number;
}
export interface FitStats {
  metric: Metric;
  n: number;
  mean: number;
  rms: number;
  p95: number;
  max: number;
  neutral: number;
  neutral_duv: number;
  worst_hues: HueError[];
}
export type SampleKind = { kind: "neutral" } | { kind: "hue"; hue_deg: number; saturation: number };
export interface SampleReport {
  kind: SampleKind;
  stops: number;
  status: "ok" | "outside_camera_gamut" | "outside_lut_domain" | "clipped_output";
  error_ls: number | null;
  error_refined: number | null;
  swatch_input: string;
  swatch_lut: string;
  swatch_target: string;
  swatch_matrix: string;
}
export interface Diagnostic {
  severity: "info" | "warning";
  code: string;
  message: string;
}
export interface ReferenceReport {
  name: string;
  input_name: string;
  output_name: string;
  stats_alone: FitStats;
  stats_colorimetric: FitStats;
  gain_percent: number;
  gray_offset_stops: number;
}
export type Mat3 = [[number, number, number], [number, number, number], [number, number, number]];
export interface Analysis {
  version: string;
  input: InputSpace;
  input_name: string;
  output: OutputSpace;
  output_name: string;
  settings: Settings;
  metric: Metric;
  m_xyz: Mat3;
  m_xyz_ls: Mat3;
  m_xyz_refined: Mat3 | null;
  stats_ls: FitStats;
  stats_refined: FitStats | null;
  lut_white_xy: { x: number; y: number };
  lut_white_duv_from_d65: number;
  anchor_stops: number;
  display_middle_gray_nits: number;
  display_middle_gray_source: string;
  gray_out_nits: number;
  stops_from_display_middle_gray: number;
  contrast_at_gray: number;
  samples_used: number;
  samples_skipped: [string, number][];
  samples: SampleReport[];
  diagnostics: Diagnostic[];
  reference: ReferenceReport | null;
}

export interface CameraEntry {
  id: CameraId;
  name: string;
  gamut_name: string;
  has_ei: boolean;
}
export interface Catalog {
  cameras: CameraEntry[];
  gamuts: [Gamut, string][];
  arri_eis: number[];
  pq_peaks: number[];
}
export interface AppInfo {
  version: string;
  os: string;
  data_dir: string | null;
  presets_dir: string | null;
}

/** How the LUT's gray-axis cast is treated when matching OpenDRT (D33). */
export type NeutralGray = "off" | "middle_gray" | "exposure" | "white_gain";
/** Diffuse white (a 90% reflector), stops over 18% gray. */
export const DIFFUSE_WHITE_STOPS = Math.log2(0.9 / 0.18);
export interface WhiteGain {
  stops: number;
  duv: number;
  /** Display-linear R, G, B gain (G = 1 unless scaled down for headroom). */
  linear: [number, number, number];
  /** Factor on all channels so no LUT output exceeds signal 1 (1 = none needed). */
  headroom?: number;
  /** The same on the encoded signal (pure power displays only). */
  signal: [number, number, number] | null;
}
export interface GrayPoint {
  stops: number;
  nits: number;
  l: number;
  a: number;
  b: number;
  /** u′v′ distance from D65. */
  duv: number;
}
export interface MatchSettings {
  interpolation: "tetrahedral" | "trilinear";
  metric: Metric;
  ring_gamut: CameraId | null;
  freeze: string[];
  regularization: number;
  max_iterations: number;
  starts: number;
  creative_whites: number[];
  auto_freeze_brilliance: boolean;
  neutral_gray: NeutralGray;
  neutral_amount: number;
  white_stops: number;
  threads: number;
}
export const defaultMatchSettings: MatchSettings = {
  interpolation: "tetrahedral",
  metric: "auto",
  ring_gamut: null,
  freeze: [],
  regularization: 0,
  max_iterations: 150,
  starts: 3,
  creative_whites: [2, 1, 3],
  auto_freeze_brilliance: true,
  neutral_gray: "off",
  neutral_amount: 1,
  white_stops: DIFFUSE_WHITE_STOPS,
  threads: 0,
};
export interface Score {
  mean: number;
  weighted_mean: number;
  p95: number;
  max: number;
  /** Mean over the gray ramp only (absent in reports saved before 0.2). */
  gray_mean?: number;
}
export interface OpenDrtMatch {
  lut_name: string | null;
  input: InputSpace;
  output: OutputSpace;
  metric: Metric;
  config: { grey_nits: number; grey_scale: number; look: Record<string, number> };
  start: { look: number; tonescale: number; creative_white: number; description: string; score: number };
  stock: Score;
  best_preset: Score;
  fitted: Score;
  samples_used: number;
  samples_total: number;
  frozen: string[];
  /** The LUT's gray axis before correction, whole stops (absent in older reports). */
  gray_axis?: GrayPoint[];
  /** The gray axis after the correction (empty when off). */
  gray_axis_corrected?: GrayPoint[];
  neutral_gray?: NeutralGray;
  neutral_amount?: number;
  white_gain?: WhiteGain | null;
  safety: {
    finite_in_range: boolean;
    reversals: { colour: string; at_stops: number; drop_percent: number }[];
    roughness: number;
    stock_roughness: number;
  };
  notes: string[];
  seconds: number;
}
export interface PresetRow {
  name: string;
  created: string;
  source_lut: string;
  grey_nits: number;
  error: [string, number] | null;
  stock_error: number | null;
  notes: string[];
}
export interface PresetList {
  dir: string;
  presets: PresetRow[];
  problems: string[];
}

/* ---------- batch: every negative × every print ---------- */

export type Stage = "negative" | "print";
export interface StockNames {
  looks: Record<string, string>;
  variants: Record<string, string>;
  flags: Record<string, string>;
}
export interface BatchItem {
  id: number;
  stage: Stage;
  path: string;
  file: string;
  name: string;
  name_edited: boolean;
  input: InputSpace;
  output: OutputSpace;
  include: boolean;
  variant: string | null;
  variant_word: string | null;
  flags: string[];
}
export interface PairCheck {
  gray_nits: number;
  negative_out: [number, number];
  print_in: [number, number];
  outside_percent: number;
  ok: boolean;
  problems: string[];
}
export interface PairSummary {
  name: string;
  negative: string;
  print: string;
  check: PairCheck;
  error: string | null;
  metric: string | null;
  stock: number | null;
  best_preset: number | null;
  fitted: number | null;
  p95: number | null;
  gray: number | null;
  grey_nits: number | null;
  start: string | null;
  safety_ok: boolean | null;
  notes: string[];
}
export interface BatchView {
  items: BatchItem[];
  results: PairSummary[];
  running: boolean;
  folder: string | null;
  names_path: string | null;
}
export interface BatchItemUpdate {
  id: number;
  name?: string;
  stage?: Stage;
  input?: InputSpace;
  output?: OutputSpace;
  include?: boolean;
}
export interface BatchProgress {
  done: number;
  total: number;
  pair: PairSummary;
}

/** Calls `f` for every `batch-pair` event (a pair finished); returns the unlisten function. */
export async function onBatchPair(f: (p: BatchProgress) => void): Promise<() => void> {
  if (!inTauri) {
    const h = (e: Event) => f((e as CustomEvent<BatchProgress>).detail);
    window.addEventListener("batch-pair", h);
    return () => window.removeEventListener("batch-pair", h);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<BatchProgress>("batch-pair", (e) => f(e.payload));
}

/* ---------- commands ---------- */

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  catalog: () => invoke<Catalog>("catalog"),
  loadLut: (path: string) => invoke<LoadedLut>("load_lut", { path }),
  initialFile: () => invoke<string | null>("initial_file"),
  analyze: (req: {
    path: string;
    input: InputSpace;
    output: OutputSpace;
    settings: Settings;
    middle_gray_hint: number | null;
    through: { path: string; input: InputSpace; output: OutputSpace } | null;
  }) => invoke<Analysis>("analyze", { req }),
  defaultDctlName: (path: string) => invoke<string>("default_dctl_name", { path }),
  exportDctl: (dctlPath: string) => invoke<{ dctl: string; json: string; txt: string }>("export_dctl", { dctlPath }),
  matrixText: () => invoke<string>("matrix_text"),
  loadPrefs: () => invoke<unknown>("load_prefs"),
  savePrefs: (prefs: unknown) => invoke<void>("save_prefs", { prefs }),
  opendrtFit: (req: { path: string; input: InputSpace; output: OutputSpace; settings: MatchSettings }) =>
    invoke<OpenDrtMatch>("opendrt_fit", { req }),
  presetsList: () => invoke<PresetList>("presets_list"),
  defaultPresetName: (path: string) => invoke<string>("default_preset_name", { path }),
  presetExists: (name: string) => invoke<boolean>("preset_exists", { name }),
  presetSave: (name: string, replace: boolean) => invoke<string>("preset_save", { name, replace }),
  presetRemove: (name: string) => invoke<void>("preset_remove", { name }),
  presetRename: (name: string, newName: string) => invoke<string>("preset_rename", { name, newName }),
  presetImport: (paths: string[], replace: boolean) => invoke<string[]>("preset_import", { paths, replace }),
  presetExport: (name: string, path: string) => invoke<void>("preset_export", { name, path }),
  opendrtBuild: (names: string[], path: string) => invoke<string>("opendrt_build", { names, path }),
  batchState: () => invoke<BatchView>("batch_state"),
  batchAdd: (paths: string[], stage: Stage | null = null) => invoke<BatchView>("batch_add", { paths, stage }),
  batchUpdate: (updates: BatchItemUpdate[]) => invoke<BatchView>("batch_update", { updates }),
  batchRemove: (ids: number[]) => invoke<BatchView>("batch_remove", { ids }),
  stockNamesGet: () => invoke<StockNames>("stock_names_get"),
  stockNamesSet: (names: StockNames) => invoke<BatchView>("stock_names_set", { names }),
  stockNamesImport: (path: string) => invoke<StockNames>("stock_names_import", { path }),
  batchRun: (req: { settings: MatchSettings; check_only: boolean; fit_mismatched: boolean }) =>
    invoke<BatchView>("batch_run", { req }),
  batchCancel: () => invoke<void>("batch_cancel"),
  batchResult: (name: string) => invoke<OpenDrtMatch>("batch_result", { name }),
  batchSave: (names: string[], replace: boolean) => invoke<string[]>("batch_save", { names, replace }),
  batchExisting: (names: string[]) => invoke<string[]>("batch_existing", { names }),
  batchExportCsv: (path: string) => invoke<void>("batch_export_csv", { path }),
};
