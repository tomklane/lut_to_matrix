// Re-runs the matrix analysis whenever the LUT, its spaces, the settings or the
// "fit through" reference change (debounced; stale results are dropped).

import { api, type PreviewDisplay } from "./api";
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

// Re-runs the chart analysis when the still, its settings or the target change.
// chartPreviewEffect fetches the before / after previews.
let chartSeq = 0;
let previewKey = "";
let afterKey = "";
let afterFor: unknown = null;
let previewSeq = 0;

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

/** The display previews are shown on. */
function previewDisplay(): PreviewDisplay {
  return app.chartDisplay === "opendrt" ? { kind: "open_drt", look: app.odrtChart.look, tonescale: app.odrtChart.tonescale } : { kind: "simple" };
}

// Before: the still on screen (the chart still, or the comparison still) decoded with the
// chart still's encoding, at the target's exposure. After: the same with the correction,
// fetched when the view shows it and the analysis (or anything shown) changed.
export function chartPreviewEffect() {
  $effect(() => {
    const a = app.chartAnalysis;
    const still = app.still;
    const subject = app.compareStill ?? still;
    const view = app.chartView;
    const display = previewDisplay();
    if (!a || !subject) return;
    const my = ++previewSeq;
    const ratio = a.fit?.exposure_ratio ?? 1;
    const base = `${subject.path}|${JSON.stringify(a.space)}|${Math.round(Math.log2(ratio) * 4)}|${JSON.stringify(display)}`;
    const t = setTimeout(async () => {
      try {
        if (base !== previewKey) {
          const p = await api.chartPreview(subject.path, a.space, ratio, false, display);
          if (my !== previewSeq) return;
          previewKey = base;
          app.chartPreview = p;
        }
        if (view !== "before" && a.fit && (base !== afterKey || afterFor !== a)) {
          const p = await api.chartPreview(subject.path, a.space, ratio, true, display);
          if (my !== previewSeq) return;
          afterKey = base;
          afterFor = a;
          app.chartAfter = p;
        }
      } catch (e) {
        if (my === previewSeq) app.notify(String(e), "error");
      }
    }, 120);
    return () => clearTimeout(t);
  });
}

/** A preview for a still with no analysis yet (no chart found): shown as stored, DI. */
export async function rawPreview(path: string) {
  const space = { curve: { curve: "da_vinci_intermediate" }, gamut: "da_vinci_wide_gamut" };
  previewKey = "";
  app.chartPreview = await api.chartPreview(path, space, 1);
}
