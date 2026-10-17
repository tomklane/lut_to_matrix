<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./lib/api";
  import { app, cameraName, gamutLabel, transferLabel, type View } from "./lib/state.svelte";
  import { listenForDrops, openFile, LUT_FILTER } from "./lib/platform";
  import Icon from "./lib/ui/Icon.svelte";
  import MatrixView from "./views/MatrixView.svelte";
  import OdrtView from "./views/OdrtView.svelte";
  import PresetsView from "./views/PresetsView.svelte";
  import About from "./views/About.svelte";

  import { analysisEffect } from "./lib/runner.svelte";

  let ready = $state(false);
  analysisEffect();
  const mac = $derived(app.info?.os === "macos");

  const views: { id: View; label: string; icon: string }[] = [
    { id: "matrix", label: "Matrix", icon: "matrix" },
    { id: "opendrt", label: "OpenDRT Match", icon: "curve" },
    { id: "presets", label: "Presets", icon: "library" },
  ];
  const titles: Record<View, string> = { matrix: "Matrix", opendrt: "OpenDRT Match", presets: "Preset Library" };

  onMount(async () => {
    try {
      [app.info, app.catalog] = await Promise.all([api.appInfo(), api.catalog()]);
      app.applyPrefs(await api.loadPrefs());
    } catch (e) {
      app.notify(String(e), "error");
    }
    ready = true;
    app.refreshPresets();
    listenForDrops(
      (paths) => {
        const lut = paths.find((p) => /\.(cube|3dl)$/i.test(p));
        if (lut) app.openLut(lut);
        else app.notify("Drop a .cube or .3dl LUT", "error");
      },
      (over) => (app.dragging = over),
    );
    const first = await api.initialFile().catch(() => null);
    if (first) app.openLut(first);
  });

  // Remember settings (debounced).
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const snap = app.prefsSnapshot();
    if (!ready) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => api.savePrefs(snap).catch(() => {}), 400);
  });

  async function chooseLut() {
    const p = await openFile([LUT_FILTER], app.lastDir || undefined);
    if (p) app.openLut(p);
  }

  function onKey(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === "o") {
      e.preventDefault();
      chooseLut();
    } else if (mod && ["1", "2", "3"].includes(e.key)) {
      e.preventDefault();
      app.view = views[Number(e.key) - 1].id;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="shell" class:mac>
  <aside class="sidebar">
    <div class="sidebar-top" data-tauri-drag-region></div>
    <div class="brand" data-tauri-drag-region>LUT Matrix</div>
    <nav>
      {#each views as v, i (v.id)}
        <button class="nav" class:on={app.view === v.id} onclick={() => (app.view = v.id)} title="{v.label} ({mac ? '⌘' : 'Ctrl+'}{i + 1})">
          <Icon name={v.icon} size={17} />
          <span>{v.label}</span>
          {#if v.id === "presets" && app.presets?.presets.length}
            <span class="count">{app.presets.presets.length}</span>
          {/if}
        </button>
      {/each}
    </nav>
    <div class="grow"></div>
    {#if app.lut && app.input && app.output}
      <div class="lutcard fade-in" title={app.lut.path}>
        <div class="lutname">{app.lut.file_name}</div>
        <div class="lutspaces">{cameraName(app.input, app.catalog)}</div>
        <div class="lutspaces">→ {gamutLabel(app.output.gamut, app.catalog)} · {transferLabel(app.output)}</div>
      </div>
    {/if}
    <button class="about" onclick={() => (app.showAbout = true)}>About LUT Matrix</button>
  </aside>

  <main>
    <header class="toolbar" data-tauri-drag-region>
      <div class="titles" data-tauri-drag-region>
        <h1 data-tauri-drag-region>{titles[app.view]}</h1>
        {#if app.lut}<div class="subtitle" data-tauri-drag-region>{app.lut.file_name}</div>{/if}
      </div>
      <div class="tools">
        {#if app.loadingLut}<div class="spinner"></div>{/if}
        <button class="btn" onclick={chooseLut} title="Open a LUT ({mac ? '⌘' : 'Ctrl+'}O)"><Icon name="open" size={15} /> Open LUT…</button>
      </div>
    </header>
    <section class="content">
      {#if ready}
        {#if app.view === "matrix"}
          <MatrixView onopen={chooseLut} />
        {:else if app.view === "opendrt"}
          <OdrtView onopen={chooseLut} />
        {:else}
          <PresetsView />
        {/if}
      {/if}
    </section>
  </main>

  {#if app.dragging}
    <div class="dropveil fade-in">
      <div class="dropbox"><Icon name="drop" size={34} /><div>Drop to open the LUT</div></div>
    </div>
  {/if}

  {#if app.toast}
    <div class="toast fade-in" class:error={app.toast.kind === "error"} role="status">{app.toast.text}</div>
  {/if}

  {#if app.showAbout}<About />{/if}
</div>

<style>
  .shell {
    display: flex;
    height: 100%;
  }
  .sidebar {
    width: 216px;
    flex: none;
    background: var(--sidebar);
    border-right: 0.5px solid var(--separator);
    display: flex;
    flex-direction: column;
    padding: 0 10px 12px;
  }
  .sidebar-top {
    height: 12px;
  }
  .mac .sidebar-top {
    height: 40px;
  }
  .brand {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 11px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--label-3);
    padding: 6px 10px 8px;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .nav {
    appearance: none;
    font: inherit;
    color: var(--label);
    background: transparent;
    border: 0;
    border-radius: 6px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    text-align: left;
    min-height: 30px;
  }
  .nav :global(svg) {
    color: var(--accent);
  }
  .nav.on {
    background: color-mix(in srgb, var(--label) 10%, transparent);
    font-weight: 500;
  }
  .nav .count {
    margin-left: auto;
    font-size: 11px;
    color: var(--label-2);
  }
  .grow {
    flex: 1;
  }
  .lutcard {
    background: color-mix(in srgb, var(--surface) 70%, transparent);
    border-radius: 8px;
    padding: 9px 10px;
    margin-bottom: 8px;
    box-shadow: var(--shadow);
    min-width: 0;
  }
  .lutname {
    font-weight: 600;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lutspaces {
    font-size: 11px;
    color: var(--label-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .about {
    appearance: none;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: 12px;
    color: var(--label-2);
    text-align: left;
    padding: 4px 10px;
  }
  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .toolbar {
    height: 52px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 0 18px 0 24px;
    border-bottom: 0.5px solid var(--separator);
    background: var(--bg);
  }
  .titles {
    min-width: 0;
  }
  h1 {
    font-size: 15px;
    font-weight: 600;
  }
  .subtitle {
    font-size: 11.5px;
    color: var(--label-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .content {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  .dropveil {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--bg) 70%, transparent);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    display: grid;
    place-items: center;
    z-index: 50;
    pointer-events: none;
  }
  .dropbox {
    border: 2px dashed var(--accent);
    color: var(--accent);
    border-radius: 18px;
    padding: 40px 64px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    font-size: 15px;
    font-weight: 600;
    background: var(--surface);
  }
  .toast {
    position: fixed;
    bottom: 20px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--surface);
    color: var(--label);
    box-shadow: var(--shadow-pop);
    border-radius: 10px;
    padding: 9px 16px;
    max-width: min(640px, 80vw);
    z-index: 60;
    -webkit-user-select: text;
    user-select: text;
  }
  .toast.error {
    color: var(--red);
  }
</style>
