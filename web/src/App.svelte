<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar from './lib/Sidebar.svelte';
  import Footer from './lib/Footer.svelte';
  import Playground from './pages/Playground.svelte';
  import ApiDocs from './pages/ApiDocs.svelte';
  import McpDocs from './pages/McpDocs.svelte';
  import RunYourself from './pages/RunYourself.svelte';
  import Quickstart from './pages/Quickstart.svelte';
  import About from './pages/About.svelte';
  import { cycleTheme, interceptLinks, isDark, isPlayground, route, ui, watchScreen } from './lib/ui.svelte';
  import { loadSite, registerUrl } from './lib/site.svelte';
  import { PAGES, setPage } from './lib/seo';
  import { isDesktop } from './lib/api';

  onMount(() => {
    loadSite();
    return watchScreen();
  });

  $effect(() => {
    document.documentElement.dataset.theme = isDark() ? 'dark' : 'light';
  });

  // Title, description and canonical URL follow the page.
  $effect(() => {
    const p = isPlayground(route.path) ? '/' : route.path;
    const meta = PAGES[p];
    if (meta) setPage(meta.title, meta.description, route.path.startsWith('/p/') ? route.path : p);
    else setPage('Page not found | MongoDB Playground', 'This page does not exist.', route.path);
  });

  const themeLabel = $derived(ui.theme === 'auto' ? 'Auto' : ui.theme === 'light' ? 'Light' : 'Dark');
</script>

<svelte:window onclick={interceptLinks} onkeydown={(e) => e.key === 'Escape' && (ui.drawer = false)} />

<a class="skip" href="#content">Skip to content</a>

<div class="frame" class:drawer={ui.drawer}>
  <header class="topbar">
    <button class="menu" aria-label="Open the menu" aria-expanded={ui.drawer} onclick={() => (ui.drawer = !ui.drawer)}>
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M4 7h16M4 12h16M4 17h16" /></svg>
    </button>
    <a class="brand" href="/" aria-label="MongoDB Playground home">
      <svg viewBox="0 0 32 32" width="26" height="26" aria-hidden="true"><path d="M16 3c-1 4-7 7-7 14 0 4 3 8 7 9 4-1 7-5 7-9 0-7-6-10-7-14z" fill="#00ed64" /><path d="M16 8v18" stroke="#001e2b" stroke-width="1.6" stroke-linecap="round" /></svg>
      <span>MongoDB <b>Playground</b></span>
    </a>
    <span class="spacer"></span>
    <button class="theme" onclick={cycleTheme} title="Theme: {themeLabel} (click to change)" aria-label="Theme: {themeLabel}. Click to change.">
      {#if isDark()}
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M20 14.5A8 8 0 019.5 4 8 8 0 1020 14.5z" /></svg>
      {:else}
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5L19 19M19 5l-1.5 1.5M6.5 17.5L5 19" /></svg>
      {/if}
      <span class="label">{themeLabel}</span>
    </button>
    {#if !isDesktop}
      <a class="btn btn-primary cta" href={registerUrl('header')} target="_blank" rel="noopener">Free Atlas cluster ↗</a>
    {/if}
  </header>

  <aside class="sidebar" aria-label="Navigation"><Sidebar /></aside>
  <button class="backdrop" aria-label="Close the menu" tabindex="-1" onclick={() => (ui.drawer = false)}></button>

  <main id="content" class="content" class:playground={isPlayground(route.path)}>
    {#if isPlayground(route.path)}
      <Playground />
    {:else if route.path === '/docs/quickstart'}
      <Quickstart />
      <Footer />
    {:else if route.path === '/docs/api'}
      <ApiDocs />
      <Footer />
    {:else if route.path === '/docs/mcp'}
      <McpDocs />
      <Footer />
    {:else if route.path === '/docs/run-it-yourself'}
      <RunYourself />
      <Footer />
    {:else if route.path === '/about'}
      <About />
      <Footer />
    {:else}
      <div class="page">
        <h1>Page not found</h1>
        <p class="lead">That page does not exist. <a href="/">Back to the playground</a>.</p>
      </div>
      <Footer />
    {/if}
  </main>
</div>

<style>
  .skip {
    position: absolute;
    left: -9999px;
    top: 0;
    z-index: 200;
    background: var(--brand);
    color: var(--brand-fg);
    padding: 0.5rem 1rem;
  }
  .skip:focus {
    left: 0;
  }
  .frame {
    display: grid;
    grid-template-columns: 15.5rem minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr);
    height: 100vh;
    height: 100dvh;
  }
  .topbar {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 1rem;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: 0.55rem;
    color: var(--fg);
    text-decoration: none;
    font-size: 1.05rem;
    white-space: nowrap;
  }
  .brand b {
    font-weight: 700;
    color: var(--accent);
  }
  .spacer {
    flex: 1;
  }
  .menu {
    display: none;
    padding: 0.3rem 0.5rem;
  }
  .theme {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.7rem;
  }
  .cta {
    padding: 0.35rem 0.9rem;
    font-size: 0.9rem;
    white-space: nowrap;
  }
  .sidebar {
    grid-row: 2;
    overflow-y: auto;
    background: var(--surface);
    border-right: 1px solid var(--border);
  }
  .backdrop {
    display: none;
  }
  .content {
    grid-row: 2;
    grid-column: 2;
    min-width: 0;
    overflow-y: auto;
  }
  .content.playground {
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  @media (max-width: 860px) {
    .frame {
      grid-template-columns: minmax(0, 1fr);
    }
    .menu {
      display: inline-flex;
    }
    .content {
      grid-column: 1;
    }
    .sidebar {
      position: fixed;
      z-index: 60;
      top: 0;
      bottom: 0;
      left: 0;
      width: min(18rem, 84vw);
      padding-top: 3.4rem;
      transform: translateX(-102%);
      transition: transform 0.18s ease;
      box-shadow: var(--shadow);
    }
    .drawer .sidebar {
      transform: translateX(0);
    }
    .drawer .backdrop {
      display: block;
      position: fixed;
      inset: 0;
      z-index: 50;
      border: none;
      border-radius: 0;
      background: rgb(0 0 0 / 50%);
    }
    .topbar .label {
      display: none;
    }
  }
  @media (max-width: 520px) {
    .cta {
      display: none;
    }
  }
</style>
