<script lang="ts">
  import { api, type BatchItem, type PairSummary, type Stage, type StockNames } from "../lib/api";
  import { app, cameraName, fmt, gamutLabel, transferLabel } from "../lib/state.svelte";
  import { confirmAsk, LUT_FILTER, openFile, openFiles, saveFile } from "../lib/platform";
  import Icon from "../lib/ui/Icon.svelte";
  import Toggle from "../lib/ui/Toggle.svelte";

  const b = $derived(app.batch);
  const negatives = $derived(b?.items.filter((i) => i.stage === "negative") ?? []);
  const prints = $derived(b?.items.filter((i) => i.stage === "print") ?? []);
  const fulls = $derived(b?.items.filter((i) => i.stage === "full") ?? []);
  const nNeg = $derived(negatives.filter((i) => i.include).length);
  const nPrint = $derived(prints.filter((i) => i.include).length);
  const nFull = $derived(fulls.filter((i) => i.include).length);
  const pairs = $derived(nNeg * nPrint);
  /** Pairs plus full LUTs: what a run matches. */
  const jobs = $derived(pairs + nFull);
  const goLabel = $derived(
    nFull && pairs ? `Match all ${jobs}` : nFull ? `Match ${nFull} LUT${nFull === 1 ? "" : "s"}` : `Match ${pairs} pair${pairs === 1 ? "" : "s"}`,
  );
  const stageWords: Record<Stage, string> = { negative: "negative", print: "print", full: "full LUT" };
  const running = $derived(!!app.batchProgress);

  /* ---------- the lists ---------- */

  async function update(u: Parameters<typeof api.batchUpdate>[0]) {
    try {
      app.batch = await api.batchUpdate(u);
    } catch (e) {
      app.notify(String(e), "error");
    }
  }
  async function add(stage: Stage) {
    const paths = await openFiles([LUT_FILTER]);
    if (paths.length) await app.addToBatch(paths, stage);
  }
  async function remove(ids: number[]) {
    app.batch = await api.batchRemove(ids);
  }

  let editing = $state<number | null>(null);
  let editText = $state("");
  function startEdit(i: BatchItem) {
    editing = i.id;
    editText = i.name;
  }
  async function finishEdit(i: BatchItem, keep: boolean) {
    if (editing !== i.id) return;
    editing = null;
    if (keep && editText.trim() !== i.name) await update([{ id: i.id, name: editText }]);
  }
  function focus(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  // Quick ticks for prints by variant ("Native", "D60" …) and the neutralized flag.
  const variantWords = $derived([...new Set(prints.map((p) => p.variant_word).filter((w): w is string => !!w))]);
  const hasFlags = $derived(prints.some((p) => p.flags.length > 0));
  function tickPrints(test: (p: BatchItem) => boolean) {
    update(prints.map((p) => ({ id: p.id, include: test(p) })));
  }

  function spaceLabel(i: BatchItem) {
    const out = `${gamutLabel(i.output.gamut, app.catalog)} · ${transferLabel(i.output)}`;
    if (i.stage === "negative") return cameraName(i.input, app.catalog);
    if (i.stage === "full") return `${cameraName(i.input, app.catalog)} → ${out}`;
    return out;
  }

  /* ---------- stock names ---------- */

  let showNames = $state(false);
  type Row = [string, string];
  type TableKey = "looks" | "variants" | "flags" | "negatives";
  let names = $state<Record<TableKey, Row[]>>({ looks: [], variants: [], flags: [], negatives: [] });
  const tables: { key: TableKey; title: string; sub: string; ph: Row }[] = [
    { key: "looks", title: "Looks", sub: "Look code at the start of a print's file name → stock name", ph: ["Look4", "2383v1"] },
    { key: "variants", title: "Variants", sub: "Code after the look's dash → word", ph: ["N", "D60"] },
    { key: "flags", title: "Flags", sub: "Code after the variant → word", ph: ["gn", "Neutralized"] },
    { key: "negatives", title: "Negatives", sub: "Start of a negative's file name → name (when the stock number isn't in it)", ph: ["MyNeg", "5219custom"] },
  ];
  function toRows(s: StockNames) {
    return {
      looks: Object.entries(s.looks) as Row[],
      variants: Object.entries(s.variants) as Row[],
      flags: Object.entries(s.flags) as Row[],
      negatives: Object.entries(s.negatives ?? {}) as Row[],
    };
  }
  async function openNames() {
    showNames = !showNames;
    if (showNames) names = toRows(await api.stockNamesGet());
  }
  async function saveNames() {
    const obj = (rows: Row[]) => Object.fromEntries(rows.filter(([k, v]) => k.trim() && v.trim()).map(([k, v]) => [k.trim(), v.trim()]));
    try {
      app.batch = await api.stockNamesSet({
        looks: obj(names.looks),
        variants: obj(names.variants),
        flags: obj(names.flags),
        negatives: obj(names.negatives),
      });
      app.notify("Stock names saved; LUT names updated");
    } catch (e) {
      app.notify(String(e), "error");
    }
  }
  async function importNames() {
    const p = await openFile([{ name: "Stock names", extensions: ["json"] }]);
    if (!p) return;
    try {
      names = toRows(await api.stockNamesImport(p));
      await saveNames();
    } catch (e) {
      app.notify(String(e), "error");
    }
  }

  /* ---------- running ---------- */

  const effort = $derived(app.match.max_iterations <= 60 ? "Quick" : app.match.max_iterations >= 250 ? "Thorough" : "Standard");
  const perPair = $derived(effort === "Quick" ? 2.5 : effort === "Thorough" ? 12 : 6);
  const neutral = $derived(
    ({ off: "off", middle_gray: "middle gray", exposure: "every level", white_gain: "white gain" } as Record<string, string>)[
      app.match.neutral_gray
    ] ?? "off",
  );
  function duration(s: number) {
    if (s < 90) return `${Math.max(1, Math.round(s))} s`;
    if (s < 5400) return `${Math.round(s / 60)} min`;
    return `${(s / 3600).toFixed(1)} h`;
  }

  let now = $state(Date.now());
  $effect(() => {
    if (!running) return;
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });
  const remaining = $derived.by(() => {
    const p = app.batchProgress;
    if (!p || p.done === 0) return null;
    return ((now - p.started) / 1000 / p.done) * (p.total - p.done);
  });

  async function run(checkOnly: boolean) {
    if (running) return;
    app.batchProgress = { done: 0, total: jobs, started: Date.now(), last: "" };
    if (app.batch) app.batch.results = [];
    selected = new Set();
    try {
      app.batch = await api.batchRun({
        settings: $state.snapshot(app.match),
        check_only: checkOnly,
        fit_mismatched: app.batchFitMismatched,
      });
      if (!checkOnly) app.notify(`Batch finished: ${app.batch.results.filter((r) => r.fitted !== null).length} fitted`);
    } catch (e) {
      app.notify(String(e), "error");
      app.batch = await api.batchState();
    } finally {
      app.batchProgress = null;
    }
  }

  /* ---------- results ---------- */

  type Key = "name" | "fitted" | "gray" | "stock" | "grey_nits";
  let sortKey = $state<Key>("fitted");
  let sortUp = $state(true);
  function sortBy(k: Key) {
    if (sortKey === k) sortUp = !sortUp;
    else {
      sortKey = k;
      sortUp = true;
    }
  }
  const results = $derived.by(() => {
    const r = [...(b?.results ?? [])];
    const v = (x: PairSummary) => (sortKey === "name" ? 0 : (x[sortKey] ?? Infinity));
    r.sort((a, c) => {
      const d = sortKey === "name" ? a.name.localeCompare(c.name) : v(a) - v(c);
      return sortUp ? d : -d;
    });
    return r;
  });
  const fittedCount = $derived(results.filter((r) => r.fitted !== null).length);
  const scale = $derived(Math.max(1, ...results.map((r) => r.stock ?? r.fitted ?? 0)));
  const checkOnlyResults = $derived(results.length > 0 && fittedCount === 0 && results.every((r) => !r.error));

  let selected = $state(new Set<string>());
  let open = $state<string | null>(null);
  function toggle(name: string) {
    const s = new Set(selected);
    if (s.has(name)) s.delete(name);
    else s.add(name);
    selected = s;
  }
  function selectAll(on: boolean) {
    selected = on ? new Set(results.filter((r) => r.fitted !== null).map((r) => r.name)) : new Set();
  }

  async function saveSelected() {
    const list = [...selected];
    if (!list.length) return;
    try {
      const existing = await api.batchExisting(list);
      let replace = false;
      if (existing.length) {
        replace = await confirmAsk(
          `${existing.length} of these are already in your library (${existing.slice(0, 3).join(", ")}${existing.length > 3 ? "…" : ""}). Replace them?`,
          "Replace presets",
          "Replace",
        );
        if (!replace) return;
      }
      const saved = await api.batchSave(list, replace);
      app.notify(`Saved ${saved.length} preset(s) to your library`);
      app.refreshPresets();
    } catch (e) {
      app.notify(String(e), "error");
    }
  }
  async function exportCsv() {
    const p = await saveFile("batch.csv", [{ name: "CSV", extensions: ["csv"] }]);
    if (!p) return;
    try {
      await api.batchExportCsv(p);
      app.notify("CSV written");
    } catch (e) {
      app.notify(String(e), "error");
    }
  }
</script>

<div class="page">
  <p class="intro muted">
    Add negative LUTs (camera log → film negative) and print LUTs (film negative → display). Every ticked negative is stacked
    with every ticked print, as two LUT nodes in Resolve, and OpenDRT is matched to each pair. Pairs are named
    <span class="mono">negative_print</span> from their film stocks. Full LUTs (camera log → display) are matched on their own.
    Each LUT is measured and sorted into its list; names that would repeat get a, b, c….
  </p>

  <div class="cols">
    {#each [{ stage: "negative" as Stage, title: "Negatives", list: negatives, n: nNeg }, { stage: "print" as Stage, title: "Prints", list: prints, n: nPrint }, { stage: "full" as Stage, title: "Full LUTs", list: fulls, n: nFull }] as col (col.stage)}
      <div class="col">
        <div class="col-head">
          <div class="group-title">{col.title} <span class="faint">{col.n}/{col.list.length} ticked</span></div>
          <button class="btn" onclick={() => add(col.stage)} disabled={running}><Icon name="plus" size={13} /> Add…</button>
        </div>
        {#if col.stage === "print" && prints.length > 1}
          <div class="chips">
            <span class="faint">Tick:</span>
            <button class="chip" onclick={() => tickPrints(() => true)}>All</button>
            <button class="chip" onclick={() => tickPrints(() => false)}>None</button>
            {#each variantWords as w (w)}
              <button class="chip" onclick={() => tickPrints((p) => p.variant_word === w)}>{w}</button>
            {/each}
            {#if hasFlags}
              <button class="chip" onclick={() => tickPrints((p) => p.flags.length > 0)}>Neutralized</button>
              <button class="chip" onclick={() => tickPrints((p) => p.flags.length === 0)}>Not neutralized</button>
            {/if}
          </div>
        {/if}
        <div class="group list">
          {#each col.list as i (i.id)}
            <div class="row item" class:off={!i.include}>
              <input type="checkbox" checked={i.include} onchange={(e) => update([{ id: i.id, include: e.currentTarget.checked }])} aria-label="Include {i.name}" disabled={running} />
              <div class="label">
                {#if editing === i.id}
                  <input
                    class="name-edit"
                    type="text"
                    bind:value={editText}
                    use:focus
                    onkeydown={(e) => {
                      if (e.key === "Enter") finishEdit(i, true);
                      if (e.key === "Escape") finishEdit(i, false);
                    }}
                    onblur={() => finishEdit(i, true)}
                    aria-label="Name"
                  />
                {:else}
                  <button class="name" onclick={() => startEdit(i)} title="Rename (empty = automatic name)" disabled={running}>
                    {i.name}{#if i.name_edited}<span class="faint"> ✎</span>{/if}
                  </button>
                {/if}
                <div class="sub" title={i.path}>{i.file} · {spaceLabel(i)}</div>
                <div class="sub measured" class:bad={i.measured.stage !== i.stage} title={i.measured.reason}>
                  {#if i.measured.stage !== i.stage}<Icon name="warn" size={11} />{/if}
                  <span class="ell"
                    >{i.measured.stage !== i.stage ? `Measures as a ${stageWords[i.measured.stage]}: ` : "Measured: "}{i.measured.reason}</span
                  >
                </div>
              </div>
              <div class="acts">
                <select
                  class="stage-pick"
                  value={i.stage}
                  onchange={(e) => update([{ id: i.id, stage: e.currentTarget.value as Stage }])}
                  title="Which list this LUT is in"
                  aria-label="Stage of {i.name}"
                  disabled={running}
                >
                  <option value="negative">Negative</option>
                  <option value="print">Print</option>
                  <option value="full">Full</option>
                </select>
                <button class="icon-btn" title="Remove" onclick={() => remove([i.id])} disabled={running}><Icon name="x" size={14} /></button>
              </div>
            </div>
          {:else}
            <div class="row empty faint">Drop LUTs anywhere in this window (each is measured and sorted), or use Add…</div>
          {/each}
        </div>
      </div>
    {/each}
  </div>

  <div class="group-title">Stock names</div>
  <div class="group">
    <div class="row">
      <div class="label" style="flex: 1 1 auto; min-width: 0">
        Print names from your stock-names table
        <div class="sub">
          e.g. Look4-N_gn_V2 → 2383v1_D60_Neutralized_V2 (a white move D55toD58 adds _D58). Negatives are named from the stock number and
          pull/push in their file names, or FilmLog; full LogC33_/LogC34_ LUTs become FilmLog_ + the print name.
        </div>
      </div>
      <div class="value"><button class="btn" onclick={openNames}>{showNames ? "Close" : "Edit…"}</button></div>
    </div>
    {#if showNames}
      <div class="names fade-in">
        {#each tables as t (t.key)}
          <div class="ntable">
            <div class="strong">{t.title}</div>
            <div class="sub faint">{t.sub}</div>
            {#each names[t.key] as row, k (k)}
              <div class="nrow">
                <input type="text" bind:value={row[0]} placeholder={t.ph[0]} aria-label="{t.title} code" />
                <span class="faint">→</span>
                <input type="text" bind:value={row[1]} placeholder={t.ph[1]} aria-label="{t.title} name" />
                <button class="icon-btn" title="Remove" onclick={() => (names[t.key] = names[t.key].filter((_, j) => j !== k))}><Icon name="x" size={13} /></button>
              </div>
            {/each}
            <button class="btn plain" onclick={() => (names[t.key] = [...names[t.key], ["", ""]])}><Icon name="plus" size={12} /> Add</button>
          </div>
        {/each}
        <div class="nfoot">
          <span class="faint small">{b?.names_path ?? ""}</span>
          <span class="grow"></span>
          <button class="btn" onclick={importNames}>Import…</button>
          <button class="btn primary" onclick={saveNames}>Save</button>
        </div>
      </div>
    {/if}
  </div>

  <div class="group-title">Run</div>
  <div class="group">
    <div class="row">
      <div class="label">
        Matching settings
        <div class="sub">From OpenDRT Match: neutralize gray {neutral}{app.match.neutral_gray !== "off" ? ` ${Math.round(app.match.neutral_amount * 100)}%` : ""} · effort {effort.toLowerCase()} · smoothing {app.match.regularization === 0 ? "off" : app.match.regularization}</div>
      </div>
      <div class="value"><button class="btn plain" onclick={() => (app.view = "opendrt")}>Change…</button></div>
    </div>
    <div class="row">
      <div class="label">Fit mismatched pairs<div class="sub">Pairs whose 18% gray lands far off, or that clip between the stages, are skipped unless this is on</div></div>
      <div class="value"><Toggle bind:checked={app.batchFitMismatched} label="Fit mismatched pairs" /></div>
    </div>
    <div class="row go">
      {#if app.batchProgress}
        {@const p = app.batchProgress}
        <div class="progress">
          <div class="track"><div class="fill" style="width:{p.total ? (100 * p.done) / p.total : 0}%"></div></div>
          <div class="sub num">
            {`${p.done} of ${p.total}`}{p.last ? ` · last: ${p.last}` : ""}{remaining !== null ? ` · about ${duration(remaining)} left` : ""}
          </div>
        </div>
        <button class="btn" onclick={() => api.batchCancel()}><Icon name="stop" size={13} /> Stop</button>
      {:else}
        <span class="muted"
          >{pairs} pair{pairs === 1 ? "" : "s"}{nFull ? ` + ${nFull} full LUT${nFull === 1 ? "" : "s"}` : ""}{jobs
            ? ` · about ${duration(jobs * perPair)}`
            : ""}</span
        >
        <span class="grow"></span>
        <button class="btn" onclick={() => run(true)} disabled={!jobs}>Check</button>
        <button class="btn primary large" onclick={() => run(false)} disabled={!jobs}
          ><Icon name="play" size={14} /> {goLabel}</button
        >
      {/if}
    </div>
  </div>

  {#if results.length}
    <div class="res-head">
      <div class="group-title">
        Results — {results.length}{fittedCount ? `, ${fittedCount} fitted` : ""}
        {#if checkOnlyResults}<span class="faint">(check only)</span>{/if}
      </div>
      {#if fittedCount}
        <div class="res-tools">
          <button class="btn plain" onclick={() => selectAll(selected.size === 0)}>{selected.size ? "Select none" : "Select all fitted"}</button>
          <button class="btn" onclick={exportCsv}>Export CSV…</button>
          <button class="btn primary" onclick={saveSelected} disabled={!selected.size}>Save {selected.size || ""} to Library</button>
        </div>
      {/if}
    </div>
    <div class="table">
      <div class="tr th">
        <span></span>
        <button onclick={() => sortBy("name")}>Name{sortKey === "name" ? (sortUp ? " ↑" : " ↓") : ""}</button>
        <button onclick={() => sortBy("fitted")}>Fitted ΔE{sortKey === "fitted" ? (sortUp ? " ↑" : " ↓") : ""}</button>
        <button onclick={() => sortBy("gray")}>Gray axis{sortKey === "gray" ? (sortUp ? " ↑" : " ↓") : ""}</button>
        <button onclick={() => sortBy("stock")}>Stock{sortKey === "stock" ? (sortUp ? " ↑" : " ↓") : ""}</button>
        <button onclick={() => sortBy("grey_nits")}>Grey{sortKey === "grey_nits" ? (sortUp ? " ↑" : " ↓") : ""}</button>
        <span>Checks</span>
      </div>
      {#each results as r (r.name)}
        <div class="tr" class:open={open === r.name}>
          <span>
            {#if r.fitted !== null}
              <input type="checkbox" checked={selected.has(r.name)} onchange={() => toggle(r.name)} aria-label="Select {r.name}" />
            {/if}
          </span>
          <button class="pair" onclick={() => (open = open === r.name ? null : r.name)}>
            <Icon name="chevron" size={11} />
            <span class="mono">{r.name}</span>{#if r.full}<span class="tag">full</span>{/if}
          </button>
          <span class="num">
            {#if r.fitted !== null}
              <span class="bar"><span class="bfill" style="width:{(100 * r.fitted) / scale}%"></span></span>
              <b>{fmt(r.fitted)}</b>
            {:else}<span class="faint">—</span>{/if}
          </span>
          <span class="num">{r.gray !== null ? fmt(r.gray) : "—"}</span>
          <span class="num faint">{r.stock !== null ? fmt(r.stock) : "—"}</span>
          <span class="num">{r.grey_nits !== null ? `${fmt(r.grey_nits, 1)}` : `${fmt(r.check.gray_nits, 1)}*`}</span>
          <span class="checks">
            {#if r.error}
              <span class="bad" title={r.error}><Icon name="warn" size={13} /> {r.error.startsWith("skipped") ? "skipped" : "failed"}</span>
            {:else if !r.check.ok}
              <span class="bad" title={r.check.problems.join("\n")}><Icon name="warn" size={13} /> mismatch</span>
            {:else if r.safety_ok === false}
              <span class="bad"><Icon name="warn" size={13} /> safety</span>
            {:else}
              <span class="good"><Icon name="check" size={13} /> ok</span>
            {/if}
          </span>
        </div>
        {#if open === r.name}
          <div class="detail fade-in">
            {#if r.full}
              <div><span class="faint">Full LUT</span> {r.print}, matched on its own</div>
              <div><span class="faint">Check</span> 18% gray → {fmt(r.check.gray_nits, 2)} nits</div>
            {:else}
              <div><span class="faint">Negative</span> {r.negative} · <span class="faint">Print</span> {r.print}</div>
              <div>
                <span class="faint">Pair check</span> 18% gray → {fmt(r.check.gray_nits, 2)} nits; {fmt(r.check.outside_percent, 1)}% of displayable
                colours clipped between the stages
              </div>
            {/if}
            {#each r.check.problems as pr}<div class="bad">{pr}</div>{/each}
            {#if r.error}<div class="bad">{r.error}</div>{/if}
            {#if r.fitted !== null}
              <div>
                <span class="faint">Start</span> {r.start} · <span class="faint">best preset</span> {fmt(r.best_preset ?? NaN)} ·
                <span class="faint">p95</span> {fmt(r.p95 ?? NaN)} · <span class="faint">{r.metric}</span>
              </div>
              {#each r.notes as n}<div class="faint">{n}</div>{/each}
            {/if}
          </div>
        {/if}
      {/each}
    </div>
    <p class="faint small">
      Lower is closer to the stacked LUTs. Grey is the fitted Display Grey Luminance (* = where 18% gray lands, for unfitted pairs).
      {#if b?.folder}Reports are kept in {b.folder}.{/if}
    </p>
  {/if}
</div>

<style>
  .page {
    max-width: 1320px;
    padding: 20px 24px 40px;
  }
  .intro {
    margin: 0 0 18px 2px;
    max-width: 760px;
  }
  .mono {
    font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
  }
  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 18px;
    margin-bottom: 20px;
  }
  @media (max-width: 1200px) {
    .cols {
      grid-template-columns: 1fr 1fr;
    }
  }
  @media (max-width: 900px) {
    .cols {
      grid-template-columns: 1fr;
    }
  }
  .col {
    min-width: 0;
  }
  .measured {
    display: flex;
    gap: 4px;
    align-items: center;
    min-width: 0;
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .stage-pick {
    font-size: 12px;
    min-width: 92px;
  }
  .tag {
    margin-left: 6px;
    font-size: 10.5px;
    padding: 0 5px;
    border-radius: 4px;
    border: 1px solid var(--border, currentColor);
    opacity: 0.75;
  }
  .col-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .col-head .group-title {
    margin: 0;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
    margin: 0 0 8px 2px;
    font-size: 11.5px;
  }
  .chip {
    border: 1px solid var(--separator);
    background: var(--surface-2);
    color: var(--label);
    border-radius: 999px;
    padding: 1px 9px;
    font-size: 11.5px;
    cursor: pointer;
  }
  .chip:hover {
    border-color: var(--accent);
  }
  .list {
    max-height: 340px;
    overflow: auto;
  }
  .item {
    gap: 10px;
  }
  .item.off .name,
  .item.off .sub {
    opacity: 0.45;
  }
  .item .label {
    flex: 1;
    min-width: 0;
  }
  .item .sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    all: unset;
    cursor: text;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: block;
    max-width: 100%;
  }
  .name-edit {
    width: 100%;
  }
  .acts {
    display: flex;
    gap: 2px;
    flex: none;
  }
  .icon-btn {
    all: unset;
    cursor: pointer;
    color: var(--label-3);
    padding: 3px;
    border-radius: 5px;
    display: inline-flex;
  }
  .icon-btn:hover:not(:disabled) {
    color: var(--label);
    background: var(--surface-2);
  }
  .icon-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .empty {
    font-size: 12px;
  }
  .group {
    margin-bottom: 20px;
  }
  .names {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 16px;
    padding: 10px 12px 12px;
  }
  @media (max-width: 900px) {
    .names {
      grid-template-columns: 1fr;
    }
  }
  .ntable {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }
  .nrow {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1.3fr) auto;
    gap: 5px;
    align-items: center;
  }
  .nrow input {
    width: 100%;
    min-width: 0;
  }
  .nfoot {
    grid-column: 1 / -1;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .grow {
    flex: 1;
  }
  .strong {
    font-weight: 500;
  }
  .small {
    font-size: 11.5px;
  }
  .row.go {
    min-height: 54px;
    gap: 10px;
  }
  .progress {
    flex: 1;
    min-width: 0;
  }
  .track,
  .bar {
    height: 8px;
    border-radius: 4px;
    background: var(--surface-2);
    overflow: hidden;
    display: block;
  }
  .fill,
  .bfill {
    height: 100%;
    display: block;
    background: var(--accent);
    border-radius: 4px;
    transition: width 0.4s var(--ease);
  }
  .bar {
    width: 70px;
    height: 6px;
    display: inline-block;
    vertical-align: middle;
    margin-right: 6px;
  }
  .res-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .res-tools {
    display: flex;
    gap: 6px;
    margin-bottom: 6px;
  }
  .table {
    border-radius: 10px;
    background: var(--surface);
    border: 1px solid var(--separator);
    overflow: hidden;
    margin-bottom: 10px;
  }
  .tr {
    display: grid;
    grid-template-columns: 28px minmax(0, 2.6fr) 130px 80px 70px 60px 100px;
    gap: 8px;
    align-items: center;
    padding: 6px 12px;
    border-top: 1px solid var(--separator);
    font-size: 12.5px;
  }
  .tr.th {
    border-top: none;
    font-size: 11px;
    color: var(--label-2, var(--label));
    background: var(--surface-2);
  }
  .tr.th button {
    all: unset;
    cursor: pointer;
    font-weight: 600;
  }
  .pair {
    all: unset;
    cursor: pointer;
    display: flex;
    gap: 6px;
    align-items: center;
    min-width: 0;
  }
  .pair .mono {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tr.open .pair :global(svg) {
    transform: rotate(90deg);
  }
  .detail {
    padding: 8px 12px 10px 48px;
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    background: var(--surface-2);
  }
  .checks span {
    display: inline-flex;
    gap: 4px;
    align-items: center;
  }
  .good {
    color: var(--green);
  }
  .bad {
    color: var(--amber);
  }
</style>
