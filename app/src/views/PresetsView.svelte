<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { app, fmt } from "../lib/state.svelte";
  import { baseName, confirmAsk, dirName, joinPath, openFiles, saveFile } from "../lib/platform";
  import Icon from "../lib/ui/Icon.svelte";

  onMount(() => app.refreshPresets());

  const list = $derived(app.presets?.presets ?? []);
  let excluded = $state<Set<string>>(new Set());
  const chosen = $derived(list.filter((p) => !excluded.has(p.name)));

  function toggle(name: string) {
    const s = new Set(excluded);
    if (s.has(name)) s.delete(name);
    else s.add(name);
    excluded = s;
  }

  let renaming = $state<string | null>(null);
  let newName = $state("");

  async function commitRename(name: string) {
    const target = newName.trim();
    renaming = null;
    if (!target || target === name) return;
    try {
      await api.presetRename(name, target);
      await app.refreshPresets();
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  async function remove(name: string) {
    if (!(await confirmAsk(`Remove “${name}” from your preset library? This can't be undone.`, "Remove preset", "Remove"))) return;
    try {
      await api.presetRemove(name);
      await app.refreshPresets();
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  async function exportOne(name: string) {
    const safe = name.replace(/[^A-Za-z0-9 _-]/g, "").replace(/\s+/g, "_");
    const path = await saveFile(joinPath(app.lastDir, `${safe}.json`), [{ name: "LUT Matrix preset", extensions: ["json"] }]);
    if (!path) return;
    try {
      await api.presetExport(name, path);
      app.notify(`Exported ${baseName(path)}`);
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  async function importFiles() {
    const paths = await openFiles([{ name: "Preset or fit", extensions: ["json"] }]);
    if (!paths.length) return;
    try {
      const names = await api.presetImport(paths, false);
      app.notify(`Imported ${names.join(", ")}`);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("already exists") && (await confirmAsk(`${msg}. Replace existing presets with the imported ones?`, "Import presets", "Replace"))) {
        try {
          const names = await api.presetImport(paths, true);
          app.notify(`Imported ${names.join(", ")}`);
        } catch (e2) {
          app.notify(String(e2), "error");
        }
      } else app.notify(msg, "error");
    }
    await app.refreshPresets();
  }

  async function build() {
    if (!chosen.length) return;
    const path = await saveFile(joinPath(app.lastDir, "OpenDRT_v1.1.0.1_LUTmatch.dctl"), [{ name: "DCTL", extensions: ["dctl"] }]);
    if (!path) return;
    try {
      await api.opendrtBuild(
        chosen.map((p) => p.name),
        path,
      );
      app.lastDir = dirName(path);
      app.notify(`Wrote ${baseName(path)} with ${chosen.length} preset${chosen.length === 1 ? "" : "s"}`);
    } catch (e) {
      app.notify(String(e), "error");
    }
  }
</script>

<div class="page">
  <div class="top">
    <p class="muted">
      Your OpenDRT presets, fitted to LUTs (LUT Match) or to colour charts (Chart). They are stored only on this computer and are not part of the app.
      Build writes OpenDRT with the ticked presets added to its Look Preset menu, in this order,
      and a P3-D65 2.6 power option (“DCI - 2.6 Power / P3-D65”) added to its Display Encoding Preset menu.
    </p>
    <div class="actions">
      <button class="btn" onclick={importFiles}><Icon name="plus" size={14} /> Import…</button>
      <button class="btn primary" onclick={build} disabled={!chosen.length}><Icon name="export" size={14} /> Build OpenDRT DCTL…</button>
    </div>
  </div>

  {#if !list.length}
    <div class="empty card">
      <Icon name="library" size={32} />
      <h2>No presets yet</h2>
      <p class="muted">Match OpenDRT to a LUT, then save the result here.</p>
      <button class="btn" onclick={() => (app.view = "opendrt")}>Go to OpenDRT Match</button>
    </div>
  {:else}
    <div class="group">
      {#each list as p (p.name)}
        <div class="row prow">
          <input type="checkbox" checked={!excluded.has(p.name)} onchange={() => toggle(p.name)} aria-label="Include {p.name}" />
          <div class="idx faint num">{excluded.has(p.name) ? "–" : chosen.indexOf(p) + 9}</div>
          <div class="main">
            {#if renaming === p.name}
              <!-- svelte-ignore a11y_autofocus -->
              <input
                type="text"
                bind:value={newName}
                autofocus
                onkeydown={(e) => {
                  if (e.key === "Enter") commitRename(p.name);
                  if (e.key === "Escape") renaming = null;
                }}
                onblur={() => commitRename(p.name)}
              />
            {:else}
              <div class="name">{p.name}</div>
            {/if}
            <div class="sub" title={p.source_lut}>{p.source_lut || "—"} · saved {p.created}</div>
          </div>
          <div class="nums num">
            {#if p.error}
              <div><span class="faint">{p.stock_error != null ? fmt(p.stock_error) + " →" : ""}</span> <b>{fmt(p.error[1])}</b> <span class="faint">{p.error[0]}</span></div>
            {/if}
            <div class="sub">grey {fmt(p.grey_nits)} nits</div>
          </div>
          {#if p.notes.length}<span class="flag" title={p.notes.join("\n")}><Icon name="info" /></span>{/if}
          <div class="rowactions">
            <button class="btn plain" title="Rename" onclick={() => ((renaming = p.name), (newName = p.name))}><Icon name="pencil" size={15} /></button>
            <button class="btn plain" title="Export to a file" onclick={() => exportOne(p.name)}><Icon name="export" size={15} /></button>
            <button class="btn plain danger" title="Remove" onclick={() => remove(p.name)}><Icon name="trash" size={15} /></button>
          </div>
        </div>
      {/each}
    </div>
    <p class="faint small">Numbers on the left are the presets' Look Preset menu positions in the built DCTL (OpenDRT's own looks are 0–8).</p>
  {/if}

  {#if app.presets?.problems.length}
    <div class="card problems">
      {#each app.presets.problems as pr}<div class="sub"><Icon name="warn" size={13} /> {pr}</div>{/each}
    </div>
  {/if}

  {#if app.presets}
    <p class="faint small">Library folder: <span class="selectable mono">{app.presets.dir}</span></p>
  {/if}
</div>

<style>
  .page {
    max-width: 900px;
    padding: 20px 24px 40px;
  }
  .top {
    display: flex;
    gap: 20px;
    align-items: flex-start;
    justify-content: space-between;
    margin-bottom: 18px;
  }
  .top p {
    margin: 0;
    max-width: 520px;
  }
  .actions {
    display: flex;
    gap: 8px;
    flex: none;
  }
  .prow {
    min-height: 56px;
  }
  .idx {
    width: 18px;
    text-align: right;
    font-size: 11px;
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .name {
    font-weight: 500;
  }
  .main .sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .nums {
    text-align: right;
    font-size: 12px;
  }
  .flag {
    color: var(--amber);
    display: flex;
  }
  .rowactions {
    display: flex;
    gap: 0;
  }
  .rowactions .btn {
    color: var(--label-2);
  }
  .rowactions .btn.danger:hover {
    color: var(--red);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px;
    color: var(--label-3);
    text-align: center;
  }
  .empty h2 {
    color: var(--label);
    font-size: 16px;
  }
  .empty p {
    margin: 0 0 6px;
  }
  .small {
    font-size: 11.5px;
    margin: 10px 0 0 12px;
  }
  .problems {
    margin-top: 14px;
    color: var(--amber);
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    width: 15px;
    height: 15px;
    margin: 0;
  }
</style>
