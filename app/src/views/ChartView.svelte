<script lang="ts">
  import { onDestroy } from "svelte";
  import { api, type PatchReport, type StillSpace } from "../lib/api";
  import { app, curveKey, fmt } from "../lib/state.svelte";
  import { baseName, confirmAsk, copyText, dirName, joinPath, openFile, saveFile, IMAGE_FILTER, DCTL_FILTER } from "../lib/platform";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Toggle from "../lib/ui/Toggle.svelte";
  import Icon from "../lib/ui/Icon.svelte";

  let { onopen }: { onopen: () => void } = $props();

  const a = $derived(app.chartAnalysis);
  const f = $derived(a?.fit ?? null);
  const cat = $derived(app.chartCatalog);
  const pv = $derived(app.chartPreview);

  /* ---------- viewer: zoom to the chart, click patches, place by hand ---------- */

  let zoomChart = $state(true);
  // Corner clicks in "place by hand" mode (image pixels), in the order below.
  let placing = $state<[number, number][] | null>(null);
  const CORNERS = [
    { i: 0, what: "dark skin (1), the brown patch next to light skin" },
    { i: 5, what: "bluish green (6), the other end of that row" },
    { i: 23, what: "black (24), below bluish green" },
    { i: 18, what: "white (19), below dark skin" },
  ];
  let svg = $state<SVGSVGElement | null>(null);

  const imgW = $derived(a?.image.width ?? app.still?.info.width ?? 1);
  const imgH = $derived(a?.image.height ?? app.still?.info.height ?? 1);

  // Wheel zoom (around the pointer); null = the automatic framing.
  let custom = $state<[number, number, number, number] | null>(null);
  let customFor = "";
  $effect(() => {
    const p = app.still?.path ?? "";
    if (p !== customFor) {
      customFor = p;
      custom = null;
    }
  });

  const autoBox = $derived.by((): [number, number, number, number] => {
    if (!a || !zoomChart) return [0, 0, imgW, imgH];
    const pts = a.patches.flatMap((p) => p.sample_box);
    const xs = pts.map((p) => p[0]);
    const ys = pts.map((p) => p[1]);
    const m = a.pitch_px * 1.6;
    let x0 = Math.min(...xs) - m,
      x1 = Math.max(...xs) + m,
      y0 = Math.min(...ys) - m,
      y1 = Math.max(...ys) + m;
    // Keep a pleasant aspect (3:2) around the chart.
    const w = x1 - x0,
      h = y1 - y0;
    if (w / h < 1.5) {
      const d = (1.5 * h - w) / 2;
      x0 -= d;
      x1 += d;
    } else {
      const d = (w / 1.5 - h) / 2;
      y0 -= d;
      y1 += d;
    }
    return [x0, y0, x1 - x0, y1 - y0];
  });
  const viewBox = $derived((custom ?? autoBox).join(" "));

  // Non-passive, so the page doesn't scroll while zooming.
  $effect(() => {
    const el = svg;
    if (!el) return;
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  });

  function onWheel(e: WheelEvent) {
    const p = toImage(e);
    if (!p) return;
    e.preventDefault();
    const [x, y, w, h] = custom ?? autoBox;
    const k = Math.exp(Math.max(-0.5, Math.min(0.5, e.deltaY * 0.0015)));
    const nw = Math.min(Math.max(w * k, 40), Math.max(imgW, imgH) * 1.5);
    const f = nw / w;
    custom = [p[0] - (p[0] - x) * f, p[1] - (p[1] - y) * f, nw, h * f];
  }

  function toImage(e: MouseEvent): [number, number] | null {
    if (!svg) return null;
    const ctm = svg.getScreenCTM();
    if (!ctm) return null;
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse());
    return [p.x, p.y];
  }

  function onViewerClick(e: MouseEvent) {
    if (!placing) return;
    const p = toImage(e);
    if (!p) return;
    placing = [...placing, p];
    if (placing.length === 4) {
      app.chart.corners = placing as typeof app.chart.corners;
      app.chart.orientation = null;
      placing = null;
      zoomChart = true;
    }
  }

  function startPlacing() {
    placing = [];
  }

  function useDetection() {
    app.chart.corners = null;
    placing = null;
  }

  function togglePatch(i: number) {
    if (placing) return;
    const ex = app.chart.exclude;
    app.chart.exclude = ex.includes(i) ? ex.filter((k) => k !== i) : [...ex, i].sort((x, y) => x - y);
  }

  function rgb(c: [number, number, number]): string {
    return `rgb(${c[0]},${c[1]},${c[2]})`;
  }

  function patchState(p: PatchReport): "used" | "excluded" | "clipped" | "unused" {
    if (p.excluded) return "excluded";
    if (p.used) return "used";
    if (p.clipped_low.some(Boolean) || p.clipped_high.some(Boolean) || p.rolled_off.some(Boolean)) return "clipped";
    return "unused";
  }

  const labelSize = $derived(a ? Math.max(8, a.pitch_px * 0.24) : 12);
  const stroke = $derived(a ? Math.max(1, a.pitch_px * 0.03) : 2);

  /* ---------- encoding ---------- */

  const curveName = (c: { curve: string; ei?: number }) => cat?.curves.find((k) => curveKey(k.curve) === curveKey(c))?.name ?? c.curve;
  const gamutName = (g: string) => cat?.gamuts.find(([id]) => id === g)?.[1] ?? g;

  function chosen(): StillSpace {
    return app.chart.space ?? (a ? $state.snapshot(a.space) : { curve: { curve: "da_vinci_intermediate" }, gamut: "da_vinci_wide_gamut" });
  }
  function setCurve(key: string) {
    if (key === "auto") {
      app.chart.space = null;
      return;
    }
    const c = cat?.curves.find((k) => curveKey(k.curve) === key);
    if (c) app.chart.space = { ...chosen(), curve: c.curve };
  }
  function setGamut(g: string) {
    if (g === "auto") {
      app.chart.space = null;
      return;
    }
    app.chart.space = { ...chosen(), gamut: g };
  }
  const curveSel = $derived(app.chart.space ? curveKey(app.chart.space.curve) : "auto");
  const gamutSel = $derived(app.chart.space ? app.chart.space.gamut : "auto");
  const detectedCurve = $derived(a ? curveName(a.guess.space.curve) : "");
  const detectedGamut = $derived(a ? gamutName(a.guess.space.gamut) : "");

  /* ---------- target ---------- */

  type TargetKind = "reference" | "chart";
  let targetKind = $state<TargetKind>(app.chartTarget ? "chart" : "reference");
  async function setTargetKind(k: TargetKind) {
    targetKind = k;
    if (k === "reference") app.chartTarget = null;
    else if (!app.chartTarget) await chooseTarget();
  }
  async function chooseTarget() {
    const p = await openFile([IMAGE_FILTER], app.lastDir || undefined);
    if (p) await app.openTargetStill(p);
    if (!app.chartTarget) targetKind = "reference";
  }

  /* ---------- export ---------- */

  async function exportDctl() {
    if (!app.still || !f) return;
    const name = await api.chartDefaultDctlName(app.still.path);
    const path = await saveFile(joinPath(app.lastDir || dirName(app.still.path), name), [DCTL_FILTER]);
    if (!path) return;
    try {
      const out = await api.chartExportDctl(path);
      app.lastDir = dirName(out);
      app.notify(`Exported ${baseName(out)}`);
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  async function copyMatrix() {
    try {
      await copyText(await api.chartMatrixText());
      app.notify("Matrix copied");
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  /* ---------- OpenDRT preset ---------- */

  let elapsed = $state(0);
  let timer: ReturnType<typeof setInterval> | undefined;
  onDestroy(() => clearInterval(timer));
  let presetName = $state("");
  const o = $derived(app.odrtChartFit);

  async function fitOdrt() {
    if (!a || app.odrtChartRunning) return;
    app.odrtChartRunning = true;
    app.odrtChartError = null;
    elapsed = 0;
    const t0 = Date.now();
    timer = setInterval(() => (elapsed = (Date.now() - t0) / 1000), 200);
    try {
      app.odrtChartFit = await api.chartOpendrtFit($state.snapshot(app.odrtChart));
      if (!presetName && app.still) presetName = await api.chartDefaultPresetName(app.still.path);
    } catch (e) {
      app.odrtChartError = String(e);
    } finally {
      clearInterval(timer);
      app.odrtChartRunning = false;
    }
  }

  async function savePreset() {
    const name = presetName.trim();
    if (!name) return;
    try {
      let replace = false;
      if (await api.presetExists(name)) {
        replace = await confirmAsk(`A preset called “${name}” is already in your library. Replace it?`, "Replace preset", "Replace");
        if (!replace) return;
      }
      const saved = await api.chartPresetSave(name, replace);
      app.notify(`Saved “${saved}” to your preset library`);
      app.refreshPresets();
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  const odrtScale = $derived(o ? Math.max(o.before.mean, o.after.mean) * 1.05 : 1);
  const odrtSmooth = $derived(o ? o.safety.roughness <= 1.5 * o.safety.stock_roughness : true);
  const unused = $derived(a ? a.patches.filter((p) => !p.used) : []);
  // Equal R, G, B through the matrix (linear DWG), brightest channel = 1.
  const whiteOut = $derived.by(() => {
    if (!f) return null;
    const w = f.m_dwg.map((r) => r[0] + r[1] + r[2]);
    const mx = Math.max(...w);
    return mx > 0 ? w.map((v) => v / mx) : null;
  });
  function clipSourceText(s: string): string {
    return s === "measured"
      ? "measured in this still"
      : s === "brightest"
        ? "this still doesn't clip: its brightest value, set it to where the footage clips"
        : s === "set"
          ? "set by hand"
          : "a typical value";
  }
  const confClass = (c: string) => `badge ${c}`;
</script>

{#if !app.still}
  <div class="empty">
    <div class="well">
      <Icon name="drop" size={40} />
      <h2>Drop a still with a colour chart</h2>
      <p class="muted">A ColorChecker Classic (24 patches), any way up. TIFF, OpenEXR, DPX, PNG or JPEG, as exported from Resolve.</p>
      <button class="btn primary large" onclick={onopen}>Open Still…</button>
    </div>
  </div>
{:else}
  <div class="page">
    <div class="top">
      <div class="viewer card" class:placing={!!placing}>
        <div class="vbar">
          {#if placing}
            <span class="hint"><b>Click the centre of {CORNERS[placing.length].what}</b> ({placing.length + 1} of 4) · scroll to zoom</span>
            <span class="grow"></span>
            <button class="btn" onclick={() => (placing = null)}>Cancel</button>
          {:else}
            <span class="muted small">
              {#if a}
                {a.placement_source === "manual" ? "Placed by hand" : `Found ${a.detected_patches} patches`} · {cat?.orientations.find(([k]) => k === a.placement.orientation)?.[1] ?? ""}
                · click a patch to leave it out · scroll to zoom
              {:else if app.chartError}
                {app.chartError}
              {:else}
                Looking for the chart…
              {/if}
            </span>
            <span class="grow"></span>
            {#if a}<button class="btn plain" onclick={() => { zoomChart = !(zoomChart && !custom); custom = null; }}>{zoomChart && !custom ? "Whole image" : "Zoom to chart"}</button>{/if}
            {#if app.chart.corners}
              <button class="btn" onclick={useDetection}>Use detection</button>
            {/if}
            <button class="btn" onclick={startPlacing}>Place by hand…</button>
          {/if}
        </div>
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <svg bind:this={svg} viewBox={viewBox} preserveAspectRatio="xMidYMid meet" onclick={onViewerClick} class:stale={app.chartAnalyzing}>
          {#if pv}
            <image href={pv.data_url} x="0" y="0" width={imgW} height={imgH} />
          {/if}
          {#if a && !placing}
            {#each a.patches as p (p.index)}
              {@const st = patchState(p)}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <g class="patch {st}" onclick={(e) => { e.stopPropagation(); togglePatch(p.index); }}>
                <title>{p.index + 1} {p.name}{p.reason ? ` — ${p.reason}` : ""} · ΔE {fmt(p.de_before, 1)} → {fmt(p.de_after, 1)}</title>
                <polygon points={p.sample_box.map((q) => q.join(",")).join(" ")} />
                <text
                  x={p.sample_box.reduce((t, q) => t + q[0], 0) / 4}
                  y={Math.min(...p.sample_box.map((q) => q[1])) - stroke * 2.5}
                  font-size={labelSize}
                  text-anchor="middle">{p.index + 1}</text
                >
                {#if st !== "used"}
                  <line x1={p.sample_box[0][0]} y1={p.sample_box[0][1]} x2={p.sample_box[2][0]} y2={p.sample_box[2][1]} />
                {/if}
              </g>
            {/each}
          {/if}
          {#if placing}
            {#each placing as q, k (k)}
              <circle cx={q[0]} cy={q[1]} r={(custom ?? autoBox)[2] / 150} class="mark" />
            {/each}
          {/if}
        </svg>
        <div class="legend small muted">
          <span><i class="sw used"></i>used</span>
          <span><i class="sw clipped"></i>clipped / out of order</span>
          <span><i class="sw excluded"></i>left out</span>
        </div>
      </div>

      <div class="side">
        <div class="card summary" class:stale={app.chartAnalyzing}>
          {#if f}
            <div class="big">
              <div><div class="muted small">Before</div><b class="num">{fmt(f.mean_before, 1)}</b></div>
              <Icon name="chevron" size={18} />
              <div><div class="muted small">After</div><b class="num accent">{fmt(f.mean_after, 1)}</b></div>
              <div class="unit muted small">mean ΔE2000<br />{f.used} of 24 patches</div>
            </div>
            <div class="small muted num">
              max {fmt(f.max_after, 1)} · neutrals {fmt(f.neutral_after, 1)} · still {f.exposure_ratio >= 1 ? "+" : ""}{fmt(Math.log2(f.exposure_ratio), 2)} stops from the target
            </div>
          {:else if a?.fit_error}
            <div class="err"><Icon name="warn" /> {a.fit_error}</div>
          {:else}
            <div class="muted">…</div>
          {/if}
        </div>

        {#if a}
          <div class="swatches" role="img" aria-label="Patches: camera, corrected, target">
            {#each a.patches as p (p.index)}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div class="tile {patchState(p)}" title="{p.index + 1} {p.name}{p.reason ? ` — ${p.reason}` : ''}" onclick={() => togglePatch(p.index)}>
                <div class="bands">
                  <span style="background:{rgb(p.swatch_measured)}"></span>
                  <span style="background:{rgb(p.swatch_corrected)}"></span>
                  <span style="background:{rgb(p.swatch_target)}"></span>
                </div>
                <div class="de num">{fmt(p.de_after, 1)}</div>
              </div>
            {/each}
          </div>
          <div class="small faint legend2">Each patch: camera · corrected · target, at the target's exposure. Number: ΔE2000 after.</div>
          {#if unused.length}
            <div class="small muted notes">
              {#each unused as p (p.index)}<div><b>{p.index + 1}</b> {p.name}: {p.reason}</div>{/each}
            </div>
          {/if}
        {/if}
      </div>
    </div>

    {#if a}
      {#each a.notes as n}
        <div class="note small"><Icon name="info" /> <span>{n}</span></div>
      {/each}
    {/if}

    <div class="cols">
      <div>
        <div class="group-title">Still</div>
        <div class="group">
          <div class="row">
            <div class="label">File<div class="sub">{app.still.info.format} · {app.still.info.sample_type} · {app.still.info.width}×{app.still.info.height}</div></div>
            <div class="value"><span class="name" title={app.still.path}>{app.still.file_name}</span><button class="btn plain" onclick={onopen}>Open…</button></div>
          </div>
          <div class="row">
            <div class="label">Orientation</div>
            <div class="value">
              <select bind:value={app.chart.orientation} aria-label="Orientation" disabled={!!app.chart.corners}>
                <option value={null}>Automatic{a && !app.chart.orientation ? ` (${cat?.orientations.find(([k]) => k === a.placement.orientation)?.[1]})` : ""}</option>
                {#each cat?.orientations ?? [] as [k, label] (k)}<option value={k}>{label}</option>{/each}
              </select>
            </div>
          </div>
          <div class="row">
            <div class="label">Curve<div class="sub">How the still's values relate to light</div></div>
            <div class="value">
              {#if a && !app.chart.space}<span class={confClass(a.guess.curve_confidence)}>{a.guess.curve_confidence}</span>{/if}
              <select value={curveSel} onchange={(e) => setCurve(e.currentTarget.value)} aria-label="Curve">
                <option value="auto">Detected{detectedCurve ? ` (${detectedCurve})` : ""}</option>
                {#each cat?.curves ?? [] as c (curveKey(c.curve))}<option value={curveKey(c.curve)}>{c.name}</option>{/each}
              </select>
            </div>
          </div>
          <div class="row">
            <div class="label">Gamut<div class="sub">What the still was exported in</div></div>
            <div class="value">
              {#if a && !app.chart.space}<span class={confClass(a.guess.gamut_confidence)}>{a.guess.gamut_confidence}</span>{/if}
              <select value={gamutSel} onchange={(e) => setGamut(e.currentTarget.value)} aria-label="Gamut">
                <option value="auto">Detected{detectedGamut ? ` (${detectedGamut})` : ""}</option>
                {#each cat?.gamuts ?? [] as [g, label] (g)}<option value={g}>{label}</option>{/each}
              </select>
            </div>
          </div>
          {#if a}
            <div class="row reasons">
              <div class="sub">
                {#each a.guess.reasons as r}<div>· {r}</div>{/each}
              </div>
            </div>
          {/if}
        </div>

        <div class="group-title">Target</div>
        <div class="group">
          <div class="row">
            <div class="label">Match to</div>
            <div class="value">
              <Segmented
                bind:value={() => targetKind, setTargetKind}
                label="Target"
                options={[
                  { value: "reference", label: "Published chart" },
                  { value: "chart", label: "Another camera", title: "The chart in another still, as that camera shows it" },
                ]}
              />
            </div>
          </div>
          {#if targetKind === "reference"}
            <div class="row">
              <div class="label">Chart</div>
              <div class="value">
                <select bind:value={app.chart.edition} aria-label="Chart edition">
                  {#each cat?.editions ?? [] as [e, label] (e)}<option value={e}>{label}</option>{/each}
                </select>
              </div>
            </div>
          {:else}
            <div class="row">
              <div class="label">Hero still<div class="sub">Its chart and exposure become the target</div></div>
              <div class="value">
                <span class="name" title={app.chartTarget?.path}>{app.chartTarget?.file_name ?? "—"}</span>
                <button class="btn plain" onclick={chooseTarget}>Choose…</button>
              </div>
            </div>
          {/if}
        </div>
      </div>

      <div>
        <div class="group-title">Correction</div>
        <div class="group">
          <div class="row">
            <div class="label">Model</div>
            <div class="value">
              <Segmented
                bind:value={app.chart.fit.model}
                label="Model"
                options={[
                  { value: "matrix", label: "Matrix", title: "One 3×3 matrix in linear XYZ" },
                  { value: "matrix_tone", label: "Matrix + tone", title: "A per-channel curve in linear DWG first: for footage whose grays don't follow the light" },
                ]}
              />
            </div>
          </div>
          <div class="row">
            <div class="label">Exposure</div>
            <div class="value">
              <Segmented
                bind:value={app.chart.fit.exposure}
                label="Exposure"
                options={[
                  { value: "match", label: "Match target", title: "The correction also sets the chart's exposure to the target's" },
                  { value: "keep", label: "Keep", title: "Unity gain on the grays: colour only" },
                ]}
              />
            </div>
          </div>
          <div class="row">
            <div class="label">
              Preserve neutrals<div class="sub">Equal R, G, B stay equal, so clipped white stays white. The grays keep the camera's cast: white-balance in the raw decode first</div>
            </div>
            <div class="value"><Toggle bind:checked={app.chart.fit.preserve_neutrals} label="Preserve neutrals" /></div>
          </div>
          <div class="row" class:off={app.chart.fit.preserve_neutrals}>
            <div class="label">
              Lock neutral<div class="sub">{app.chart.fit.preserve_neutrals ? "Off while neutrals are preserved" : "Grays land exactly on the target's gray"}</div>
            </div>
            <div class="value"><Toggle bind:checked={app.chart.fit.lock_neutral} label="Lock neutral" disabled={app.chart.fit.preserve_neutrals} /></div>
          </div>
          <div class="row">
            <div class="label">Restraint<div class="sub">Closer to a plain white balance: less noise where a channel was barely recorded</div></div>
            <div class="value">
              <Segmented
                bind:value={app.chart.fit.restraint}
                label="Restraint"
                options={[
                  { value: 0, label: "Off" },
                  { value: 0.25, label: "Light" },
                  { value: 0.6, label: "Strong" },
                ]}
              />
            </div>
          </div>
          {#if f}
            <div class="row mat">
              <div class="label">Matrix<div class="sub">In linear DWG{f.tone ? ", after the tone curve" : ""}</div></div>
              <div class="value num">
                <table>
                  <tbody>
                    {#each f.m_dwg as r}<tr>{#each r as v}<td>{fmt(v, 3)}</td>{/each}</tr>{/each}
                  </tbody>
                </table>
              </div>
            </div>
            {#if f.tone}
              <div class="row">
                <div class="label">Tone curve</div>
                <div class="value num small">power {f.tone.power.map((v) => fmt(v, 3)).join(" / ")} · black {f.tone.black.map((v) => fmt(v, 4)).join(" / ")}</div>
              </div>
            {/if}
            <div class="row">
              <div class="label">Neutrals<div class="sub">The camera's gray, and what equal R, G, B (clipped white) becomes</div></div>
              <div class="value num small">
                gray {fmt(f.gray_dwg[0], 2)} : 1 : {fmt(f.gray_dwg[2], 2)} · white →
                <span class:warnc={whiteOut && Math.min(...whiteOut) < 0.8}>{whiteOut ? whiteOut.map((v) => fmt(v, 2)).join(" : ") : "—"}</span>
              </div>
            </div>
          {/if}
          <div class="row">
            <div class="label">Highlight protection<div class="sub">In the DCTL: near the clip, clipped areas go to white instead of the colour the correction gives them</div></div>
            <div class="value"><Toggle bind:checked={app.chart.highlights.enabled} label="Highlight protection" /></div>
          </div>
          {#if app.chart.highlights.enabled}
            <div class="row">
              <div class="label">
                Clip level<div class="sub">
                  DaVinci Intermediate code where the footage clips{#if a}
                    {" · "}{clipSourceText(app.chart.highlights.clip_di === null ? a.highlights.source : a.highlights.measured_source)}
                    {#if app.chart.highlights.clip_di !== null}{" "}{fmt(a.highlights.measured_di, 3)}{/if}
                  {/if}
                </div>
              </div>
              <div class="value">
                <input
                  type="number"
                  class="clip"
                  min="0.2"
                  max="1"
                  step="0.001"
                  value={(app.chart.highlights.clip_di ?? a?.highlights.clip_di ?? 0.68).toFixed(3)}
                  onchange={(e) => {
                    const v = Number((e.currentTarget as HTMLInputElement).value);
                    if (Number.isFinite(v)) app.chart.highlights.clip_di = Math.min(1, Math.max(0.2, v));
                  }}
                  aria-label="Clip level"
                />
                {#if app.chart.highlights.clip_di !== null}
                  <button class="btn plain" onclick={() => (app.chart.highlights.clip_di = null)} title="Use the level measured in the still">Auto</button>
                {/if}
              </div>
            </div>
            <div class="row">
              <div class="label">Detect<div class="sub">Which pixels count as clipped: any channel at the clip, or only all of them</div></div>
              <div class="value">
                <Segmented
                  bind:value={app.chart.highlights.detect}
                  label="Detect"
                  options={[
                    { value: 0, label: "Any channel", title: "Also pixels where only one channel clips (e.g. green around the sun under water): larger white areas" },
                    { value: 0.5, label: "Between" },
                    { value: 1, label: "All channels", title: "Only where every channel clips" },
                  ]}
                />
              </div>
            </div>
            <div class="row">
              <div class="label">Softness<div class="sub">How far below the clip the protection starts</div></div>
              <div class="value">
                <Segmented
                  bind:value={app.chart.highlights.softness}
                  label="Softness"
                  options={[
                    { value: 0.5, label: "½ stop" },
                    { value: 1, label: "1 stop" },
                    { value: 2, label: "2 stops" },
                  ]}
                />
              </div>
            </div>
            <div class="row"><div class="faint small">All four are sliders in the DCTL too, to fine-tune in Resolve.</div></div>
          {/if}
          <div class="row go">
            <span class="grow"></span>
            <button class="btn" onclick={copyMatrix} disabled={!f}><Icon name="copy" size={14} /> Copy Matrix</button>
            <button class="btn primary" onclick={exportDctl} disabled={!f}><Icon name="export" size={14} /> Export DCTL…</button>
          </div>
        </div>
        <p class="faint small">The DCTL works in DaVinci Wide Gamut / Intermediate: use it on this camera's footage, decoded like the still, before the output transform.</p>

        <div class="group-title">OpenDRT preset</div>
        <div class="group">
          <div class="row">
            <div class="label">Look</div>
            <div class="value">
              <select bind:value={app.odrtChart.look} aria-label="Look preset">
                {#each cat?.looks ?? [] as l, i (l)}<option value={i}>{l}</option>{/each}
              </select>
              <select bind:value={app.odrtChart.tonescale} aria-label="Tonescale preset">
                {#each cat?.tonescales ?? [] as t, i (t)}<option value={i}>{i === 0 ? "Look's tonescale" : t}</option>{/each}
              </select>
            </div>
          </div>
          <div class="row">
            <div class="label">Display</div>
            <div class="value">
              <select bind:value={app.odrtChart.display_encoding} aria-label="Display encoding">
                {#each cat?.displays ?? [] as [i, label] (i)}<option value={i}>{label}</option>{/each}
              </select>
            </div>
          </div>
          <div class="row">
            <div class="label">After the matrix<div class="sub">Fit for “chart DCTL → OpenDRT”; off fits OpenDRT alone</div></div>
            <div class="value"><Toggle bind:checked={app.odrtChart.after_matrix} label="After the matrix" /></div>
          </div>
          <div class="row">
            <div class="label">Fit tone<div class="sub">Contrast and black offset too, not only colour</div></div>
            <div class="value"><Toggle bind:checked={app.odrtChart.fit_tone} label="Fit tone" /></div>
          </div>
          <div class="row">
            <div class="label">Smoothing<div class="sub">Pull towards the look: 24 patches can't pin down every parameter</div></div>
            <div class="value">
              <Segmented
                bind:value={app.odrtChart.regularization}
                label="Smoothing"
                options={[
                  { value: 3, label: "Light" },
                  { value: 6, label: "Normal" },
                  { value: 12, label: "Strong" },
                ]}
              />
            </div>
          </div>
          <div class="row go">
            {#if app.odrtChartRunning}
              <div class="spinner"></div>
              <span class="muted">Fitting… {elapsed.toFixed(0)} s</span>
            {:else if app.odrtChartError}
              <span class="err"><Icon name="warn" /> {app.odrtChartError}</span>
            {/if}
            <span class="grow"></span>
            <button class="btn primary" onclick={fitOdrt} disabled={!a || app.odrtChartRunning || (app.odrtChart.after_matrix && !f)}><Icon name="play" size={14} /> Fit Preset</button>
          </div>
        </div>

        {#if o}
          <div class="card odrt fade-in" class:stale={app.odrtChartRunning}>
            {#each [{ l: "Stock look", s: o.before }, { l: "Fitted preset", s: o.after }] as b (b.l)}
              <div class="bar">
                <div class="strong">{b.l}</div>
                <div class="track"><div class="fill" class:best={b.l !== "Stock look"} style="width:{(100 * b.s.mean) / odrtScale}%"></div></div>
                <div class="num"><b>{fmt(b.s.mean, 2)}</b> <span class="faint">max {fmt(b.s.max, 1)}</span></div>
              </div>
            {/each}
            <div class="small muted">{o.base}{o.settings.after_matrix ? ", after the matrix" : ""} · grey {fmt(o.config.grey_nits, 2)} nits · mean ΔE2000 on the display</div>
            <div class="checks small">
              <span class:bad={!o.safety.finite_in_range}><Icon name={o.safety.finite_in_range ? "check" : "warn"} size={13} /> in range</span>
              <span class:bad={o.safety.reversals.length > 0}><Icon name={o.safety.reversals.length ? "warn" : "check"} size={13} /> brightness rises with exposure</span>
              <span class:bad={!odrtSmooth}><Icon name={odrtSmooth ? "check" : "warn"} size={13} /> smooth</span>
            </div>
            {#each o.notes as n}<div class="small muted"><Icon name="info" size={13} /> {n}</div>{/each}
            <div class="saverow">
              <input type="text" bind:value={presetName} maxlength="60" spellcheck="false" aria-label="Preset name" />
              <button class="btn primary" onclick={savePreset} disabled={!presetName.trim()}>Save to Library</button>
            </div>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .empty {
    height: 100%;
    min-height: 420px;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .well {
    width: min(540px, 100%);
    border: 1.5px dashed var(--border);
    border-radius: 18px;
    padding: 48px 32px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    text-align: center;
    color: var(--label-3);
  }
  .well h2 {
    color: var(--label);
    font-size: 18px;
    font-weight: 600;
  }
  .well p {
    margin: 0 0 8px;
    max-width: 420px;
  }
  .page {
    padding: 18px 22px 40px;
    max-width: 1280px;
  }
  .top {
    display: grid;
    grid-template-columns: minmax(0, 1.55fr) minmax(300px, 1fr);
    gap: 18px;
    align-items: start;
    margin-bottom: 14px;
  }
  @media (max-width: 980px) {
    .top,
    .cols {
      grid-template-columns: 1fr !important;
    }
  }
  .viewer {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .vbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: 0.5px solid var(--separator);
    min-height: 44px;
  }
  .viewer.placing .vbar {
    background: var(--accent-soft);
  }
  .hint {
    color: var(--accent);
  }
  svg {
    width: 100%;
    aspect-ratio: 3 / 2;
    background: #111;
    display: block;
    cursor: default;
  }
  .placing svg {
    cursor: crosshair;
  }
  svg.stale {
    opacity: 0.75;
  }
  .patch {
    cursor: pointer;
  }
  .patch polygon,
  .patch line {
    stroke-width: 1.5px;
    vector-effect: non-scaling-stroke;
  }
  .patch polygon {
    fill: transparent;
    stroke: rgba(255, 255, 255, 0.95);
  }
  .patch text {
    fill: #fff;
    paint-order: stroke;
    stroke: rgba(0, 0, 0, 0.75);
    stroke-width: 3px;
    font-weight: 600;
    font-family: var(--font);
  }
  .patch.clipped polygon,
  .patch.clipped line {
    stroke: #ff453a;
  }
  .patch.excluded polygon,
  .patch.excluded line {
    stroke: rgba(255, 255, 255, 0.45);
    stroke-dasharray: 4 3;
  }
  .patch.unused polygon,
  .patch.unused line {
    stroke: #ffd60a;
  }
  .patch:hover polygon {
    stroke: var(--accent);
  }
  .mark {
    fill: var(--accent);
    stroke: #fff;
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
  .legend {
    display: flex;
    gap: 14px;
    padding: 7px 10px;
  }
  .legend .sw {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 2px;
    border: 1.5px solid;
    margin-right: 5px;
    vertical-align: -1px;
  }
  .sw.used {
    border-color: var(--label-2);
  }
  .sw.clipped {
    border-color: #ff453a;
  }
  .sw.excluded {
    border-color: var(--label-3);
    border-style: dashed;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .summary .big {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .summary .big b {
    font-size: 28px;
    font-weight: 600;
    line-height: 1.1;
  }
  .summary .big :global(svg) {
    color: var(--label-3);
  }
  .accent {
    color: var(--accent);
  }
  .unit {
    margin-left: auto;
    text-align: right;
  }
  .swatches {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 6px;
  }
  .tile {
    cursor: pointer;
    border-radius: 6px;
    overflow: hidden;
    background: var(--surface);
    box-shadow: var(--shadow);
  }
  .tile .bands {
    display: flex;
    height: 34px;
  }
  .tile .bands span {
    flex: 1;
  }
  .tile .de {
    font-size: 10.5px;
    text-align: center;
    padding: 1px 0 2px;
    color: var(--label-2);
  }
  .tile.excluded,
  .tile.clipped,
  .tile.unused {
    opacity: 0.4;
  }
  .tile.clipped .de {
    color: var(--red);
  }
  .legend2 {
    margin-top: -2px;
  }
  .notes {
    line-height: 1.45;
  }
  .note {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    color: var(--label-2);
    margin: 0 2px 6px;
    max-width: 980px;
  }
  .note :global(svg) {
    flex: none;
    margin-top: 1px;
  }
  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0 22px;
    margin-top: 10px;
  }
  .group {
    margin-bottom: 18px;
  }
  .cols .row > .label {
    flex: 1 1 0;
    min-width: 96px;
  }
  .cols .row > .value {
    flex: 0 1 auto;
  }
  .cols .row .sub {
    white-space: normal;
  }
  .name {
    max-width: 230px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .reasons .sub {
    white-space: normal;
    line-height: 1.45;
  }
  select {
    max-width: 260px;
  }
  .row.go {
    min-height: 50px;
  }
  .row.off .label {
    opacity: 0.55;
  }
  input.clip {
    width: 84px;
    text-align: right;
  }
  .warnc {
    color: var(--orange, var(--red));
  }
  .grow {
    flex: 1;
  }
  .err {
    color: var(--red);
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .stale {
    opacity: 0.55;
  }
  .small {
    font-size: 11.5px;
  }
  .mat table {
    border-collapse: collapse;
    font-size: 12px;
  }
  .mat td {
    padding: 1px 0 1px 12px;
    text-align: right;
  }
  .odrt {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-bottom: 18px;
  }
  .bar {
    display: grid;
    grid-template-columns: 110px 1fr 110px;
    gap: 12px;
    align-items: center;
  }
  .track {
    height: 10px;
    border-radius: 5px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: 5px;
    background: color-mix(in srgb, var(--label) 30%, transparent);
    transition: width 0.4s var(--ease);
  }
  .fill.best {
    background: var(--accent);
  }
  .checks {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
  }
  .checks span {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    color: var(--green);
  }
  .checks span.bad {
    color: var(--amber);
  }
  .saverow {
    display: flex;
    gap: 8px;
  }
  .saverow input {
    flex: 1;
  }
  .strong {
    font-weight: 500;
  }
</style>
