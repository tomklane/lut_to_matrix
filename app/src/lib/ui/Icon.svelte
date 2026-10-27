<script lang="ts">
  // A few line icons in the spirit of SF Symbols (drawn here, 20×20 grid).
  let { name, size = 16 }: { name: string; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "matrix"}
    <rect x="3" y="3" width="4" height="4" rx="1" /><rect x="8" y="3" width="4" height="4" rx="1" /><rect x="13" y="3" width="4" height="4" rx="1" />
    <rect x="3" y="8" width="4" height="4" rx="1" /><rect x="8" y="8" width="4" height="4" rx="1" /><rect x="13" y="8" width="4" height="4" rx="1" />
    <rect x="3" y="13" width="4" height="4" rx="1" /><rect x="8" y="13" width="4" height="4" rx="1" /><rect x="13" y="13" width="4" height="4" rx="1" />
  {:else if name === "curve"}
    <path d="M3 17h14M3 17V3" /><path d="M3 15c4 0 5-3 7-6s3-5 7-6" />
  {:else if name === "library"}
    <rect x="3" y="4" width="14" height="3" rx="1" /><rect x="3" y="9" width="14" height="3" rx="1" /><rect x="3" y="14" width="14" height="3" rx="1" />
  {:else if name === "open"}
    <path d="M3 6.5V15a1.5 1.5 0 0 0 1.5 1.5h11A1.5 1.5 0 0 0 17 15V8a1.5 1.5 0 0 0-1.5-1.5H10L8.5 4.5h-4A1.5 1.5 0 0 0 3 6Z" />
  {:else if name === "export"}
    <path d="M10 3v9M6.5 6.5 10 3l3.5 3.5" /><path d="M5 10H4.5A1.5 1.5 0 0 0 3 11.5v4A1.5 1.5 0 0 0 4.5 17h11a1.5 1.5 0 0 0 1.5-1.5v-4a1.5 1.5 0 0 0-1.5-1.5H15" />
  {:else if name === "copy"}
    <rect x="7" y="7" width="10" height="10" rx="2" /><path d="M13 7V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h2" />
  {:else if name === "drop"}
    <path d="M10 3v10M6 9l4 4 4-4" /><path d="M3 14v1.5A1.5 1.5 0 0 0 4.5 17h11a1.5 1.5 0 0 0 1.5-1.5V14" />
  {:else if name === "check"}
    <path d="m5 10.5 3.2 3L15 6.5" />
  {:else if name === "warn"}
    <path d="M10 3.5 2.8 16h14.4Z" /><path d="M10 8.5v3.5M10 14.2v.1" />
  {:else if name === "info"}
    <circle cx="10" cy="10" r="7" /><path d="M10 9v5M10 6.5v.1" />
  {:else if name === "x"}
    <path d="m6 6 8 8M14 6l-8 8" />
  {:else if name === "plus"}
    <path d="M10 4v12M4 10h12" />
  {:else if name === "trash"}
    <path d="M4 6h12M8 6V4h4v2M5.5 6l.8 10a1.5 1.5 0 0 0 1.5 1.4h4.4a1.5 1.5 0 0 0 1.5-1.4l.8-10" />
  {:else if name === "pencil"}
    <path d="M13 4.5 15.5 7 7.5 15H5v-2.5Z" />
  {:else if name === "chevron"}
    <path d="m8 5 5 5-5 5" />
  {:else if name === "play"}
    <path d="M6.5 4.5v11l9-5.5Z" />
  {:else if name === "stack"}
    <path d="M10 3 17 6.5 10 10 3 6.5Z" /><path d="m3 10 7 3.5 7-3.5" /><path d="m3 13.5 7 3.5 7-3.5" />
  {:else if name === "stop"}
    <rect x="5" y="5" width="10" height="10" rx="1.5" />
  {:else if name === "swap"}
    <path d="M4 7h11l-3-3M16 13H5l3 3" />
  {/if}
</svg>
