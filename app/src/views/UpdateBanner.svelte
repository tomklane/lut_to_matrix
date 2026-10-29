<script lang="ts">
  import { updates } from "../lib/updates.svelte";
  import Icon from "../lib/ui/Icon.svelte";

  const u = $derived(updates.available);
  const show = $derived(!!u && updates.dismissed !== u.version);
  let notes = $state(false);
  // Release notes are Markdown; show them as plain text, headings and list marks removed.
  const plain = $derived(
    (u?.notes ?? "")
      .split("\n")
      .map((l) => l.replace(/^#+\s*/, "").replace(/^\s*[-*]\s+/, "• ").replace(/\*\*|`/g, ""))
      .join("\n")
      .trim(),
  );
</script>

{#if show && u}
  <div class="banner fade-in" role="status">
    <Icon name="export" size={16} />
    <div class="text">
      <b>LUT Matrix {u.version} is available.</b>
      {#if updates.progress !== null}
        <span class="muted">Downloading… {Math.round(100 * updates.progress)}%</span>
      {:else if updates.message}
        <span class="err">{updates.message}</span>
      {:else if plain}
        <button class="btn plain" onclick={() => (notes = !notes)}>{notes ? "Hide" : "What's new"}</button>
      {/if}
    </div>
    <span class="grow"></span>
    {#if updates.progress === null}
      <button class="btn" onclick={() => (updates.dismissed = u.version)}>Later</button>
      <button class="btn primary" onclick={() => updates.install()}>Install and Relaunch</button>
    {:else}
      <div class="bar"><div style="width:{100 * updates.progress}%"></div></div>
    {/if}
  </div>
  {#if notes && plain}
    <pre class="notes selectable">{plain}</pre>
  {/if}
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 12px 22px 0;
    padding: 9px 12px;
    border-radius: 10px;
    background: var(--accent-soft);
    color: var(--label);
  }
  .banner > :global(svg) {
    color: var(--accent);
    transform: rotate(180deg);
    flex: none;
  }
  .text {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .err {
    color: var(--red);
  }
  .bar {
    width: 160px;
    height: 6px;
    border-radius: 3px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s var(--ease);
  }
  .notes {
    margin: 6px 22px 0;
    padding: 10px 14px;
    max-height: 220px;
    overflow: auto;
    background: var(--surface);
    border-radius: 10px;
    box-shadow: var(--shadow);
    font: inherit;
    font-size: 12px;
    white-space: pre-wrap;
    color: var(--label-2);
  }
</style>
