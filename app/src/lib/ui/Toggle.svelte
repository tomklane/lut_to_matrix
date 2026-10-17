<script lang="ts">
  // macOS switch.
  let { checked = $bindable(), label = "", disabled = false }: { checked: boolean; label?: string; disabled?: boolean } =
    $props();
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  class="sw"
  class:on={checked}
  {disabled}
  onclick={() => (checked = !checked)}><span></span></button
>

<style>
  .sw {
    appearance: none;
    width: 38px;
    height: 22px;
    border-radius: 11px;
    border: 0;
    padding: 0;
    background: color-mix(in srgb, var(--label) 16%, transparent);
    position: relative;
    transition: background 0.18s var(--ease);
    flex: none;
  }
  .sw.on {
    background: var(--accent);
  }
  .sw:disabled {
    opacity: 0.45;
  }
  span {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform 0.18s var(--ease);
  }
  .sw.on span {
    transform: translateX(16px);
  }
  .sw:focus-visible {
    outline: 3px solid var(--accent-soft);
  }
</style>
