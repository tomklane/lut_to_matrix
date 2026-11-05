<script lang="ts" generics="T">
  // macOS segmented control.
  let {
    options,
    value = $bindable(),
    label = "",
    disabled = false,
  }: { options: { value: T; label: string; title?: string }[]; value: T; label?: string; disabled?: boolean } = $props();
</script>

<div class="seg" role="radiogroup" aria-label={label} class:disabled>
  {#each options as o (o.label)}
    <button
      type="button"
      role="radio"
      aria-checked={value === o.value}
      class:on={value === o.value}
      title={o.title ?? ""}
      {disabled}
      onclick={() => (value = o.value)}>{o.label}</button
    >
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    max-width: 100%;
    padding: 2px;
    gap: 2px;
    background: color-mix(in srgb, var(--label) 7%, transparent);
    border: 0.5px solid var(--border);
    border-radius: 7px;
  }
  .seg.disabled {
    opacity: 0.5;
  }
  button {
    appearance: none;
    font: inherit;
    font-size: 12.5px;
    color: var(--label);
    background: transparent;
    border: 0;
    border-radius: 5px;
    padding: 2px 11px;
    min-height: 22px;
    white-space: nowrap;
    transition: background 0.15s var(--ease), box-shadow 0.15s var(--ease);
  }
  button:not(.on):not(:disabled):hover {
    background: color-mix(in srgb, var(--label) 8%, transparent);
  }
  button.on {
    background: var(--accent);
    color: var(--accent-fg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.18);
    font-weight: 600;
  }
  button:focus-visible {
    outline: 3px solid var(--accent-soft);
  }
</style>
