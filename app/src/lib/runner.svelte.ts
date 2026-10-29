// Re-runs the matrix analysis whenever the LUT, its spaces, the settings or the
// "fit through" reference change (debounced; stale results are dropped).

import { api } from "./api";
import { app } from "./state.svelte";

let seq = 0;

export function analysisEffect() {
  $effect(() => {
    const lut = app.lut;
    const input = app.input ? $state.snapshot(app.input) : null;
    const output = app.output ? $state.snapshot(app.output) : null;
    const settings = $state.snapshot(app.settings);
    const through = app.through ? $state.snapshot(app.through) : null;
    if (!lut || !input || !output) return;
    const req = {
      path: lut.path,
      input,
      output,
      settings,
      middle_gray_hint: lut.detection.middle_gray_hint?.[0] ?? null,
      through: through ? { path: through.path, input: through.input, output } : null,
    };
    const my = ++seq;
    app.analyzing = true;
    const t = setTimeout(async () => {
      try {
        const a = await api.analyze(req);
        if (my === seq) {
          app.analysis = a;
          app.analysisError = null;
        }
      } catch (e) {
        if (my === seq) app.analysisError = String(e);
      } finally {
        if (my === seq) app.analyzing = false;
      }
    }, 180);
    return () => clearTimeout(t);
  });
}

// Re-runs the chart analysis when the still, its settings or the target change, and
// fetches a new preview when the encoding or exposure it is shown with changes.
let chartSeq = 0;
let previewKey = "";

export function chartEffect() {
  $effect(() => {
    const still = app.still;
    const settings = $state.snapshot(app.chart);
    const target = app.chartTarget;
    if (!still) return;
    const my = ++chartSeq;
    app.chartAnalyzing = true;
    const t = setTimeout(async () => {
      try {
        const a = await api.chartAnalyze({
          path: still.path,
          settings,
          target: target ? { path: target.path, space: null } : null,
        });
        if (my !== chartSeq) return;
        app.chartAnalysis = a;
        app.chartError = null;
        const ratio = a.fit?.exposure_ratio ?? 1;
        const key = `${still.path}|${JSON.stringify(a.space)}|${Math.round(Math.log2(ratio) * 4)}`;
        if (key !== previewKey) {
          previewKey = key;
          const p = await api.chartPreview(still.path, a.space, ratio);
          if (my === chartSeq) app.chartPreview = p;
        }
      } catch (e) {
        if (my === chartSeq) {
          app.chartError = String(e);
          app.chartAnalysis = null;
          // Still show the image, so the chart can be placed by hand.
          if (!app.chartPreview) rawPreview(still.path).catch(() => {});
        }
      } finally {
        if (my === chartSeq) app.chartAnalyzing = false;
      }
    }, 150);
    return () => clearTimeout(t);
  });
}

/** A preview for a still with no analysis yet (no chart found): shown as stored, DI. */
export async function rawPreview(path: string) {
  const space = { curve: { curve: "da_vinci_intermediate" }, gamut: "da_vinci_wide_gamut" };
  previewKey = "";
  app.chartPreview = await api.chartPreview(path, space, 1);
}
