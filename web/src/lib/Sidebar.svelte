<script lang="ts">
  import { route, isPlayground } from './ui.svelte';
  import { registerUrl } from './site.svelte';

  const active = (path: string) => (path === '/' ? isPlayground(route.path) : route.path === path);

  // Small inline icons (24x24 stroke paths) so the nav needs no icon font.
  const icons = {
    play: 'M8 5v14l11-7z',
    start: 'M13 2L4 14h7l-1 8 9-12h-7z',
    api: 'M8 9l-4 3 4 3M16 9l4 3-4 3M13 6l-2 12',
    mcp: 'M12 3v4M12 17v4M3 12h4M17 12h4M6.5 6.5l2.8 2.8M14.7 14.7l2.8 2.8M17.5 6.5l-2.8 2.8M9.3 14.7l-2.8 2.8',
    run: 'M4 17l6-6-6-6M12 19h8',
    about: 'M12 8h.01M11 12h1v5h1M12 21a9 9 0 100-18 9 9 0 000 18z',
    leaf: 'M12 3c-1 4-6 7-6 12 0 3 3 6 6 6s6-3 6-6c0-5-5-8-6-12zM12 8v13',
    file: 'M7 3h7l5 5v13H7zM14 3v5h5',
    spec: 'M5 4h14v16H5zM9 9h6M9 13h6',
  };
</script>

{#snippet icon(d: string)}
  <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path {d} /></svg>
{/snippet}

<nav aria-label="Site">
  <ul>
    <li><a href="/" class:on={active('/')} aria-current={active('/') ? 'page' : undefined}>{@render icon(icons.play)}Playground</a></li>
  </ul>

  <h2>Docs</h2>
  <ul>
    <li><a href="/docs/quickstart" class:on={active('/docs/quickstart')} aria-current={active('/docs/quickstart') ? 'page' : undefined}>{@render icon(icons.start)}Quickstart</a></li>
    <li><a href="/docs/api" class:on={active('/docs/api')} aria-current={active('/docs/api') ? 'page' : undefined}>{@render icon(icons.api)}REST API</a></li>
    <li><a href="/docs/mcp" class:on={active('/docs/mcp')} aria-current={active('/docs/mcp') ? 'page' : undefined}>{@render icon(icons.mcp)}MCP server</a></li>
    <li><a href="/docs/run-it-yourself" class:on={active('/docs/run-it-yourself')} aria-current={active('/docs/run-it-yourself') ? 'page' : undefined}>{@render icon(icons.run)}Run it yourself</a></li>
  </ul>

  <h2>Resources</h2>
  <ul>
    <li><a href={registerUrl()} target="_blank" rel="noopener">{@render icon(icons.leaf)}Free Atlas cluster <span class="ext">↗</span></a></li>
    <li><a href="/api/v1/openapi.json" target="_blank" rel="noopener">{@render icon(icons.spec)}OpenAPI spec <span class="ext">↗</span></a></li>
    <li><a href="/llms.txt" target="_blank" rel="noopener">{@render icon(icons.file)}llms.txt <span class="ext">↗</span></a></li>
  </ul>

  <h2>Project</h2>
  <ul>
    <li><a href="/about" class:on={active('/about')} aria-current={active('/about') ? 'page' : undefined}>{@render icon(icons.about)}About</a></li>
  </ul>
</nav>

<style>
  nav {
    padding: 0.75rem 0.6rem 1.5rem;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  h2 {
    margin: 1.1rem 0.6rem 0.3rem;
    font: 700 0.7rem var(--font-body);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted);
  }
  a {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    padding: 0.5rem 0.65rem;
    border-radius: 8px;
    color: var(--fg);
    text-decoration: none;
    font-size: 0.93rem;
  }
  a:hover {
    background: var(--surface-2);
  }
  a.on {
    background: var(--surface-2);
    font-weight: 700;
    box-shadow: inset 3px 0 0 var(--brand);
  }
  svg {
    flex: none;
    color: var(--accent);
  }
  .ext {
    margin-left: auto;
    color: var(--muted);
    font-size: 0.8rem;
  }
</style>
