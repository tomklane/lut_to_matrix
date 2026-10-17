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
