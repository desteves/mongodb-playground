<script lang="ts">
  import { onMount } from 'svelte';
  import CodeBlock from '../lib/CodeBlock.svelte';
  import { site } from '../lib/site.svelte';

  type Op = { method: string; path: string; summary: string };
  let ops = $state<Op[]>([]);
  const origin = typeof location === 'undefined' ? '' : location.origin;

  onMount(async () => {
    try {
      const spec = await (await fetch('/api/v1/openapi.json')).json();
      ops = Object.entries(spec.paths as Record<string, Record<string, { summary?: string }>>).flatMap(([path, methods]) =>
        Object.entries(methods).map(([method, op]) => ({ method: method.toUpperCase(), path, summary: op.summary ?? '' })),
      );
    } catch {
      /* the static text below still works */
    }
  });

  const runExample = `curl -s -X POST ${origin}/api/v1/run \\
  -H 'content-type: application/json' \\
  -d '{
    "script": "db.people.insertOne({name: \\"Tim\\", age: 30})\\ndb.people.find({age: {$gt: 30}}).sort({age: -1})",
    "config": "db = { people: { documents: [{name: \\"Ada\\", age: 36}, {name: \\"Linus\\", age: 55}], indexes: [{key: {age: 1}}] } }"
  }'`;

  const responseExample = `{
  "ok": true,
  "statements": [
    { "index": 0, "statement": "db.people.insertOne({name: \\"Tim\\", age: 30})", "op": "insert", "write": { "inserted": 1 }, "results": [ ... ] },
    { "index": 1, "statement": "db.people.find({age: {$gt: 30}}).sort({age: -1})", "op": "find",
      "results": [ { "name": "Linus", "age": 55 }, { "name": "Ada", "age": 36 } ] }
  ],
  "elapsed_ms": 14
}`;
</script>

<div class="page">
  <h1>REST API</h1>
  <p class="lead">
    Everything the playground does is an HTTP call: run a mongosh-style script, explain it, validate it, share it, export it. No
    sign-up. The same service also speaks <a href="/docs/mcp">MCP</a>.
  </p>

  <h2>Run a script</h2>
  <p>
    <code>script</code> is several <code>db.collection.method(…)</code> statements run in order against one throwaway database.
    <code>config</code> is the data panel: documents, indexes, Atlas Search and Vector Search indexes per collection.
  </p>
  <CodeBlock code={runExample} label="Copy curl" />
  <h3>Response</h3>
  <CodeBlock code={responseExample} />
  <p>
    A statement that fails stops the script: <code>ok</code> is <code>false</code>, that statement carries
    <code>error: {'{code, message, hint}'}</code>, earlier output is kept. A request that cannot be parsed or validated is rejected
    before anything runs with HTTP 4xx and the statement and line in the message.
  </p>

  <h2>Endpoints</h2>
  <div class="card">
    <table>
      <thead><tr><th>Method</th><th>Path</th><th>What it does</th></tr></thead>
      <tbody>
        {#each ops as o}
          <tr><td><span class="badge {o.method.toLowerCase()}">{o.method}</span></td><td><code>{o.path}</code></td><td>{o.summary}</td></tr>
        {:else}
          <tr><td colspan="3" class="muted">Loading the endpoint list from <a href="/api/v1/openapi.json">/api/v1/openapi.json</a>…</td></tr>
        {/each}
      </tbody>
    </table>
  </div>

  <h2>Good to know</h2>
  <div class="grid">
    <div class="card">
      <h3>Errors</h3>
      <p>Every error is <code>{'{error: {code, message, hint}}'}</code> with a stable <code>code</code>: <code>invalid_request</code>, <code>forbidden</code>, <code>database_error</code>, <code>timeout</code>, <code>not_found</code>, <code>busy</code>, <code>rate_limited</code>, <code>unavailable</code>.</p>
    </div>
    <div class="card">
      <h3>Rate limits</h3>
      <p>
        Per client address, with <code>X-RateLimit-Remaining</code> on every response and <code>Retry-After</code> on a
        <code>429</code>.
        {#if site.limits?.rateLimit}
          Basic tier: {site.limits.rateLimit.basic.burst} requests, refilling {site.limits.rateLimit.basic.perSecond}/s. The web
          app passes a Cloudflare Turnstile check for the verified tier ({site.limits.rateLimit.verified.burst}, {site.limits.rateLimit.verified.perSecond}/s).
        {/if}
      </p>
    </div>
    <div class="card">
      <h3>Compression</h3>
      <p>Send <code>Content-Encoding: gzip</code> (or br, zstd) for large bodies and <code>Accept-Encoding</code> for compressed responses. The size limit applies after decompression.</p>
    </div>
    <div class="card">
      <h3>Limits</h3>
      {#if site.limits}
        <p>{site.limits.maxCollections} collections, {site.limits.maxDocsPerCollection} documents each, {Math.round(site.limits.maxPayloadBytes / 1048576)} MB payload, {site.limits.maxTimeMs / 1000} s per statement, {site.limits.maxIndexesPerCollection} indexes + 1 search + 1 vector index per collection, no cap on results.</p>
      {:else}
        <p><a href="/api/v1/limits">GET /api/v1/limits</a></p>
      {/if}
    </div>
  </div>

  <h2>For agents and tools</h2>
  <ul>
    <li><a href="/llms.txt" target="_blank" rel="noopener">/llms.txt</a>: a quick start written for LLMs</li>
    <li><a href="/api/v1/openapi.json" target="_blank" rel="noopener">/api/v1/openapi.json</a>: OpenAPI 3.1</li>
    <li><a href="/.well-known/mcp.json" target="_blank" rel="noopener">/.well-known/mcp.json</a>: MCP discovery card</li>
    <li><a href="/api/v1/examples" target="_blank" rel="noopener">/api/v1/examples</a>: templates and examples, each a valid request body</li>
    <li><a href="/api/v1/guide" target="_blank" rel="noopener">/api/v1/guide</a>: run it yourself, get a free or ephemeral Atlas cluster</li>
  </ul>
</div>
