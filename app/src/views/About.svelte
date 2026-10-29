<script lang="ts">
  import { app } from "../lib/state.svelte";
  import { inTauri } from "../lib/api";
  import { updates } from "../lib/updates.svelte";
  import Toggle from "../lib/ui/Toggle.svelte";

  function close() {
    app.showAbout = false;
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="veil fade-in" onclick={close}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="about-title" tabindex="-1" onclick={(e) => e.stopPropagation()}>
    <div class="icon" aria-hidden="true">
      {#each [0, 1, 2, 3, 4, 5, 6, 7, 8] as i (i)}<span class="c{i}"></span>{/each}
    </div>
    <h2 id="about-title">LUT Matrix</h2>
    <div class="ver muted">Version {app.info?.version ?? "?"}</div>
    <p>
      Fits a 3×3 matrix in linear CIE XYZ to a camera-log → display LUT and exports it as a DaVinci Resolve DCTL;
      matches OpenDRT's look parameters to LUTs.
    </p>
    <p class="small muted">
      © 2026 tomklane. Free software under the GNU GPL v3 or later.<br />
      Includes OpenDRT v1.1.0 © Jed Smith (GPLv3), github.com/jedypod/open-display-transform.
    </p>
    {#if app.info?.data_dir}<p class="small faint selectable">Data: {app.info.data_dir}</p>{/if}
    {#if inTauri}
      <div class="upd">
        {#if updates.available}
          <div class="small">Version {updates.available.version} is available.</div>
          <button class="btn primary" onclick={() => updates.install()} disabled={updates.progress !== null}>
            {updates.progress !== null ? `Installing… ${Math.round(100 * updates.progress)}%` : "Install and Relaunch"}
          </button>
        {:else}
          <button class="btn" onclick={() => updates.check(true)} disabled={updates.checking}>
            {updates.checking ? "Checking…" : "Check for Updates"}
          </button>
        {/if}
        {#if updates.message}<div class="small muted">{updates.message}</div>{/if}
        <label class="small muted auto"><Toggle bind:checked={updates.auto} label="Check for updates at launch" /> Check for updates at launch</label>
      </div>
    {/if}
    <button class="btn primary" onclick={close}>OK</button>
  </div>
</div>

<style>
  .veil {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.25);
    display: grid;
    place-items: center;
    z-index: 70;
  }
  .upd {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    margin: 4px 0 16px;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .sheet {
    width: 380px;
    background: var(--surface);
    border-radius: 14px;
    box-shadow: var(--shadow-pop);
    padding: 26px 28px 22px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .icon {
    width: 64px;
    height: 64px;
    border-radius: 15px;
    background: linear-gradient(#26282f, #121419);
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 4px;
    padding: 10px;
    margin-bottom: 12px;
  }
  .icon span {
    border-radius: 3px;
    background: #4e525f;
  }
  .icon .c0 {
    background: #ff5c52;
  }
  .icon .c4 {
    background: #4cd678;
  }
  .icon .c8 {
    background: #408cff;
  }
  h2 {
    font-size: 17px;
  }
  .ver {
    font-size: 12px;
    margin-bottom: 8px;
  }
  p {
    margin: 6px 0;
  }
  .small {
    font-size: 11.5px;
  }
  .btn {
    margin-top: 12px;
    min-width: 80px;
  }
</style>
