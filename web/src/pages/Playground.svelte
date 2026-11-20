<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Editor from '../lib/Editor.svelte';
  import Footer from '../lib/Footer.svelte';
  import {
    ApiError,
    explain,
    exportPlayground,
    getConnection,
    isDesktop,
    loadShared,
    run,
    samplePreview,
    samples as fetchSamples,
    sendEvent,
    setConnection,
    share,
    type SampleInfo,
    type ScriptBody,
    type ScriptOutput,
  } from '../lib/api';
  import { download, toCsv } from '../lib/download';
  import { fmt } from '../lib/shellfmt';
  import { site } from '../lib/site.svelte';
  import { isDark, route, ui } from '../lib/ui.svelte';
  import { transcript } from '../lib/transcript';

  type Layout = 'columns' | 'rows' | 'tabs';
  type Pane = 'data' | 'shell' | 'result';
  type EditorApi = { undo: () => void; redo: () => void; getSelection: () => string };
  const KEYS: Pane[] = ['data', 'shell', 'result'];

  const stored = (k: string) => {
    try {
      return localStorage.getItem(k);
    } catch {
      return null;
    }
  };

  // ---- the starting point: the "single collection" template -----------------------------------------------
  // Same shape as the multi-collection template: db = { name: { documents, indexes, ... } }.
  // (The server's `single` template is the source of truth; this copy is the offline fallback.)
  const INITIAL_CONFIG = `db = {
  collection: {
    documents: [
      { name: "Ada",   age: 36, city: "London" },
      { name: "Linus", age: 55, city: "Helsinki" },
      { name: "Grace", age: 85, city: "New York" }
    ],
    indexes: [
      { key: { age: 1 } }   // up to 3 per collection
    ]
    // Atlas Search, 1 per collection:
    // searchIndexes: [ { definition: { mappings: { dynamic: true } } } ],
    // Vector Search, 1 per collection:
    // vectorSearchIndexes: [ { definition: { fields: [ { type: "vector", path: "embedding", numDimensions: 3, similarity: "cosine" } ] } } ]
  }
}`;
  const INITIAL_SCRIPT = 'db.collection.find({ age: { $gt: 40 } })';

  const DESC = {
    data: 'Input data to test: documents, plus indexes and search indexes.',
    shell: 'Run mongosh commands: CRUD operations, aggregations, Vector Search and more.',
    result: 'Output: what your commands returned, query plans and errors.',
  };

  let config = $state(INITIAL_CONFIG);
  let script = $state(INITIAL_SCRIPT);
  let selection = $state(''); // the highlighted text in the shell
  let results = $state('');
  let output = $state<ScriptOutput | null>(null);
  let status = $state('');
  let error = $state('');
  let notice = $state(''); // a banner that shows whichever panel or tab you are on
  let busy = $state(false);
  let picked = $state('');
  let autorun = $state(false);
  let toast = $state('');
  let connection = $state(''); // desktop app only
  let source = $state<'mine' | 'sample'>('mine');
  let sampleInfo = $state<SampleInfo>({ available: false, databases: [] });
  let sampleDb = $state('sample_mflix');
  let dataEd: EditorApi | undefined = $state();
  let shellEd: EditorApi | undefined = $state();
  let exportOpen = $state(false);
  let moreOpen = $state(false);
  let shareUrl = $state('');
  let shareDialog: HTMLDialogElement;
  let collapsed = $state<Record<Pane, boolean>>({ data: false, shell: false, result: false });

  const pristine = $derived(config === INITIAL_CONFIG && script === INITIAL_SCRIPT);
  const templates = $derived(site.examples.filter((e) => e.kind === 'template'));
  const examples = $derived(site.examples.filter((e) => e.kind === 'example'));
  // Sample data is behind a feature flag (FEATURE_SAMPLE_DATA), off by default.
  const sampleEnabled = $derived(!isDesktop && !!site.limits?.features.sampleData);
  const origin = typeof location === 'undefined' ? '' : location.origin;
  const anyCollapsed = $derived(Object.values(collapsed).some(Boolean));
  const hasSelection = $derived(selection.trim().length > 0);

  // ---- layout ---------------------------------------------------------------------------------------------------
  let layoutPref = $state<Layout | null>(stored('layout') as Layout | null);
  let active = $state<Pane>('shell');
  let sizes = $state([32, 34]); // percent for the data and shell panels; results take the rest
  let panesEl: HTMLDivElement;
  const layout = $derived<Layout>(ui.small ? 'tabs' : (layoutPref ?? (ui.medium ? 'rows' : 'columns')));

  function setLayout(l: Layout) {
    layoutPref = l;
    try {
      localStorage.setItem('layout', l);
    } catch {
      /* ignore */
    }
  }

  /** Size of each panel. A collapsed panel shrinks to a rail (columns) or just its header (rows). */
  function paneStyle(i: number) {
    if (layout === 'tabs') return 'flex: 1 1 0';
    if (collapsed[KEYS[i]]) return layout === 'columns' ? 'flex: 0 0 2.6rem' : 'flex: 0 0 auto';
    if (anyCollapsed) return 'flex: 1 1 0'; // the panels that are left share the space
    return i < 2 ? `flex: 0 0 ${sizes[i]}%` : 'flex: 1 1 0';
  }

  function toggle(k: Pane) {
    collapsed[k] = !collapsed[k];
  }

  function drag(i: number, e: PointerEvent) {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const r = panesEl.getBoundingClientRect();
      const pos = layout === 'columns' ? (ev.clientX - r.left) / r.width : (ev.clientY - r.top) / r.height;
      resize(i, pos * 100 - (i === 1 ? sizes[0] : 0));
    };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
  }
  function resize(i: number, pct: number) {
    const other = i === 0 ? sizes[1] : sizes[0];
    sizes[i] = Math.max(12, Math.min(pct, 88 - other));
  }
  function nudge(i: number, e: KeyboardEvent) {
    const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 2 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -2 : 0;
    if (step) {
      e.preventDefault();
      resize(i, sizes[i] + step);
    }
  }

  // ---- loading ----------------------------------------------------------------------------------------------------
  // /?example=<id> opens an example (the Quickstart links use it).
  const wantedExample = typeof location === 'undefined' ? null : new URLSearchParams(location.search).get('example');
  let appliedDefault = false;

  onMount(() => {
    sendEvent('page_view');
    if (isDesktop) getConnection().then((c) => (connection = c));
    const m = /^\/p\/([0-9A-Za-z-]+)$/.exec(route.path);
    if (m) {
      loadShared(m[1])
        .then((p) => {
          const text = (v: unknown) => (typeof v === 'string' ? v : v == null ? '' : fmt(v));
          config = text(p.collections);
          script = p.script ?? '';
          if (p.datasource) {
            source = 'sample';
            sampleDb = p.datasource.sample;
          }
          appliedDefault = true;
          sendEvent('page_view', { shareId: m[1] });
        })
        .catch((e) => (notice = e instanceof ApiError ? e.message : 'Could not load that shared playground.'));
    }
    const closeMenus = () => {
      exportOpen = false;
    };
    window.addEventListener('click', closeMenus);
    return () => window.removeEventListener('click', closeMenus);
  });

  // Once the examples arrive: open the one asked for, else the server's single-collection template.
  $effect(() => {
    if (appliedDefault || !site.examples.length) return;
    appliedDefault = true;
    if (wantedExample && site.examples.some((e) => e.id === wantedExample)) {
      load(wantedExample);
      return;
    }
    const single = site.examples.find((t) => t.id === 'single');
    if (single && untrack(() => pristine) && !route.path.startsWith('/p/')) {
      config = single.config ?? config;
      script = single.script;
    }
  });

  // Sample data: only when the feature flag is on.
  $effect(() => {
    if (sampleEnabled) {
      fetchSamples().then((s) => {
        sampleInfo = s;
        if (s.available && !s.databases.some((d) => d.name === sampleDb)) sampleDb = s.databases[0].name;
      });
    }
  });

  function load(id: string) {
    const ex = site.examples.find((e) => e.id === id);
    picked = '';
    if (!ex) return;
    config = ex.config ?? '';
    script = ex.script;
    source = ex.datasource && sampleEnabled ? 'sample' : 'mine';
    if (ex.datasource) sampleDb = ex.datasource.sample;
    results = '';
    output = null;
    status = '';
    error = '';
    notice = '';
    moreOpen = false;
    active = 'shell';
    sendEvent('example_open', { exampleId: ex.id });
  }

  /** Back to the single-collection template. Undo (in either editor) brings your work back. */
  function reset() {
    if (site.examples.some((e) => e.id === 'single')) load('single');
    else {
      config = INITIAL_CONFIG;
      script = INITIAL_SCRIPT;
    }
    moreOpen = false;
  }

  const body = (text = script): ScriptBody =>
    source === 'sample' ? { script: text, datasource: { sample: sampleDb } } : { script: text, config };

  function show(out: ScriptOutput, planOnly: boolean, part: string) {
    output = out;
    results = transcript(out);
    const n = out.statements.length;
    status = `${n} statement${n === 1 ? '' : 's'} · ${out.elapsed_ms} ms${part}${planOnly ? ' · plans only' : ''}${out.ok ? '' : ' · stopped at an error'}`;
    error = '';
  }

  /** Run everything in the shell, or only the highlighted statements. */
  async function execute(planOnly = false, scope: 'all' | 'selection' = 'all') {
    if (busy) return;
    if (scope === 'selection' && !hasSelection) {
      notice = 'Highlight one or more statements in the Shell first, or use Run all.';
      return;
    }
    error = '';
    notice = '';
    busy = true;
    moreOpen = false;
    try {
      const text = scope === 'selection' ? selection : script;
      const b = body(text);
      show(await (planOnly ? explain(b) : run(b)), planOnly, scope === 'selection' ? ' · selection' : '');
    } catch (e) {
      results = '';
      output = null;
      status = '';
      error = e instanceof ApiError ? e.message + (e.hint ? `\n${e.hint}` : '') : `Request failed: ${(e as Error).message}`;
      if (layout === 'tabs') notice = error;
    } finally {
      busy = false;
      if (layout === 'tabs') active = 'result';
      collapsed.result = false; // make sure the answer is visible
    }
  }

  /** Ctrl/Cmd+Enter: the highlighted statements if there are any, otherwise everything. */
  const runDefault = () => execute(false, hasSelection ? 'selection' : 'all');

  // Auto-run: re-run shortly after you stop typing.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    script;
    config;
    source;
    sampleDb;
    if (!autorun) return;
    clearTimeout(timer);
    timer = setTimeout(() => untrack(() => execute()), 900);
    return () => clearTimeout(timer);
  });

  function say(msg: string) {
    toast = msg;
    setTimeout(() => (toast = ''), 1800);
  }

  async function copy(text: string, msg = 'Copied') {
    await navigator.clipboard?.writeText(text).catch(() => {});
    say(msg);
  }

  // ---- share: a UUID link, shown in a dialog so it is never "silent" -----------------------------------------------
  async function doShare() {
    notice = '';
    try {
      const { id } = await share(body());
      shareUrl = `${origin}/p/${id}`;
      history.pushState({}, '', `/p/${id}`);
      route.path = `/p/${id}`;
      navigator.clipboard?.writeText(shareUrl).catch(() => {});
      shareDialog.showModal();
      sendEvent('copy_link', { shareId: id });
    } catch (e) {
      notice = e instanceof ApiError ? `Could not create the share link: ${e.message}` : `Could not create the share link: ${(e as Error).message}`;
    }
  }

  // ---- export -------------------------------------------------------------------------------------------------------
  async function serverExport(format: 'mongosh' | 'json') {
    exportOpen = false;
    try {
      const out = await exportPlayground({ script, config }, format);
      download(out.filename, out.content, out.mimeType);
      say(`Saved ${out.filename}`);
    } catch (e) {
      notice = e instanceof ApiError ? `Export failed: ${e.message}` : `Export failed: ${(e as Error).message}`;
    }
  }

  const lastRows = () => output?.statements.filter((s) => !s.error && s.results.length).at(-1)?.results ?? [];

  function exportResults(kind: 'json' | 'csv' | 'txt') {
    exportOpen = false;
    if (!output) return say('Run something first');
    if (kind === 'txt') download('transcript.txt', results, 'text/plain');
    else if (kind === 'csv') download('results.csv', toCsv(lastRows()), 'text/csv');
    else
      download(
        'results.json',
        JSON.stringify(output.statements.map((s) => ({ statement: s.statement, op: s.op, results: s.results, write: s.write, error: s.error })), null, 2),
        'application/json',
      );
  }

  const requestJson = () => JSON.stringify(body(), null, 2);
  const curl = () =>
    `curl -s -X POST ${origin}/api/v1/run \\\n  -H 'content-type: application/json' \\\n  -d '${requestJson().replace(/'/g, `'\\''`)}'`;

  // ---- sample data (feature flag) ------------------------------------------------------------------------------------
  async function preview() {
    notice = '';
    const coll = /db\s*\.\s*([A-Za-z_$][\w$]*)|getCollection\(\s*['"]([^'"]+)/.exec(script);
    const name = coll?.[1] ?? coll?.[2];
    if (!name) {
      notice = 'Write a statement like db.movies.find() first, then preview that collection.';
      return;
    }
    try {
      results = `> // 3 random documents from ${sampleDb}.${name}\n` + fmt(await samplePreview(sampleDb, name));
      status = 'preview';
      if (layout === 'tabs') active = 'result';
    } catch (e) {
      notice = e instanceof ApiError ? e.message : (e as Error).message;
    }
  }

  async function saveConnection() {
    try {
      await setConnection(connection);
      say('Connection saved');
    } catch (e) {
      notice = e instanceof ApiError ? e.message : (e as Error).message;
    }
  }
</script>

{#snippet collapseButton(k: Pane, name: string)}
  {#if layout !== 'tabs'}
    <button class="mini collapse" onclick={() => toggle(k)} aria-expanded={!collapsed[k]} aria-label={collapsed[k] ? `Expand the ${name} panel` : `Collapse the ${name} panel`} title={collapsed[k] ? `Expand ${name}` : `Collapse ${name}`}>
      {collapsed[k] ? (layout === 'columns' ? '▸' : '▾') : layout === 'columns' ? '◂' : '▴'}
    </button>
  {/if}
{/snippet}

<div class="pg">
  <div class="toolbar">
    <div class="actions">
      <button class="run primary" onclick={() => execute()} disabled={busy} aria-label="Run the script" title="Run all statements in the Shell">
        <span aria-hidden="true">▶</span>{busy ? 'Running…' : 'Run'}
      </button>
      {#if !isDesktop}<button onclick={doShare} title="Create a link to this playground">Share</button>{/if}
      <div class="menuwrap">
        <button aria-haspopup="menu" aria-expanded={exportOpen} onclick={(e) => { e.stopPropagation(); exportOpen = !exportOpen; }}>Export ▾</button>
        {#if exportOpen}
          <!-- Plain buttons in a popover: stopping the click keeps it open while you pick. -->
          <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
          <div class="menu" onclick={(e) => e.stopPropagation()}>
            {#if !isDesktop}
              <h3>Playground</h3>
              <button onclick={() => serverExport('mongosh')}>Script for mongosh (.js)</button>
              <button onclick={() => serverExport('json')}>Data &amp; script as JSON</button>
            {/if}
            <h3>Results</h3>
            <button onclick={() => exportResults('json')} disabled={!output}>Results (.json)</button>
            <button onclick={() => exportResults('csv')} disabled={!output}>Last result as CSV</button>
            <button onclick={() => exportResults('txt')} disabled={!output}>Transcript (.txt)</button>
            {#if !isDesktop}
              <h3>API</h3>
              <button onclick={() => { exportOpen = false; copy(curl(), 'curl copied'); }}>Copy as curl</button>
              <button onclick={() => { exportOpen = false; copy(requestJson(), 'Request JSON copied'); }}>Copy request JSON</button>
            {/if}
          </div>
        {/if}
      </div>
    </div>

    <button class="more-btn" aria-expanded={moreOpen} onclick={() => (moreOpen = !moreOpen)}>
      {moreOpen ? '✕' : '⋯'}<span class="sr"> More</span>
    </button>
    <div class="secondary" class:open={moreOpen}>
      <select bind:value={picked} onchange={() => load(picked)} aria-label="Templates and examples">
        <option value="">Templates &amp; examples…</option>
        <optgroup label="Templates">
          {#each templates as t}<option value={t.id}>{t.title}</option>{/each}
        </optgroup>
        <optgroup label="Examples">
          {#each examples as ex}<option value={ex.id}>{ex.title}</option>{/each}
        </optgroup>
      </select>
      <button class="reset" onclick={reset} disabled={pristine} title="Back to the single-collection template (undo brings your work back)">Reset</button>
      {#if sampleEnabled}
        <label>
          Data
          <select bind:value={source} onchange={() => sendEvent('datasource_change', { datasource: source })}>
            <option value="mine">My documents</option>
            <option value="sample" disabled={!sampleInfo.available}>Atlas sample data</option>
          </select>
        </label>
        {#if source === 'sample'}
          <select bind:value={sampleDb} aria-label="Sample database">
            {#each sampleInfo.databases as d}<option value={d.name}>{d.name}</option>{/each}
          </select>
          <button onclick={preview}>Preview docs</button>
        {/if}
      {/if}
      {#if isDesktop}
        <label>MongoDB <input bind:value={connection} size="30" aria-label="MongoDB connection string" /></label>
        <button onclick={saveConnection}>Save</button>
      {/if}
      <label class="check"><input type="checkbox" bind:checked={autorun} /> Auto-run</label>
      {#if !ui.small}
        <span class="seg" role="group" aria-label="Layout">
          <button class:on={layout === 'columns'} onclick={() => setLayout('columns')} title="Side by side" aria-label="Side by side">▥</button>
          <button class:on={layout === 'rows'} onclick={() => setLayout('rows')} title="Stacked" aria-label="Stacked">☰</button>
          <button class:on={layout === 'tabs'} onclick={() => setLayout('tabs')} title="One panel at a time" aria-label="One panel at a time">▭</button>
        </span>
      {/if}
    </div>
  </div>

  {#if notice}
    <div class="notice" role="alert">
      <span>{notice}</span>
      <button class="mini" onclick={() => (notice = '')} aria-label="Dismiss">✕</button>
    </div>
  {/if}

  {#if layout === 'tabs'}
    <nav class="tabbar" aria-label="Panels">
      <button class:on={active === 'data'} onclick={() => (active = 'data')}>Data</button>
      <button class:on={active === 'shell'} onclick={() => (active = 'shell')}>Shell</button>
      <button class:on={active === 'result'} onclick={() => (active = 'result')}>Results{status ? ' ·' : ''}</button>
    </nav>
  {/if}

  <div class="panes {layout}" bind:this={panesEl}>
    <!-- ===== Data ===== -->
    <section class="pane" class:collapsed={collapsed.data && layout !== 'tabs'} style={paneStyle(0)} hidden={layout === 'tabs' && active !== 'data'}>
      {#if collapsed.data && layout !== 'tabs'}
        <button class="rail" onclick={() => toggle('data')} aria-label="Expand the Data panel"><span>Data</span><span aria-hidden="true">{layout === 'columns' ? '▸' : '▾'}</span></button>
      {:else}
        <header class="phead">
          <div class="row">
            <h2>Data</h2>
            <span class="tools">
              {#if source !== 'sample'}
                <button class="mini" onclick={() => dataEd?.undo()} title="Undo" aria-label="Undo in the data panel">↶</button>
                <button class="mini" onclick={() => dataEd?.redo()} title="Redo" aria-label="Redo in the data panel">↷</button>
                <button class="mini" onclick={() => (config = '')} title="Clear the data panel (undo brings it back)" disabled={!config}>Clear</button>
              {/if}
              {@render collapseButton('data', 'Data')}
            </span>
          </div>
          <p class="desc">{source === 'sample' ? `Atlas sample data (${sampleDb}), read-only.` : DESC.data}</p>
        </header>
        {#if source === 'sample'}
          <p class="note">Statements run against <code>{sampleDb}</code>: find, aggregate, countDocuments and distinct only.</p>
        {:else}
          <div class="editor-wrap"><Editor bind:this={dataEd} bind:value={config} dark={isDark()} onrun={runDefault} /></div>
        {/if}
      {/if}
    </section>

    {#if layout !== 'tabs' && !collapsed.data && !collapsed.shell}
      <!-- A focusable separator is a valid ARIA widget: arrow keys resize, the pointer drags. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div class="splitter" role="separator" tabindex="0" aria-valuenow={Math.round(sizes[0])} aria-valuemin="12" aria-valuemax="88" aria-orientation={layout === 'columns' ? 'vertical' : 'horizontal'} aria-label="Resize the data and shell panels" onpointerdown={(e) => drag(0, e)} onkeydown={(e) => nudge(0, e)}></div>
    {/if}

    <!-- ===== Shell ===== -->
    <section class="pane" class:collapsed={collapsed.shell && layout !== 'tabs'} style={paneStyle(1)} hidden={layout === 'tabs' && active !== 'shell'}>
      {#if collapsed.shell && layout !== 'tabs'}
        <button class="rail" onclick={() => toggle('shell')} aria-label="Expand the Shell panel"><span>Shell</span><span aria-hidden="true">{layout === 'columns' ? '▸' : '▾'}</span></button>
      {:else}
        <header class="phead">
          <div class="row">
            <h2>Shell</h2>
            <span class="tools">
              <button class="mini" onclick={() => shellEd?.undo()} title="Undo" aria-label="Undo in the shell">↶</button>
              <button class="mini" onclick={() => shellEd?.redo()} title="Redo" aria-label="Redo in the shell">↷</button>
              <button class="mini" onclick={() => (script = '')} title="Clear the shell (undo brings it back)" disabled={!script}>Clear</button>
              {@render collapseButton('shell', 'Shell')}
            </span>
          </div>
          <p class="desc">{DESC.shell}</p>
          <div class="shell-actions">
            <button class="mini run-all" onclick={() => execute()} disabled={busy} title="Run every statement in the Shell">▶ Run all</button>
            <button class="mini" onclick={() => execute(false, 'selection')} disabled={busy || !hasSelection} title={hasSelection ? 'Run only the highlighted statements' : 'Highlight statements in the Shell to run just those'}>Run selection</button>
            <button class="mini" onclick={() => execute(true, hasSelection ? 'selection' : 'all')} disabled={busy} title="Show the query plan (index use, documents examined) instead of running the statements. Uses the highlighted statements if any.">Explain plan</button>
          </div>
        </header>
        <div class="editor-wrap"><Editor bind:this={shellEd} bind:value={script} dark={isDark()} onrun={runDefault} onselection={(t) => (selection = t)} /></div>
      {/if}
    </section>

    {#if layout !== 'tabs' && !collapsed.shell && !collapsed.result}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div class="splitter" role="separator" tabindex="0" aria-valuenow={Math.round(sizes[1])} aria-valuemin="12" aria-valuemax="88" aria-orientation={layout === 'columns' ? 'vertical' : 'horizontal'} aria-label="Resize the shell and results panels" onpointerdown={(e) => drag(1, e)} onkeydown={(e) => nudge(1, e)}></div>
    {/if}

    <!-- ===== Results ===== -->
    <section class="pane" class:collapsed={collapsed.result && layout !== 'tabs'} style={paneStyle(2)} hidden={layout === 'tabs' && active !== 'result'}>
      {#if collapsed.result && layout !== 'tabs'}
        <button class="rail" onclick={() => toggle('result')} aria-label="Expand the Results panel"><span>Results</span><span aria-hidden="true">{layout === 'columns' ? '▸' : '▾'}</span></button>
      {:else}
        <header class="phead">
          <div class="row">
            <h2>Results <small>{status}</small></h2>
            <span class="tools">
              {#if results}
                <button class="mini" onclick={() => copy(results, 'Results copied')}>Copy</button>
                <button class="mini" onclick={() => { results = ''; output = null; status = ''; }}>Clear</button>
              {/if}
              {@render collapseButton('result', 'Results')}
            </span>
          </div>
          <p class="desc">{DESC.result}</p>
        </header>
        {#if error}<pre class="error" role="alert">{error}</pre>{/if}
        <div class="editor-wrap"><Editor bind:value={results} dark={isDark()} readonly /></div>
      {/if}
    </section>
  </div>

  <Footer slim />
  {#if toast}<p class="toast" role="status">{toast}</p>{/if}
</div>

<dialog bind:this={shareDialog} class="dialog" aria-labelledby="share-title" onclick={(e) => e.target === shareDialog && shareDialog.close()}>
  <div class="dialog-body">
    <h2 id="share-title">Your share link</h2>
    <p class="muted">Anyone with this link can open your data and statements. It works for 90 days after its last view. Don't share secrets or personal data.</p>
    <input class="url" readonly value={shareUrl} aria-label="Share link" onfocus={(e) => e.currentTarget.select()} />
    <div class="dialog-actions">
      <button class="primary" onclick={() => copy(shareUrl, 'Link copied')}>Copy link</button>
      <a class="btn" href={shareUrl} target="_blank" rel="noopener">Open in a new tab</a>
      <button onclick={() => shareDialog.close()}>Close</button>
    </div>
  </div>
</dialog>

<style>
  .pg {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .toolbar {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.9rem;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .actions {
    display: flex;
    gap: 0.45rem;
    align-items: center;
  }
  /* Run is the main action: big, Spring Green, with a glow. */
  .run {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 7.5rem;
    min-height: 2.8rem;
    padding: 0.5rem 1.5rem;
    font-size: 1.1rem;
    justify-content: center;
    box-shadow: 0 2px 14px rgb(0 237 100 / 35%);
  }
  .more-btn {
    display: none;
    margin-left: auto;
  }
  .secondary {
    display: contents;
  }
  .secondary label {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.88rem;
  }
  .secondary > select {
    max-width: 20rem;
  }
  .check input {
    min-height: 0;
  }
  .seg {
    display: inline-flex;
    margin-left: auto;
  }
  .seg button {
    border-radius: 0;
    padding: 0.3rem 0.55rem;
  }
  .seg button:first-child {
    border-radius: 8px 0 0 8px;
  }
  .seg button:last-child {
    border-radius: 0 8px 8px 0;
  }
  .seg button.on,
  .tabbar button.on {
    background: var(--brand);
    color: var(--brand-fg);
    border-color: var(--brand);
  }
  .menuwrap {
    position: relative;
  }
  .menu {
    position: absolute;
    z-index: 30;
    top: calc(100% + 4px);
    left: 0;
    min-width: 15rem;
    display: grid;
    padding: 0.4rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow);
  }
  .menu h3 {
    margin: 0.4rem 0.5rem 0.2rem;
    font: 700 0.68rem var(--font-body);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .menu button {
    text-align: left;
    border: none;
    background: transparent;
    border-radius: 6px;
  }
  .menu button:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.45rem 0.9rem;
    background: var(--err-bg);
    color: var(--err-fg);
    font-size: 0.9rem;
    border-bottom: 1px solid var(--border);
  }
  .tabbar {
    display: flex;
    border-bottom: 1px solid var(--border);
  }
  .tabbar button {
    flex: 1;
    border-radius: 0;
    border: none;
    border-bottom: 3px solid transparent;
    background: var(--bg);
    padding: 0.55rem 0.5rem;
  }
  .tabbar button.on {
    background: var(--bg);
    color: var(--fg);
    border-bottom-color: var(--brand);
    font-weight: 700;
  }
  .panes {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .panes.columns {
    flex-direction: row;
  }
  .panes.rows {
    flex-direction: column;
  }
  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .pane[hidden] {
    display: none;
  }
  .editor-wrap {
    flex: 1;
    min-height: 0;
  }
  .splitter {
    flex: 0 0 6px;
    background: var(--border);
    touch-action: none;
  }
  .splitter:hover,
  .splitter:focus-visible {
    background: var(--brand);
  }
  .columns > .splitter {
    cursor: col-resize;
  }
  .rows > .splitter {
    cursor: row-resize;
  }

  /* Panel headers: title, a one-line description, then actions. */
  .phead {
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    padding: 0.35rem 0.75rem 0.45rem;
  }
  .phead .row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .phead h2 {
    margin: 0;
    font: 700 0.85rem var(--font-body);
    letter-spacing: 0.02em;
  }
  .phead h2 small {
    font-weight: 400;
    color: var(--muted);
    margin-left: 0.4rem;
  }
  .desc {
    margin: 0.1rem 0 0;
    font-size: 0.8rem;
    color: var(--muted);
    line-height: 1.35;
  }
  .tools {
    margin-left: auto;
    display: inline-flex;
    gap: 0.25rem;
  }
  .mini {
    min-height: 0;
    padding: 0.1rem 0.55rem;
    font-size: 0.78rem;
  }
  .shell-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-top: 0.4rem;
  }
  .shell-actions .mini {
    padding: 0.25rem 0.7rem;
    font-size: 0.82rem;
  }
  .run-all {
    background: var(--brand);
    color: var(--brand-fg);
    border-color: var(--brand);
    font-weight: 700;
  }
  .collapse {
    padding: 0.1rem 0.45rem;
  }

  /* A collapsed panel: a thin rail in columns, just its bar in rows. */
  .rail {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    width: 100%;
    height: 100%;
    border: none;
    border-radius: 0;
    background: var(--surface);
    font-weight: 700;
    font-size: 0.82rem;
    padding: 0.4rem;
  }
  .columns .rail {
    flex-direction: column;
    writing-mode: vertical-rl;
    transform: rotate(180deg);
  }
  .rows .rail {
    justify-content: flex-start;
    min-height: 2.2rem;
    padding: 0.4rem 0.9rem;
  }
  .rail:hover {
    background: var(--surface-2);
  }
  .note {
    margin: 0;
    padding: 0.75rem;
    font-size: 0.88rem;
  }
  .error {
    margin: 0;
    padding: 0.5rem 0.75rem;
    white-space: pre-wrap;
    color: var(--err-fg);
    background: var(--err-bg);
    font-size: 0.85rem;
  }

  /* share dialog */
  .dialog {
    width: min(34rem, 94vw);
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--bg);
    color: var(--fg);
  }
  .dialog::backdrop {
    background: rgb(0 0 0 / 50%);
  }
  .dialog-body {
    padding: 1.1rem 1.3rem 1.3rem;
  }
  .dialog-body h2 {
    margin: 0 0 0.4rem;
    font-size: 1.3rem;
  }
  .dialog-body p {
    margin: 0 0 0.8rem;
    font-size: 0.9rem;
  }
  .url {
    width: 100%;
    font-family: var(--font-mono);
    font-size: 0.85rem;
  }
  .dialog-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-top: 0.9rem;
  }
  .dialog-actions a.btn {
    padding: 0.35rem 0.9rem;
  }

  /* Below ~1100px the secondary controls live in a dropdown, so the toolbar stays one clean row. */
  @media (max-width: 1100px) {
    .more-btn {
      display: block;
    }
    .secondary {
      display: none;
    }
    .secondary.open {
      display: grid;
      grid-template-columns: minmax(0, 1fr);
      gap: 0.5rem;
      position: absolute;
      z-index: 20;
      top: 100%;
      right: 0;
      width: min(26rem, 100%);
      padding: 0.75rem;
      background: var(--surface);
      border: 1px solid var(--border);
      border-top: none;
      box-shadow: var(--shadow);
    }
    .secondary.open > *,
    .secondary.open select,
    .secondary.open input:not([type='checkbox']) {
      min-width: 0;
      max-width: 100%;
    }
    .secondary.open select,
    .secondary.open input:not([type='checkbox']) {
      width: 100%;
    }
    .secondary.open label {
      display: grid;
      grid-template-columns: 5.5rem minmax(0, 1fr);
      align-items: center;
    }
    .secondary.open label.check {
      grid-template-columns: auto 1fr;
    }
  }
  @media (max-width: 860px) {
    .toolbar {
      padding: 0.4rem 0.5rem;
      flex-wrap: nowrap;
      gap: 0.4rem;
    }
    .actions {
      gap: 0.3rem;
      flex: 1;
    }
    .actions button {
      padding: 0.3rem 0.5rem;
      font-size: 0.9rem;
    }
    .actions > button,
    .actions .menuwrap {
      flex: 1;
    }
    .actions .menuwrap > button {
      width: 100%;
    }
    .actions .run {
      flex: 2;
      min-width: 0;
      padding: 0.5rem 0.9rem;
      font-size: 1.05rem;
    }
    .more-btn {
      margin-left: 0;
      flex: none;
    }
    .secondary.open {
      left: 0;
      width: auto;
    }
    .menu {
      left: auto;
      right: 0;
    }
  }
</style>
