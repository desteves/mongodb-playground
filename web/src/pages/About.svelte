<script lang="ts">
  import { site, registerUrl } from '../lib/site.svelte';

  const tier = $derived(site.limits?.deployment?.cluster ?? 'M20');
</script>

<div class="page">
  <h1>About MongoDB Playground</h1>
  <p class="lead">
    A free online MongoDB shell: paste documents, run mongosh-style commands, see query plans, try Atlas Search and Vector Search,
    then share or export. Built for people and for AI agents.
  </p>

  <h2>It runs on a MongoDB Atlas {tier} cluster</h2>
  <p>
    Every run executes on a dedicated <strong>MongoDB Atlas {tier} cluster</strong>, reached privately from the application (no public
    network path). Your documents are loaded into a throwaway database, your statements run against it in order, and the database is
    dropped when the run ends. Share links and anonymous usage counts live in separate databases on the same cluster, each with its
    own database user.
  </p>

  <div class="grid">
    <div class="card">
      <h3>What is stored</h3>
      <ul>
        <li>Share links: the playground you chose to share, public, for 90 days after the last view.</li>
        <li>Usage counts: what kind of action, how long it took, whether it worked. Never your queries or documents, never raw IP addresses.</li>
      </ul>
    </div>
    <div class="card">
      <h3>Limits</h3>
      {#if site.limits}
        <ul>
          <li>{site.limits.maxCollections} collections, {site.limits.maxDocsPerCollection} documents each</li>
          <li>{Math.round(site.limits.maxPayloadBytes / 1048576)} MB payload, {site.limits.maxTimeMs / 1000} s per statement</li>
          <li>{site.limits.maxIndexesPerCollection} indexes, 1 Atlas Search and 1 Vector Search index per collection</li>
          <li>No cap on the number of results</li>
        </ul>
      {:else}
        <p class="muted">See <a href="/api/v1/limits">/api/v1/limits</a>.</p>
      {/if}
    </div>
    <div class="card">
      <h3>What is blocked</h3>
      <p>Server-side JavaScript (<code>$where</code>, <code>$function</code>), <code>$out</code> and <code>$merge</code>, admin stages, and references to other databases.</p>
    </div>
  </div>

  <h2>How it is built</h2>
  <ul>
    <li>A Rust service (axum and the MongoDB Rust driver) with the shell parser, validation and the runner.</li>
    <li>A Svelte and CodeMirror web app, also packaged as a desktop app with Tauri.</li>
    <li>Cloud Run behind a Cloudflare-fronted load balancer, secrets in Secret Manager, everything defined in Terraform.</li>
    <li>A REST API with an OpenAPI spec, and an MCP server, so agents can use everything the web app can.</li>
  </ul>

  <h2>Use your own cluster</h2>
  <p>
    Want your data to stick around? <a href={registerUrl('about')} target="_blank" rel="noopener">Create a free Atlas cluster</a>, then
    <a href="/docs/run-it-yourself">run the playground against it</a>.
  </p>

  <h2>Terms of use</h2>
  <p class="muted">
    Don't paste secrets or personal data: shared links are public and runs are logged on the cluster for operations. This project is
    independent; MongoDB and Atlas are trademarks of MongoDB, Inc.
  </p>
</div>
