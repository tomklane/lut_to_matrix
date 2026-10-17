<script lang="ts">
  import { api, type Gamut, type InputSpace } from "../lib/api";
  import { app, cameraName, DI_INPUT, fmt, metricLabel } from "../lib/state.svelte";
  import { baseName, copyText, dirName, joinPath, openFile, saveFile, LUT_FILTER } from "../lib/platform";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Toggle from "../lib/ui/Toggle.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import Swatches from "./Swatches.svelte";
  import DropHere from "./DropHere.svelte";

  let { onopen }: { onopen: () => void } = $props();

  const cat = $derived(app.catalog);
  const det = $derived(app.lut?.detection);
  const camera = $derived(cat?.cameras.find((c) => c.id === app.input?.id));
  const a = $derived(app.analysis);
  const stats = $derived(a ? (a.stats_refined ?? a.stats_ls) : null);
  const metric = $derived(a ? metricLabel(a.metric) : "ΔE");

  /* ---- spaces, bound to app.input / app.output ---- */
  type TKey = "2.2" | "2.4" | "2.6" | "pq";
  function getTransferKey(): TKey {
    const t = app.output?.transfer;
    return !t ? "2.4" : t.kind === "pq" ? "pq" : (t.gamma.toFixed(1) as TKey);
  }
  function setTransferKey(k: TKey) {
    if (!app.output) return;
    const prevPeak = app.output.transfer.kind === "pq" ? app.output.transfer.peak_nits : 1000;
    app.output = {
      ...app.output,
      transfer: k === "pq" ? { kind: "pq", peak_nits: prevPeak } : { kind: "gamma", gamma: Number(k) },
    };
  }
  function setInputId(id: string) {
    if (app.input) app.input = { ...app.input, id };
  }

  function setGamut(g: Gamut) {
    if (app.output) app.output = { ...app.output, gamut: g };
  }
  function setPeak(p: number) {
    if (app.output?.transfer.kind === "pq") app.output = { ...app.output, transfer: { kind: "pq", peak_nits: p } };
  }
  function setEi(ei: number) {
    if (app.input) app.input = { ...app.input, arri_ei: ei };
  }
  function resetDetected() {
    if (app.lut) {
      app.input = app.lut.input;
      app.output = app.lut.output;
    }
  }
  const overridden = $derived(
    !!app.lut && (JSON.stringify(app.input) !== JSON.stringify(app.lut.input) || JSON.stringify(app.output) !== JSON.stringify(app.lut.output)),
  );

  let showWhy = $state(false);

  /* ---- fit through ---- */
  async function chooseThrough() {
    const p = await openFile([LUT_FILTER], app.through ? dirName(app.through.path) : app.lastDir || undefined);
    if (!p) return;
    try {
      await api.loadLut(p);
      app.through = { path: p, fileName: baseName(p), input: { ...DI_INPUT } };
    } catch (e) {
      app.notify(String(e), "error");
    }
  }
  function setThroughInput(id: string) {
    if (app.through) app.through = { ...app.through, input: { ...app.through.input, id } as InputSpace };
  }

  /* ---- actions ---- */
  async function copyMatrix() {
    try {
      await copyText(await api.matrixText());
      app.notify("Matrix copied");
    } catch (e) {
      app.notify(String(e), "error");
    }
  }
  async function exportDctl() {
    if (!app.lut || !a) return;
    const name = await api.defaultDctlName(app.lut.path);
    const path = await saveFile(joinPath(app.lastDir || dirName(app.lut.path), name), [{ name: "DCTL", extensions: ["dctl"] }]);
    if (!path) return;
    try {
      const out = await api.exportDctl(path);
      app.lastDir = dirName(out.dctl);
      app.notify(`Exported ${baseName(out.dctl)} (+ .json and .txt)`);
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  function onKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "e") {
      e.preventDefault();
      exportDctl();
    }
  }

  const bracketOptions = [0, 0.5, 1, 2];
  const warnings = $derived(a?.diagnostics.filter((d) => d.severity === "warning") ?? []);
  const infos = $derived(a?.diagnostics.filter((d) => d.severity === "info" && d.code !== "reference") ?? []);
</script>

<svelte:window onkeydown={onKey} />

{#if !app.lut}
  <DropHere {onopen} />
{:else if app.input && app.output && cat}
  <div class="page">
    <div class="col left">
      <div class="group-title">LUT</div>
      <div class="group">
        <div class="row">
          <div class="label">Input</div>
          <div class="value">
            {#if det}<span class="badge {det.input.confidence}">{det.input.confidence}</span>{/if}
            <select value={app.input.id} onchange={(e) => setInputId(e.currentTarget.value)} aria-label="Input space">
              {#each cat.cameras as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
            </select>
          </div>
        </div>
        {#if camera?.has_ei}
          <div class="row">
            <div class="label">Exposure index</div>
            <div class="value">
              <select value={app.input.arri_ei} onchange={(e) => setEi(Number(e.currentTarget.value))} aria-label="ARRI EI">
                {#each cat.arri_eis as ei (ei)}<option value={ei}>EI {ei}</option>{/each}
              </select>
            </div>
          </div>
        {/if}
        <div class="row">
          <div class="label">Output gamut</div>
          <div class="value">
            {#if det}<span class="badge {det.output_gamut.confidence}">{det.output_gamut.confidence}</span>{/if}
            <select value={app.output.gamut} onchange={(e) => setGamut(e.currentTarget.value as Gamut)} aria-label="Output gamut">
              {#each cat.gamuts as [id, name] (id)}<option value={id}>{name}</option>{/each}
            </select>
          </div>
        </div>
        <div class="row">
          <div class="label">Output EOTF</div>
          <div class="value">
            {#if det}<span class="badge {det.output_transfer.confidence}">{det.output_transfer.confidence}</span>{/if}
            <Segmented
              bind:value={getTransferKey, setTransferKey}
              label="Output transfer"
              options={[
                { value: "2.2", label: "2.2" },
                { value: "2.4", label: "2.4" },
                { value: "2.6", label: "2.6" },
                { value: "pq", label: "PQ" },
              ]}
            />
          </div>
        </div>
        {#if app.output.transfer.kind === "pq"}
          <div class="row">
            <div class="label">PQ peak</div>
            <div class="value">
              <select value={app.output.transfer.peak_nits} onchange={(e) => setPeak(Number(e.currentTarget.value))} aria-label="PQ peak">
                {#each cat.pq_peaks as p (p)}<option value={p}>{p} nits</option>{/each}
              </select>
            </div>
          </div>
        {/if}
        <div class="row foot">
          <button class="btn plain" onclick={() => (showWhy = !showWhy)}>{showWhy ? "Hide" : "Why these?"}</button>
          {#if overridden}<button class="btn plain" onclick={resetDetected}>Use detected</button>{/if}
          <span class="grow"></span>
          {#if det?.probe}
            <span class="sub num" title="Where the detected spaces put 18% gray">
              18% gray → {fmt(det.probe.output_nits)} nits ({det.probe.stops_from_expected >= 0 ? "+" : ""}{fmt(det.probe.stops_from_expected)} st)
            </span>
          {/if}
        </div>
        {#if showWhy && det}
          <div class="why fade-in">
            <div><b>Input</b> — {det.input.reasons.join("; ") || "no evidence; default"}</div>
            <div><b>Gamut</b> — {det.output_gamut.reasons.join("; ") || "no evidence; default"}</div>
            <div><b>EOTF</b> — {det.output_transfer.reasons.join("; ") || "no evidence; default"}</div>
            {#each det.notes as n}<div class="faint">{n}</div>{/each}
            {#if det.middle_gray_hint}<div class="faint">{det.middle_gray_hint[1]}</div>{/if}
          </div>
        {/if}
      </div>

      <div class="group-title">Fit</div>
      <div class="group">
        <div class="row">
          <div class="label">White</div>
          <div class="value">
            <Segmented
              bind:value={app.settings.white_mode}
              label="White mode"
              options={[
                { value: "d65", label: "D65", title: "Neutral in → D65 neutral out" },
                { value: "preserve_lut", label: "Keep LUT white", title: "Bake the LUT's white cast into the matrix" },
              ]}
            />
          </div>
        </div>
        {#if app.settings.white_mode === "preserve_lut"}
          <div class="row">
            <div class="label">Lock neutral<div class="sub">Gray reproduces the LUT's white exactly</div></div>
            <div class="value"><Toggle bind:checked={app.settings.lock_neutral} label="Lock neutral" /></div>
          </div>
        {/if}
        <div class="row">
          <div class="label">Measure at</div>
          <div class="value">
            <Segmented
              bind:value={app.settings.anchor}
              label="Anchor"
              options={[
                { value: "scene_linear", label: "Scene 18%", title: "A: scene-linear middle gray" },
                { value: "display_target", label: "Display gray", title: "B: the exposure the LUT maps to display middle gray" },
              ]}
            />
          </div>
        </div>
        <div class="row">
          <div class="label">
            Exposure bracket
            {#if app.through && !app.settings.bracket_stops}<div class="sub">±1 stop is used when fitting through</div>{/if}
          </div>
          <div class="value">
            <select bind:value={app.settings.bracket_stops} aria-label="Bracket">
              {#each bracketOptions as b (b)}<option value={b}>{b === 0 ? "Off" : `±${b} stop${b === 1 ? "" : "s"}`}</option>{/each}
            </select>
          </div>
        </div>
        <div class="row">
          <div class="label">Error metric</div>
          <div class="value">
            <select bind:value={app.settings.metric} aria-label="Metric">
              <option value="auto">Auto (ΔE2000 SDR, ΔE ITP HDR)</option>
              <option value="delta_e2000">ΔE2000</option>
              <option value="delta_e_itp">ΔE ITP</option>
            </select>
          </div>
        </div>
        <div class="row">
          <div class="label">Fit through<div class="sub">The display transform after the DCTL</div></div>
          <div class="value">
            {#if app.through}
              <span class="file" title={app.through.path}>{app.through.fileName}</span>
              <button class="btn plain" onclick={() => (app.through = null)} title="Don't fit through a transform"><Icon name="x" size={14} /></button>
            {:else}
              <button class="btn" onclick={chooseThrough}>Choose LUT…</button>
            {/if}
          </div>
        </div>
        {#if app.through}
          <div class="row">
            <div class="label">Its input</div>
            <div class="value">
              <select value={app.through.input.id} onchange={(e) => setThroughInput(e.currentTarget.value)} aria-label="Reference input">
                {#each cat.cameras as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
              </select>
            </div>
          </div>
        {/if}
      </div>

      <details class="adv">
        <summary>Advanced</summary>
        <div class="group">
          <div class="row">
            <div class="label">Adaptation</div>
            <div class="value">
              <Segmented bind:value={app.settings.cat} label="CAT" options={[{ value: "Bradford", label: "Bradford" }, { value: "Cat02", label: "CAT02" }]} />
            </div>
          </div>
          <div class="row">
            <div class="label">Hues</div>
            <div class="value">
              <Segmented bind:value={app.settings.hues} label="Hues" options={[{ value: 24, label: "24" }, { value: 36, label: "36" }]} />
            </div>
          </div>
          <div class="row">
            <div class="label">Sample limit</div>
            <div class="value">
              <Segmented
                bind:value={app.settings.ring_limit}
                label="Ring limit"
                options={[
                  { value: "camera_and_display", label: "Camera ∩ display" },
                  { value: "camera", label: "Camera" },
                ]}
              />
            </div>
          </div>
          <div class="row">
            <div class="label">Interpolation</div>
            <div class="value">
              <Segmented
                bind:value={app.settings.interpolation}
                label="Interpolation"
                options={[
                  { value: "tetrahedral", label: "Tetrahedral" },
                  { value: "trilinear", label: "Trilinear" },
                ]}
              />
            </div>
          </div>
          <div class="row">
            <div class="label">Perceptual refinement</div>
            <div class="value"><Toggle bind:checked={app.settings.refine} label="Refine" /></div>
          </div>
        </div>
      </details>
    </div>

    <div class="col right">
      <div class="status">
        {#if app.analyzing}<div class="spinner"></div><span class="muted">Measuring…</span>{/if}
      </div>
      {#if app.analysisError}
        <div class="card error fade-in"><Icon name="warn" /> {app.analysisError}</div>
      {:else if a && stats}
        <div class="card matrix" class:stale={app.analyzing}>
          <div class="mhead">
            <div>
              <h2>M<sub>XYZ</sub></h2>
              <div class="sub muted">Linear CIE XYZ (D65) · {cameraName(a.input, cat)} → {a.output_name}</div>
            </div>
            <div class="actions">
              <button class="btn" onclick={copyMatrix}><Icon name="copy" size={14} /> Copy Matrix</button>
              <button class="btn primary" onclick={exportDctl}><Icon name="export" size={14} /> Export DCTL…</button>
            </div>
          </div>
          <div class="m3 mono selectable">
            {#each a.m_xyz as row, r (r)}
              {#each row as v, c (c)}<div class:diag={r === c}>{v.toFixed(6)}</div>{/each}
            {/each}
          </div>
        </div>

        <div class="stats" class:stale={app.analysisError || app.analyzing}>
          {#each [["Mean", stats.mean], ["95th pct", stats.p95], ["Max", stats.max], ["Neutral", stats.neutral]] as [k, v] (k)}
            <div class="card stat">
              <div class="k muted">{k}</div>
              <div class="v num">{fmt(v as number)}</div>
              <div class="u faint">{metric}</div>
            </div>
          {/each}
        </div>

        {#if a.reference}
          {@const r = a.reference}
          <div class="card ref">
            <div class="refrow">
              <div><div class="muted sub">{r.name} alone</div><div class="num big">{fmt(r.stats_alone.mean)}</div></div>
              <Icon name="chevron" />
              <div><div class="muted sub">with this matrix</div><div class="num big good">{fmt(stats.mean)}</div></div>
              <div class="gain num">{r.gain_percent > 0 ? "−" : "+"}{fmt(Math.abs(r.gain_percent), 0)}%</div>
            </div>
            <div class="sub muted">
              Mean {metric}, exposure matched. A plain colorimetric matrix through this transform would give {fmt(r.stats_colorimetric.mean)}.
            </div>
          </div>
        {/if}

        <div class="card">
          <h3>Samples</h3>
          <Swatches samples={a.samples} {metric} refined={!!a.stats_refined} />
        </div>

        <div class="card notes">
          <h3>Report</h3>
          {#each warnings as d (d.code)}
            <div class="note warn"><Icon name="warn" /><span>{d.message}</span></div>
          {/each}
          {#each infos as d (d.code)}
            <div class="note"><Icon name="info" /><span>{d.message}</span></div>
          {/each}
          <div class="note">
            <Icon name="info" />
            <span>
              {a.samples_used} samples{a.samples_skipped.length
                ? `; skipped ${a.samples_skipped.map(([s, n]) => `${n} ${s.replaceAll("_", " ")}`).join(", ")}`
                : ""}. Hardest hues: {stats.worst_hues.slice(0, 3).map((h) => `${h.name} ${h.hue_deg}° (${fmt(h.mean)})`).join(", ")}.
            </span>
          </div>
        </div>
      {:else}
        <div class="placeholder muted">Measuring the LUT…</div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .page {
    display: grid;
    grid-template-columns: minmax(360px, 420px) minmax(0, 1fr);
    gap: 24px;
    padding: 20px 24px 32px;
    align-items: start;
  }
  @media (max-width: 1080px) {
    .page {
      grid-template-columns: 1fr;
    }
  }
  .col {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .left .group {
    margin-bottom: 20px;
  }
  .right {
    gap: 14px;
  }
  .row.foot {
    min-height: 34px;
    gap: 4px;
  }
  .row select {
    max-width: 230px;
  }
  .grow {
    flex: 1;
  }
  .why {
    padding: 2px 14px 12px;
    font-size: 11.5px;
    color: var(--label-2);
    display: flex;
    flex-direction: column;
    gap: 3px;
    -webkit-user-select: text;
    user-select: text;
  }
  .file {
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .adv summary {
    color: var(--label-2);
    font-size: 12px;
    margin: 0 0 8px 12px;
    cursor: default;
  }
  .status {
    height: 16px;
    margin-top: -6px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .stale {
    opacity: 0.55;
    transition: opacity 0.2s;
  }
  .matrix .mhead {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
    flex-wrap: wrap;
    margin-bottom: 14px;
  }
  h2 {
    font-size: 17px;
    font-weight: 600;
  }
  h3 {
    font-size: 13px;
    font-weight: 600;
    margin-bottom: 12px;
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .m3 {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
    font-size: 15px;
  }
  .m3 div {
    background: var(--surface-2);
    border-radius: 6px;
    padding: 8px 12px;
    text-align: right;
  }
  .m3 .diag {
    font-weight: 600;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
  }
  .stat {
    padding: 12px 14px;
  }
  .stat .k {
    font-size: 11.5px;
  }
  .stat .v {
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 600;
    line-height: 1.15;
  }
  .stat .u {
    font-size: 11px;
  }
  .ref .refrow {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 8px;
  }
  .ref .big {
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 600;
  }
  .ref .good {
    color: var(--green);
  }
  .ref .gain {
    margin-left: auto;
    font-size: 15px;
    font-weight: 600;
    color: var(--green);
  }
  .notes .note {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    font-size: 12px;
    color: var(--label-2);
    padding: 4px 0;
    -webkit-user-select: text;
    user-select: text;
  }
  .note :global(svg) {
    flex: none;
    margin-top: 1px;
  }
  .note.warn {
    color: var(--label);
  }
  .note.warn :global(svg) {
    color: var(--amber);
  }
  .error {
    color: var(--red);
    display: flex;
    gap: 8px;
  }
  .placeholder {
    padding: 40px 0;
    text-align: center;
  }
</style>
