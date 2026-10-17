<script lang="ts">
  // Before/after swatches of the sample ring: one column per hue, one row per saturation.
  // Each tile is split: left = reference (LUT), right = comparison (matrix).
  import type { SampleReport } from "../lib/api";
  import { fmt } from "../lib/state.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";

  let { samples, metric, refined }: { samples: SampleReport[]; metric: string; refined: boolean } = $props();

  type Mode = "matrix" | "input";
  let mode = $state<Mode>("matrix");
  const levels = $derived([...new Set(samples.map((s) => s.stops))].sort((a, b) => a - b));
  let level = $state(0);
  $effect(() => {
    if (!levels.includes(level)) level = levels.includes(0) ? 0 : levels[Math.floor(levels.length / 2)] ?? 0;
  });

  const atLevel = $derived(samples.filter((s) => s.stops === level));
  const hues = $derived([...new Set(atLevel.flatMap((s) => (s.kind.kind === "hue" ? [s.kind.hue_deg] : [])))].sort((a, b) => a - b));
  const sats = $derived(
    [...new Set(atLevel.flatMap((s) => (s.kind.kind === "hue" ? [s.kind.saturation] : [])))].sort((a, b) => b - a),
  );
  const neutral = $derived(atLevel.find((s) => s.kind.kind === "neutral"));
  const grid = $derived(
    new Map(atLevel.flatMap((s) => (s.kind.kind === "hue" ? [[`${s.kind.hue_deg}|${s.kind.saturation}`, s] as const] : []))),
  );

  let picked = $state<SampleReport | null>(null);
  const shown = $derived(picked && atLevel.includes(picked) ? picked : (neutral ?? null));

  function pair(s: SampleReport): [string, string] {
    return mode === "matrix" ? [s.swatch_target, s.swatch_matrix] : [s.swatch_input, s.swatch_lut];
  }
  function err(s: SampleReport): number | null {
    return refined ? s.error_refined : s.error_ls;
  }
  function describe(s: SampleReport): string {
    return s.kind.kind === "neutral" ? "Neutral gray" : `Hue ${s.kind.hue_deg}° · saturation ${Math.round(s.kind.saturation * 100)}%`;
  }
  const statusText: Record<string, string> = {
    outside_camera_gamut: "outside the camera gamut",
    outside_lut_domain: "outside the LUT's input range",
    clipped_output: "clipped by the LUT",
  };
  const labels = $derived(mode === "matrix" ? ["LUT", "Matrix"] : ["Scene (no change)", "LUT"]);
</script>

<div class="head">
  <Segmented
    bind:value={mode}
    label="Compare"
    options={[
      { value: "matrix", label: "LUT vs Matrix" },
      { value: "input", label: "Scene vs LUT" },
    ]}
  />
  {#if levels.length > 1}
    <Segmented
      bind:value={level}
      label="Exposure"
      options={levels.map((l) => ({ value: l, label: l === 0 ? "0" : `${l > 0 ? "+" : "−"}${Math.abs(l)}`, title: `${l} stops` }))}
    />
  {/if}
</div>

<div class="layout">
  <div class="grid" style="--cols:{hues.length}" role="grid" aria-label="Sample swatches">
    {#each sats as sat (sat)}
      {#each hues as hue (hue)}
        {@const s = grid.get(`${hue}|${sat}`)}
        {#if s}
          {@const [a, b] = pair(s)}
          <button
            class="tile"
            class:off={s.status !== "ok"}
            class:sel={shown === s}
            title="{describe(s)}{err(s) != null ? ` · ${metric} ${fmt(err(s)!)}` : ''}{s.status !== 'ok' ? ` · ${statusText[s.status]}` : ''}"
            onmouseenter={() => (picked = s)}
            onfocus={() => (picked = s)}
            onclick={() => (picked = s)}
            style="--a:{a};--b:{b}"
            aria-label={describe(s)}
          ></button>
        {:else}
          <div class="tile empty"></div>
        {/if}
      {/each}
    {/each}
  </div>

  {#if shown}
    {@const [a, b] = pair(shown)}
    <div class="detail fade-in">
      <div class="big">
        <div style="background:{a}"><span>{labels[0]}</span></div>
        <div style="background:{b}"><span>{labels[1]}</span></div>
      </div>
      <div class="what">{describe(shown)}</div>
      <div class="muted num">
        {#if shown.status !== "ok"}Skipped: {statusText[shown.status]}
        {:else if err(shown) != null}{metric} {fmt(err(shown)!)}{/if}
      </div>
      <div class="hex mono faint selectable">{a} · {b}</div>
    </div>
  {/if}
</div>
<p class="legend faint">
  Each tile: left {labels[0]}, right {labels[1]}. Swatches are sRGB previews normalised so gray sits at 18%;
  hover a tile for its values.
</p>

<style>
  .head {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    margin-bottom: 12px;
  }
  .layout {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .grid {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    gap: 2px;
    min-width: 0;
  }
  .tile {
    appearance: none;
    border: 0;
    padding: 0;
    aspect-ratio: 1 / 2;
    border-radius: 3px;
    background: linear-gradient(90deg, var(--a) 50%, var(--b) 50%);
    outline-offset: 1px;
  }
  .tile.empty {
    background: var(--surface-2);
  }
  .tile.off {
    opacity: 0.25;
  }
  .tile.sel {
    outline: 2px solid var(--label);
    position: relative;
    z-index: 1;
  }
  .detail {
    display: grid;
    grid-template-columns: 168px 1fr;
    grid-template-rows: auto auto auto 1fr;
    column-gap: 14px;
    align-items: start;
  }
  .big {
    grid-row: 1 / 5;
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-radius: 8px;
    overflow: hidden;
    height: 72px;
    box-shadow: var(--shadow);
  }
  .big div {
    display: flex;
    align-items: flex-end;
    padding: 5px 6px;
  }
  .big span {
    font-size: 10px;
    font-weight: 600;
    color: #fff;
    text-shadow: 0 0 3px rgba(0, 0, 0, 0.6);
  }
  .what {
    margin-top: 6px;
    font-weight: 500;
    font-size: 12px;
  }
  .hex {
    font-size: 10.5px;
    margin-top: 2px;
  }
  .legend {
    font-size: 11px;
    margin: 10px 0 0;
  }
</style>
