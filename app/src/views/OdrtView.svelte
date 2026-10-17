<script lang="ts">
  import { onDestroy } from "svelte";
  import { api } from "../lib/api";
  import { app, cameraName, fmt, gamutLabel, metricLabel, transferLabel } from "../lib/state.svelte";
  import { confirmAsk } from "../lib/platform";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import DropHere from "./DropHere.svelte";

  let { onopen }: { onopen: () => void } = $props();

  const m = $derived(app.odrt && app.odrtForLut === app.lut?.path ? app.odrt : null);
  const metric = $derived(m ? metricLabel(m.metric) : "ΔE");

  type Brl = "auto" | "free" | "frozen";
  function getBrl(): Brl {
    if (app.match.freeze.includes("brl")) return "frozen";
    return app.match.auto_freeze_brilliance ? "auto" : "free";
  }
  function setBrl(v: Brl) {
    app.match.freeze = v === "frozen" ? ["brl"] : [];
    app.match.auto_freeze_brilliance = v === "auto";
  }
  type Effort = "quick" | "standard" | "thorough";
  const efforts: Record<Effort, [number, number]> = { quick: [40, 2], standard: [150, 3], thorough: [300, 4] };
  function getEffort(): Effort {
    return app.match.max_iterations <= 60 ? "quick" : app.match.max_iterations >= 250 ? "thorough" : "standard";
  }
  function setEffort(e: Effort) {
    [app.match.max_iterations, app.match.starts] = efforts[e];
  }

  let elapsed = $state(0);
  let timer: ReturnType<typeof setInterval> | undefined;
  onDestroy(() => clearInterval(timer));

  let presetName = $state("");

  async function run() {
    if (!app.lut || !app.input || !app.output || app.odrtRunning) return;
    const path = app.lut.path;
    app.odrtRunning = true;
    app.odrtError = null;
    elapsed = 0;
    const t0 = Date.now();
    timer = setInterval(() => (elapsed = (Date.now() - t0) / 1000), 200);
    try {
      const r = await api.opendrtFit({
        path,
        input: $state.snapshot(app.input),
        output: $state.snapshot(app.output),
        settings: $state.snapshot(app.match),
      });
      app.odrt = r;
      app.odrtForLut = path;
      presetName = await api.defaultPresetName(path);
    } catch (e) {
      app.odrtError = String(e);
    } finally {
      clearInterval(timer);
      app.odrtRunning = false;
    }
  }

  async function save() {
    const name = presetName.trim();
    if (!name) return;
    try {
      let replace = false;
      if (await api.presetExists(name)) {
        replace = await confirmAsk(`A preset called “${name}” is already in your library. Replace it?`, "Replace preset", "Replace");
        if (!replace) return;
      }
      const saved = await api.presetSave(name, replace);
      app.notify(`Saved “${saved}” to your preset library`);
      app.refreshPresets();
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  const bars = $derived(
    m
      ? [
          { label: "Stock OpenDRT", sub: "Standard look, ACES-2.0 tonescale, 10 nits grey", s: m.stock },
          { label: "Best preset", sub: m.start.description, s: m.best_preset },
          { label: "Fitted", sub: `grey ${fmt(m.config.grey_nits)} nits${m.frozen.length ? ` · frozen: ${m.frozen.join(", ")}` : ""}`, s: m.fitted },
        ]
      : [],
  );
  const scale = $derived(m ? Math.max(m.stock.mean, m.best_preset.mean, m.fitted.mean) * 1.05 : 1);
  const smoothOk = $derived(m ? m.safety.roughness <= 1.5 * m.safety.stock_roughness : true);
</script>

{#if !app.lut}
  <DropHere {onopen} text="Drop the LUT to match" />
{:else if app.input && app.output}
  <div class="page">
    <p class="intro muted">
      Fits every OpenDRT look parameter and the grey level so that OpenDRT — fed DaVinci Wide Gamut / Intermediate —
      reproduces this LUT. Save the result to your library and build an OpenDRT DCTL with it as a Look Preset.
    </p>

    <div class="group-title">Match</div>
    <div class="group">
      <div class="row">
        <div class="label">LUT</div>
        <div class="value"><span class="strong">{app.lut.file_name}</span></div>
      </div>
      <div class="row">
        <div class="label">Spaces</div>
        <div class="value">
          <span class="muted">{cameraName(app.input, app.catalog)} → {gamutLabel(app.output.gamut, app.catalog)} · {transferLabel(app.output)}</span>
          <button class="btn plain" onclick={() => (app.view = "matrix")}>Change…</button>
        </div>
      </div>
      <div class="row">
        <div class="label">Brilliance<div class="sub">Auto freezes it if colours darken as exposure rises</div></div>
        <div class="value">
          <Segmented
            bind:value={getBrl, setBrl}
            label="Brilliance"
            options={[
              { value: "auto", label: "Auto" },
              { value: "free", label: "Fit" },
              { value: "frozen", label: "Keep preset" },
            ]}
          />
        </div>
      </div>
      <div class="row">
        <div class="label">Smoothing<div class="sub">Pull toward the preset where the LUT has no data</div></div>
        <div class="value">
          <Segmented
            bind:value={app.match.regularization}
            label="Smoothing"
            options={[
              { value: 0, label: "Off" },
              { value: 0.1, label: "Light" },
              { value: 0.3, label: "Strong" },
            ]}
          />
        </div>
      </div>
      <div class="row">
        <div class="label">Effort</div>
        <div class="value">
          <Segmented
            bind:value={getEffort, setEffort}
            label="Effort"
            options={[
              { value: "quick", label: "Quick" },
              { value: "standard", label: "Standard" },
              { value: "thorough", label: "Thorough" },
            ]}
          />
        </div>
      </div>
      <div class="row go">
        {#if app.odrtRunning}
          <div class="spinner"></div>
          <span class="muted">Searching 378 OpenDRT presets, then refining the best {app.match.starts}… {elapsed.toFixed(0)} s</span>
        {:else if app.odrtError}
          <span class="err"><Icon name="warn" /> {app.odrtError}</span>
        {/if}
        <span class="grow"></span>
        <button class="btn primary large" onclick={run} disabled={app.odrtRunning}><Icon name="play" size={14} /> Match OpenDRT</button>
      </div>
    </div>

    {#if m}
      <div class="results fade-in" class:stale={app.odrtRunning}>
        <div class="group-title">Result — mean {metric} over {m.samples_used} samples</div>
        <div class="card">
          {#each bars as b (b.label)}
            <div class="bar">
              <div class="bl">
                <div class="strong">{b.label}</div>
                <div class="sub muted">{b.sub}</div>
              </div>
              <div class="track"><div class="fill" class:best={b.label === "Fitted"} style="width:{(100 * b.s.mean) / scale}%"></div></div>
              <div class="bv num"><b>{fmt(b.s.mean)}</b> <span class="faint">p95 {fmt(b.s.p95)} · max {fmt(b.s.max)}</span></div>
            </div>
          {/each}
        </div>

        <div class="group-title">Safety checks</div>
        <div class="group">
          <div class="row" class:bad={!m.safety.finite_in_range}>
            <Icon name={m.safety.finite_in_range ? "check" : "warn"} />
            <div class="label">Output stays finite and in range</div>
          </div>
          <div class="row" class:bad={m.safety.reversals.length > 0}>
            <Icon name={m.safety.reversals.length ? "warn" : "check"} />
            <div class="label">
              Brightness never falls as exposure rises
              {#each m.safety.reversals.slice(0, 3) as r (r.colour)}
                <div class="sub">{r.colour}: −{fmt(r.drop_percent, 1)}% at {r.at_stops >= 0 ? "+" : ""}{r.at_stops} stops</div>
              {/each}
            </div>
          </div>
          <div class="row" class:bad={!smoothOk}>
            <Icon name={smoothOk ? "check" : "warn"} />
            <div class="label">
              Smooth like the stock looks
              <div class="sub num">sharpest transition {fmt(m.safety.roughness, 3)} vs {fmt(m.safety.stock_roughness, 3)} for stock</div>
            </div>
          </div>
          {#each m.notes as n}
            <div class="row note"><Icon name="info" /><div class="label sub">{n}</div></div>
          {/each}
        </div>

        <div class="group-title">Save</div>
        <div class="group">
          <div class="row">
            <div class="label">Preset name</div>
            <div class="value">
              <input type="text" bind:value={presetName} maxlength="60" spellcheck="false" aria-label="Preset name" />
              <button class="btn primary" onclick={save} disabled={!presetName.trim()}>Save to Library</button>
            </div>
          </div>
        </div>
        <p class="faint small">Presets are kept only on this computer, in your preset library.</p>
      </div>
    {/if}
  </div>
{/if}

<style>
  .page {
    max-width: 860px;
    padding: 20px 24px 40px;
  }
  .intro {
    margin: 0 0 18px 2px;
    max-width: 680px;
  }
  .group {
    margin-bottom: 20px;
  }
  .strong {
    font-weight: 500;
  }
  .row.go {
    min-height: 54px;
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
    opacity: 0.5;
  }
  .card {
    margin-bottom: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .bar {
    display: grid;
    grid-template-columns: 220px 1fr 190px;
    align-items: center;
    gap: 14px;
  }
  .bl {
    min-width: 0;
  }
  .bl .sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
  .bv {
    font-size: 12px;
  }
  .bv b {
    font-size: 15px;
  }
  .row > :global(svg) {
    flex: none;
    color: var(--green);
  }
  .row.bad > :global(svg) {
    color: var(--amber);
  }
  .row.note > :global(svg) {
    color: var(--label-3);
  }
  .row .label {
    min-width: 0;
  }
  input[type="text"] {
    width: 260px;
  }
  .small {
    font-size: 11.5px;
    margin: -10px 0 0 12px;
  }
</style>
