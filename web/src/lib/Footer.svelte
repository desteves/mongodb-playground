<script lang="ts">
  import { registerUrl } from './site.svelte';

  let { slim = false }: { slim?: boolean } = $props();
</script>

<footer class:slim>
  <div class="cta">
    <div>
      <strong>Create your own free Atlas cluster</strong>
      {#if !slim}<p class="muted">Free forever, 512 MB, no credit card. Keep your data, add search, and point this playground at it.</p>{/if}
    </div>
    <a class="btn btn-primary" href={registerUrl('footer')} target="_blank" rel="noopener">Start free ↗</a>
  </div>
  <nav aria-label="Footer">
    <a href="/">Playground</a>
    <a href="/docs/quickstart">Quickstart</a>
    <a href="/docs/api">REST API</a>
    <a href="/docs/mcp">MCP</a>
    <a href="/docs/run-it-yourself">Run it yourself</a>
    <a href="/about">About</a>
    <a href="/llms.txt" target="_blank" rel="noopener">llms.txt</a>
  </nav>
  {#if !slim}
    <p class="fine">
      MongoDB Playground runs your commands on a MongoDB Atlas cluster in throwaway databases. Don't paste secrets or personal
      data: shared links are public. MongoDB and Atlas are trademarks of MongoDB, Inc.
    </p>
  {/if}
</footer>

<style>
  footer {
    border-top: 1px solid var(--border);
    background: var(--surface);
    padding: 1.2rem 1.5rem;
  }
  footer.slim {
    padding: 0.45rem 0.9rem;
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
    justify-content: space-between;
  }
  .cta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .slim .cta {
    gap: 0.75rem;
    font-size: 0.88rem;
    justify-content: flex-start;
  }
  .slim .cta a.btn {
    padding: 0.2rem 0.8rem;
    font-size: 0.85rem;
  }
  .cta p {
    margin: 0.2rem 0 0;
    font-size: 0.9rem;
  }
  nav {
    display: flex;
    gap: 1.1rem;
    flex-wrap: wrap;
    margin-top: 0.9rem;
    font-size: 0.88rem;
  }
  .slim nav {
    margin: 0;
    font-size: 0.8rem;
  }
  nav a {
    color: var(--muted);
    text-decoration: none;
  }
  nav a:hover {
    color: var(--fg);
    text-decoration: underline;
  }
  .fine {
    margin: 0.9rem 0 0;
    font-size: 0.78rem;
    color: var(--muted);
    max-width: 46rem;
  }
  @media (max-width: 1240px) {
    footer.slim nav {
      display: none;
    }
  }
</style>
