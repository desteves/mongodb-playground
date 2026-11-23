<script lang="ts">
  import CodeBlock from '../lib/CodeBlock.svelte';
  import { registerUrl } from '../lib/site.svelte';

  const crud = `db.collection.insertOne({ name: "Tim", age: 30, city: "London" })
db.collection.updateMany({ city: "London" }, { $set: { country: "UK" } })
db.collection.find({ country: "UK" }, { _id: 0, name: 1 }).sort({ name: 1 })
db.collection.countDocuments({})`;

  const shape = `db = {
  orders:    { documents: [ ... ], indexes: [ { key: { item: 1 } } ] },
  inventory: { documents: [ ... ] }
}`;
</script>

<div class="page">
  <h1>Quickstart</h1>
  <p class="lead">Test a MongoDB query in about a minute: no install, no account. Each step has a button that opens the playground with that example ready to run.</p>

  <h2>The three panels</h2>
  <div class="grid">
    <div class="card"><h3>Data</h3><p>Input data to test: your documents, plus the indexes, Atlas Search and Vector Search indexes you want on them.</p></div>
    <div class="card"><h3>Shell</h3><p>Run mongosh commands: CRUD operations, aggregations, query plans, Vector Search and more. Several statements run in order.</p></div>
    <div class="card"><h3>Results</h3><p>The output: what each statement returned, the plans, and any error with a hint. Each panel can be collapsed.</p></div>
  </div>

  <h2>1. Run your first query</h2>
  <p>Open the playground. It starts with a small collection and <code>db.collection.find({'{ age: { $gt: 40 } }'})</code>. Press <strong>▶ Run</strong> (or <kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>Enter</kbd>).</p>
  <p><a class="btn btn-primary" href="/?example=single">Open the first example</a></p>

  <h2>2. Write several statements</h2>
  <p>The shell runs statements one after another against the same data, so an insert is visible to the next query.</p>
  <CodeBlock code={crud} />
  <ul>
    <li><strong>Run all</strong> runs everything in the Shell. <strong>Run selection</strong> runs only the statements you highlighted.</li>
    <li>If a statement fails, the script stops there and keeps the output so far.</li>
  </ul>
  <p><a class="btn" href="/?example=crud-session">Try a CRUD session</a></p>

  <h2>3. See how MongoDB runs it</h2>
  <p>
    <strong>Explain plan</strong> (in the Shell panel) shows the query plan instead of running the statements: whether an index was used,
    how many documents were examined, and plain-language advice. Add an index in the Data panel and explain again.
  </p>
  <p><a class="btn" href="/?example=index-explain">See an index being used</a></p>

  <h2>4. Add more data and indexes</h2>
  <p>The Data panel takes one entry per collection, each with its documents and indexes, so one collection or many use the same shape.</p>
  <CodeBlock code={shape} />
  <p><a class="btn" href="/?example=multi">Open the multi-collection template</a></p>

  <h2>5. Try search and vector search</h2>
  <p>Declare a <code>searchIndexes</code> or <code>vectorSearchIndexes</code> entry for a collection and use <code>$search</code> or <code>$vectorSearch</code>. (This needs a deployment with Atlas Search; the examples tell you if it is not available.)</p>
  <p>
    <a class="btn" href="/?example=atlas-search">Atlas Search</a>
    <a class="btn" href="/?example=vector-search">Vector Search</a>
  </p>

  <h2>6. Share and export</h2>
  <ul>
    <li><strong>Share</strong> creates a link (a UUID) to your data and statements. Anyone with the link can open it for 90 days after its last view.</li>
    <li><strong>Export</strong> downloads a <code>mongosh</code> script that recreates everything against your own MongoDB, the data as JSON, the results as JSON or CSV, or a <code>curl</code> command.</li>
  </ul>

  <h2>7. Use it from code or an agent</h2>
  <p>Everything the page does is an API call, and there is an MCP server for AI agents. See the <a href="/docs/api">REST API</a> and <a href="/docs/mcp">MCP server</a> pages.</p>

  <h2>Keep your data</h2>
  <p>The playground uses throwaway databases. To keep data, create your own cluster and <a href="/docs/run-it-yourself">run the playground against it</a>.</p>
  <p><a class="btn btn-primary" href={registerUrl()} target="_blank" rel="noopener">Create a free Atlas cluster ↗</a></p>

  <h2>Cheat sheet</h2>
  <div class="card">
    <table>
      <tbody>
        <tr><td>Read</td><td><code>find</code>, <code>findOne</code>, <code>aggregate</code>, <code>countDocuments</code>, <code>distinct</code></td></tr>
        <tr><td>Write</td><td><code>insertOne/Many</code>, <code>updateOne/Many</code>, <code>replaceOne</code>, <code>deleteOne/Many</code></td></tr>
        <tr><td>Indexes</td><td><code>createIndex</code>, <code>dropIndex</code>, <code>getIndexes</code>, <code>createSearchIndex</code></td></tr>
        <tr><td>Cursor</td><td><code>.sort()</code> <code>.limit()</code> <code>.skip()</code> <code>.projection()</code> <code>.count()</code> <code>.explain()</code></td></tr>
        <tr><td>Values</td><td>unquoted keys, <code>'single quotes'</code>, <code>/regex/i</code>, <code>ObjectId()</code>, <code>ISODate()</code>, comments</td></tr>
        <tr><td>Shortcuts</td><td><kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>Enter</kbd> runs the highlighted statements, or everything if nothing is highlighted</td></tr>
      </tbody>
    </table>
  </div>
</div>

<style>
  kbd {
    font-size: 0.8em;
    padding: 0.05rem 0.4rem;
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: 5px;
    background: var(--surface);
  }
</style>
