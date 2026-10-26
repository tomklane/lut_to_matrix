// Development-only stand-in for the Rust backend, so the UI can be worked on in a plain
// browser (`npm run dev`). It answers from JSON files in app/dev-fixtures/ (git-ignored:
// produce them with `lutmatrix inspect --json`, `fit --json` and `opendrt fit --json`).
// Never bundled in production builds.

import type { PresetRow } from "./api";

const fixtures = new Map<string, unknown>();
async function fx(name: string): Promise<any> {
  if (!fixtures.has(name)) {
    const r = await fetch(`/dev-fixtures/${name}`);
    if (!r.ok) throw new Error(`dev fixture ${name} missing (see app/src/lib/mock.ts)`);
    fixtures.set(name, await r.json());
  }
  return structuredClone(fixtures.get(name));
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const params = new URLSearchParams(location.search);

const cameras: [string, string, string, boolean][] = [
  ["arri_log_c3_awg3", "ARRI LogC3 / AWG3", "ARRI Wide Gamut 3", true],
  ["arri_log_c4_awg4", "ARRI LogC4 / AWG4", "ARRI Wide Gamut 4", false],
  ["sony_s_log3_s_gamut3", "Sony S-Log3 / S-Gamut3", "S-Gamut3", false],
  ["sony_s_log3_s_gamut3_cine", "Sony S-Log3 / S-Gamut3.Cine", "S-Gamut3.Cine", false],
  ["red_log3_g10_rwg", "RED Log3G10 / RWG", "REDWideGamutRGB", false],
  ["canon_log2_cinema_gamut", "Canon Log 2 / Cinema Gamut", "Cinema Gamut", false],
  ["canon_log3_cinema_gamut", "Canon Log 3 / Cinema Gamut", "Cinema Gamut", false],
  ["panasonic_v_log_v_gamut", "Panasonic V-Log / V-Gamut", "V-Gamut", false],
  ["bmd_film_gen5", "Blackmagic Film Gen 5", "BMD WG Gen 5", false],
  ["fuji_f_log_f_gamut", "Fujifilm F-Log / F-Gamut", "F-Gamut", false],
  ["fuji_f_log2_f_gamut", "Fujifilm F-Log2 / F-Gamut", "F-Gamut", false],
  ["da_vinci_intermediate_dwg", "DaVinci Intermediate / DWG", "DaVinci Wide Gamut", false],
  ["aces_cct_ap1", "ACEScct / AP1", "ACES AP1", false],
  ["aces2065_ap0", "ACES2065-1 (linear AP0)", "ACES AP0", false],
];

let presets: PresetRow[] = params.has("empty")
  ? []
  : [
      { name: "LUT Match K1S1", created: "2026-10-17", source_lut: "AlexaV3_K1S1_LogC2Video_Rec709_EE_davinci3d.cube", grey_nits: 10.84, error: ["ΔE2000", 0.86], stock_error: 2.8, notes: [] },
      { name: "LUT Match Ulook4b", created: "2026-10-17", source_lut: "LogC33_Ulook4b_Rec709.cube", grey_nits: 4.45, error: ["ΔE2000", 2.3], stock_error: 10.47, notes: [] },
      { name: "LUT Match 5207 Pull2 2393", created: "2026-10-17", source_lut: "ACES2065_to_5207_pull2_K2393_D65_709G24.cube", grey_nits: 9.49, error: ["ΔE2000", 2.37], stock_error: 4.9, notes: ["luminance fell with rising exposure for 7 colour(s); refitted with brilliance frozen"] },
    ];

export async function mockInvoke(cmd: string, args: Record<string, any> = {}): Promise<unknown> {
  switch (cmd) {
    case "app_info":
      return { version: "0.1.0", os: params.get("os") ?? "macos", data_dir: "/Users/you/Library/Application Support/LUT Matrix", presets_dir: "/Users/you/Library/Application Support/LUT Matrix/opendrt-presets" };
    case "catalog":
      return {
        cameras: cameras.map(([id, name, gamut_name, has_ei]) => ({ id, name, gamut_name, has_ei })),
        gamuts: [["rec709", "Rec.709"], ["p3_d65", "P3-D65"], ["p3_dci", "P3-DCI"], ["rec2020", "Rec.2020"]],
        arri_eis: [160, 200, 250, 320, 400, 500, 640, 800, 1000, 1280, 1600],
        pq_peaks: [100, 300, 600, 1000, 2000, 4000, 10000],
      };
    case "load_prefs":
      return null;
    case "save_prefs":
      return null;
    case "initial_file":
      return params.has("lut") ? "/Users/you/LUTs/AlexaV3_K1S1_LogC2Video_Rec709_EE_davinci3d.cube" : null;
    case "load_lut": {
      const d = (await fx("inspect.json"))[0];
      await sleep(150);
      return {
        path: args.path,
        file_name: String(args.path).split(/[\\/]/).pop(),
        describe: d.lut,
        title: d.title,
        output_range: [0.0, 1.0],
        warnings: [],
        detection: d.detection,
        input: d.detection.input.value,
        input_name: "ARRI LogC3 / AWG3 (EI 800)",
        output: { gamut: d.detection.output_gamut.value, transfer: d.detection.output_transfer.value, sdr_peak_nits: 100 },
      };
    }
    case "analyze":
      await sleep(400);
      return fx(args.req.through ? "analysis_through.json" : "analysis.json");
    case "default_dctl_name":
      return "AlexaV3_K1S1_LogC2Video_Rec709_EE_davinci3d_MXYZ.dctl";
    case "export_dctl":
      return { dctl: args.dctlPath, json: "", txt: "" };
    case "matrix_text":
      return "1 0 0\n0 1 0\n0 0 1";
    case "opendrt_fit":
      await sleep(2500);
      return fx("odrt.json");
    case "presets_list":
      return { dir: "/Users/you/Library/Application Support/LUT Matrix/opendrt-presets", presets, problems: [] };
    case "default_preset_name":
      return "LUT Match AlexaV3_K1S1_LogC2Video_Rec709_EE_davinci3d";
    case "preset_exists":
      return presets.some((p) => p.name.toLowerCase() === String(args.name).toLowerCase());
    case "preset_save":
      presets = [...presets.filter((p) => p.name !== args.name), { name: args.name, created: "2026-10-17", source_lut: "K1S1.cube", grey_nits: 10.84, error: ["ΔE2000", 0.86], stock_error: 2.8, notes: [] }];
      return args.name;
    case "preset_remove":
      presets = presets.filter((p) => p.name !== args.name);
      return null;
    case "preset_rename":
      presets = presets.map((p) => (p.name === args.name ? { ...p, name: args.newName } : p));
      return args.newName;
    case "batch_state":
    case "batch_add":
    case "batch_remove":
    case "stock_names_set":
      return batchView();
    case "batch_update": {
      const b = await batchFx();
      for (const u of (args?.updates ?? []) as Record<string, unknown>[]) {
        const i = b.items.find((x: { id: number }) => x.id === u.id);
        if (i) Object.assign(i, Object.fromEntries(Object.entries(u).filter(([k, v]) => k !== "id" && v !== undefined)));
      }
      return batchView();
    }
    case "stock_names_get":
    case "stock_names_import":
      return (await batchFx()).names;
    case "batch_run": {
      const b = await batchFx();
      const all = structuredClone(b.all_results);
      const total = all.length;
      for (let k = 0; k < total; k++) {
        await sleep(700);
        window.dispatchEvent(new CustomEvent("batch-pair", { detail: { done: k + 1, total, pair: all[k] } }));
      }
      b.results = all;
      return batchView();
    }
    case "batch_existing":
      return [];
    case "batch_save":
      return args?.names ?? [];
    case "batch_cancel":
    case "batch_export_csv":
      return null;
    default:
      return null;
  }
}

/* ---------- batch (dev fixture app/dev-fixtures/batch.json) ---------- */

let batchData: any = null;
async function batchFx(): Promise<any> {
  batchData ??= await fx("batch.json");
  return batchData;
}
async function batchView(): Promise<unknown> {
  const b = await batchFx();
  return { items: b.items, results: b.results, running: false, folder: b.folder, names_path: b.names_path };
}
