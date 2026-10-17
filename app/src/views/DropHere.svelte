<script lang="ts">
  import Icon from "../lib/ui/Icon.svelte";
  let { onopen, text = "Drop a LUT here" }: { onopen: () => void; text?: string } = $props();
</script>

<div class="empty">
  <div class="well">
    <Icon name="drop" size={40} />
    <h2>{text}</h2>
    <p class="muted">A camera-log → display 3D LUT: .cube or .3dl</p>
    <button class="btn primary large" onclick={onopen}>Open LUT…</button>
  </div>
</div>

<style>
  .empty {
    height: 100%;
    min-height: 420px;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .well {
    width: min(520px, 100%);
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
  h2 {
    color: var(--label);
    font-size: 18px;
    font-weight: 600;
  }
  p {
    margin: 0 0 8px;
  }
</style>
