<script lang="ts">
  import { onMount } from 'svelte';
  import CodeBlock from '../lib/CodeBlock.svelte';

  const origin = typeof location === 'undefined' ? '' : location.origin;
  const mcpUrl = `${origin}/mcp`;
  let tools = $state<{ name: string; description: string }[]>([]);

  onMount(async () => {
    try {
      const res = await fetch('/api/v1/mcp', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
      });
      tools = (await res.json()).result.tools;
    } catch {
      /* the page still explains how to connect */
    }
  });

  const claude = `claude mcp add --transport http mongodb-playground ${mcpUrl}`;
  const json = JSON.stringify({ mcpServers: { 'mongodb-playground': { type: 'http', url: mcpUrl } } }, null, 2);
  const prompts = [
    'Use the MongoDB playground to test this aggregation on my sample documents and explain the plan.',
    'Create a playground with a vector index on "embedding" and run a $vectorSearch for [0.1, 0.9, 0.0].',
    'Export this playground as a mongosh script I can run against my own cluster.',
  ];
</script>

<div class="page">
  <h1>MCP server</h1>
  <p class="lead">
    Give an AI agent a real MongoDB to test queries against. The playground exposes a stateless Streamable HTTP
    <a href="https://modelcontextprotocol.io" target="_blank" rel="noopener">Model Context Protocol</a> server: no sign-up, no API key.
  </p>

  <h2>Connect</h2>
  <h3>Server URL</h3>
  <CodeBlock code={mcpUrl} />
  <h3>Claude Code</h3>
  <CodeBlock code={claude} />
  <h3>Any MCP client (JSON config)</h3>
  <CodeBlock code={json} />
  <p class="muted">Discovery card: <a href="/.well-known/mcp.json" target="_blank" rel="noopener">/.well-known/mcp.json</a></p>

  <h2>Tools</h2>
  <div class="grid">
    {#each tools as t}
      <div class="card">
        <h3><code>{t.name}</code></h3>
        <p class="muted">{t.description.length > 260 ? t.description.slice(0, 257) + '…' : t.description}</p>
      </div>
    {:else}
      <p class="muted">Loading the tool list…</p>
    {/each}
  </div>

  <h2>Try asking</h2>
  <ul>
    {#each prompts as p}<li>{p}</li>{/each}
  </ul>

  <h2>How it behaves</h2>
  <ul>
    <li>Scripts are mongosh-style: several statements run in order against one throwaway database; the result is a transcript.</li>
    <li>If a statement fails, <code>isError</code> is true and the transcript up to that point is still returned, with a <code>hint</code>.</li>
    <li>Limits and capabilities (Atlas Search, rate limits) come from <code>get_limits</code>; setup help from <code>get_setup_guide</code>.</li>
    <li>The same service is available as a <a href="/docs/api">REST API</a>. To see what a tool call does, open an <a href="/?example=crud-session">example in the web app</a> or follow the <a href="/docs/quickstart">Quickstart</a>.</li>
  </ul>
</div>
