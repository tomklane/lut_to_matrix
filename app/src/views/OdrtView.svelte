<script lang="ts">
  import { onDestroy } from "svelte";
  import { api, DIFFUSE_WHITE_STOPS, type GrayPoint } from "../lib/api";
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

  // Gray axis (D33): the LUT's cast from D65 per stop, as measured before correction.
  const VISIBLE_L = 10;
  const VISIBLE_DUV = 0.005;
  function castName(p: GrayPoint): string {
    if (p.duv < 0.0015) return "neutral";
    const h = ((Math.atan2(p.b, p.a) * 180) / Math.PI + 360) % 360;
    if (h >= 20 && h < 70) return "red-orange";
    if (h >= 70 && h < 105) return "yellow";
    if (h >= 105 && h < 150) return "yellow-green";
    if (h >= 150 && h < 200) return "green";
    if (h >= 200 && h < 250) return "cyan";
    if (h >= 250 && h < 300) return "blue";
    if (h >= 300 && h < 340) return "magenta";
    return "red-magenta";
  }
  // The cast's hue at a fixed lightness, chroma exaggerated so it reads at a glance.
  function castColour(p: GrayPoint): string {
    const c = Math.hypot(p.a, p.b);
    if (p.duv < 0.0015 || c < 1e-6) return "var(--label-3)";
    return `lab(62% ${(45 * p.a) / c} ${(45 * p.b) / c})`;
  }
  type GrayView = "lut" | "corrected";
  let grayView = $state<GrayView>("corrected");
  const hasCorrected = $derived((m?.gray_axis_corrected?.length ?? 0) > 0);
  const showCorrected = $derived(hasCorrected && grayView === "corrected");
  const gray = $derived((showCorrected ? m?.gray_axis_corrected : m?.gray_axis) ?? []);
  const grayLut = $derived(m?.gray_axis ?? []);
  const grayVisible = $derived(gray.filter((p) => p.l >= VISIBLE_L));
  const grayWorst = $derived(grayVisible.length ? grayVisible.reduce((a, b) => (b.duv > a.duv ? b : a)) : null);
  // One scale for both views, so switching shows what the correction changed.
  const grayScale = $derived(
    Math.max(0.015, ...[...grayLut, ...(m?.gray_axis_corrected ?? [])].filter((p) => p.l >= VISIBLE_L).map((p) => p.duv)),
  );
  const neutralOn = $derived(!!m && (m.neutral_gray ?? "off") !== "off");
  const neutralLabel: Record<string, string> = { middle_gray: "middle gray", exposure: "every level", white_gain: "white gain" };
  const wg = $derived(m?.white_gain ?? null);

  const bars = $derived(
    m
      ? [
          { label: "Stock OpenDRT", sub: "Standard look, ACES-2.0 tonescale, 10 nits grey", s: m.stock },
          { label: "Best preset", sub: m.start.description, s: m.best_preset },
          {
            label: "Fitted",
            sub: `grey ${fmt(m.config.grey_nits)} nits${m.frozen.length ? ` · frozen: ${m.frozen.join(", ")}` : ""}${
              neutralOn ? ` · neutral gray (${neutralLabel[m.neutral_gray!]}, ${Math.round(100 * (m.neutral_amount ?? 1))}%)` : ""
            }`,
            s: m.fitted,
          },
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
        <div class="label">
          Neutralize gray<div class="sub">Remove the LUT's gray-axis cast before matching (OpenDRT's gray is always neutral)</div>
        </div>
        <div class="value">
          <Segmented
            bind:value={app.match.neutral_gray}
            label="Neutralize gray"
            options={[
              { value: "off", label: "Off", title: "Match the LUT as it is, cast included" },
              { value: "middle_gray", label: "Middle gray", title: "One white balance, taken from the LUT's 18% gray, for every colour" },
              { value: "exposure", label: "Every level", title: "White-balance each brightness level to the LUT's own gray there" },
              { value: "white_gain", label: "White gain", title: "Balance the LUT's white with one gain after the LUT, like a Gain node; darker levels keep their look relative to it" },
            ]}
          />
        </div>
      </div>
      {#if app.match.neutral_gray === "white_gain"}
        <div class="row">
          <div class="label">White at<div class="sub">The level made D65 by the gain</div></div>
          <div class="value">
            <Segmented
              bind:value={app.match.white_stops}
              label="White level"
              options={[
                { value: DIFFUSE_WHITE_STOPS, label: "Diffuse white", title: "A 90% white card, +2.3 stops over 18% gray" },
                { value: 3, label: "+3 stops" },
                { value: 5, label: "+5 stops" },
              ]}
            />
          </div>
        </div>
      {/if}
      {#if app.match.neutral_gray !== "off"}
        <div class="row">
          <div class="label">Amount<div class="sub">Less than 100% keeps some of the cast</div></div>
          <div class="value">
            <Segmented
              bind:value={app.match.neutral_amount}
              label="Neutralize amount"
              options={[
                { value: 0.5, label: "50%" },
                { value: 0.75, label: "75%" },
                { value: 1, label: "100%" },
              ]}
            />
          </div>
        </div>
      {/if}
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
              <div class="bv num">
                <b>{fmt(b.s.mean)}</b> <span class="faint">p95 {fmt(b.s.p95)} · max {fmt(b.s.max)}</span>
                {#if b.s.gray_mean !== undefined}<div class="faint">gray axis {fmt(b.s.gray_mean)}</div>{/if}
              </div>
            </div>
          {/each}
        </div>

        {#if gray.length}
          <div class="group-title">Gray axis — cast from D65</div>
          <div class="card gray">
            {#if hasCorrected}
              <div class="gray-head">
                <span class="muted">{showCorrected ? "What the fit matched: the LUT after the correction" : "The LUT as it is"}</span>
                <Segmented
                  bind:value={grayView}
                  label="Gray axis view"
                  options={[
                    { value: "lut", label: "LUT" },
                    { value: "corrected", label: "After correction" },
                  ]}
                />
              </div>
            {/if}
            <div class="strip" role="img" aria-label="Gray-axis cast by exposure">
              <div class="jnd" style="bottom:{(100 * VISIBLE_DUV) / grayScale}%" title="Δu′v′ {VISIBLE_DUV}: a clearly visible cast"></div>
              {#each gray as p (p.stops)}
                <div class="col" class:dim={p.l < VISIBLE_L} title="{p.stops >= 0 ? '+' : ''}{p.stops} stops · {fmt(p.nits, 2)} nits · Δu′v′ {fmt(p.duv, 4)} · {castName(p)}">
                  <div class="g" style="height:{Math.min(100, (100 * p.duv) / grayScale)}%; background:{castColour(p)}"></div>
                </div>
              {/each}
            </div>
            <div class="axis num faint">
              {#each gray as p (p.stops)}<span>{p.stops >= 0 ? "+" : ""}{p.stops}</span>{/each}
            </div>
            <div class="sub muted">
              {#if grayWorst && grayWorst.duv > VISIBLE_DUV}
                Largest visible cast Δu′v′ {fmt(grayWorst.duv, 4)}, {castName(grayWorst)}, at {grayWorst.stops >= 0 ? "+" : ""}{grayWorst.stops} stops.
                {#if !neutralOn}Neutralize gray can remove it before matching.{/if}
              {:else}
                Neutral to within Δu′v′ {fmt(grayWorst?.duv ?? 0, 4)}.
              {/if}
              Faded bars are too dark to see (L* below {VISIBLE_L}).
            </div>
            {#if wg}
              <div class="sub num gain">
                Gain after the LUT, white at {wg.stops >= 0 ? "+" : ""}{fmt(wg.stops, 2)} stops: linear R ×{fmt(wg.linear[0], 3)} · G ×{fmt(wg.linear[1], 3)} · B ×{fmt(wg.linear[2], 3)}
                {#if wg.signal}— on the display signal (a Gain control) R {fmt(wg.signal[0], 3)} · G {fmt(wg.signal[1], 3)} · B {fmt(wg.signal[2], 3)}{/if}
                <div class="faint">
                  {#if (wg.headroom ?? 1) < 1}
                    Scaled down ×{fmt(wg.headroom ?? 1, 3)} ({fmt(Math.log2(wg.headroom ?? 1), 2)} stops) so no LUT output goes above 1.
                  {:else}
                    No LUT output goes above 1 with this gain.
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        {/if}

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
  .gray {
    gap: 6px;
  }
  .gain {
    color: var(--label);
  }
  .gray-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 4px;
  }
  .strip {
    position: relative;
    height: 64px;
    display: flex;
    align-items: flex-end;
    gap: 3px;
    border-bottom: 1px solid var(--separator);
  }
  .strip .col {
    flex: 1;
    height: 100%;
    display: flex;
    align-items: flex-end;
  }
  .strip .col.dim {
    opacity: 0.3;
  }
  .strip .g {
    width: 100%;
    min-height: 2px;
    border-radius: 3px 3px 0 0;
  }
  .jnd {
    position: absolute;
    left: 0;
    right: 0;
    border-top: 1px dashed color-mix(in srgb, var(--label) 30%, transparent);
    pointer-events: none;
  }
  .axis {
    display: flex;
    gap: 3px;
    font-size: 10.5px;
  }
  .axis span {
    flex: 1;
    text-align: center;
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
  .row.note .label {
    flex: 1;
    white-space: normal;
  }
  input[type="text"] {
    width: 260px;
  }
  .small {
    font-size: 11.5px;
    margin: -10px 0 0 12px;
  }
</style>
