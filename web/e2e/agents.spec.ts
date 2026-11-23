import { expect, test } from '@playwright/test';

// Agents never open a browser: everything here is plain HTTP.
test('every page has a Markdown version for agents', async ({ request }) => {
  for (const path of ['/', '/docs/quickstart', '/docs/api', '/docs/mcp', '/docs/run-it-yourself', '/about']) {
    const asked = await request.get(path, { headers: { Accept: 'text/markdown' } });
    expect(asked.headers()['content-type'], path).toContain('text/markdown');
    expect(asked.headers()['vary'], path).toContain('Accept');
    expect(asked.headers()['x-markdown-tokens'], path).toBeTruthy();
    const text = await asked.text();
    expect(text, path).toMatch(/^---\ntitle: /);
    expect(text, path).not.toContain('{{');
    const twin = await request.get(path === '/' ? '/index.md' : `${path}.md`);
    expect(twin.headers()['content-type'], path).toContain('text/markdown');
    expect(await twin.text()).toBe(text);
  }
  // a browser still gets the app, and is told where the Markdown twin is
  const html = await request.get('/about', { headers: { Accept: 'text/html,application/xhtml+xml,*/*;q=0.8' } });
  expect(html.headers()['content-type']).toContain('text/html');
  expect(html.headers()['link']).toContain('rel="alternate"; type="text/markdown"');
});

test('discovery: llms.txt, OpenAPI, MCP card, setup guide', async ({ request }) => {
  expect(await (await request.get('/llms.txt')).text()).toContain('/api/v1/run');
  const spec = await (await request.get('/api/v1/openapi.json')).json();
  expect(spec.openapi).toMatch(/^3\.1/);
  expect(Object.keys(spec.paths)).toEqual(expect.arrayContaining(['/api/v1/run', '/api/v1/export', '/api/v1/verify']));
  const card = await (await request.get('/.well-known/mcp.json')).json();
  expect(card.transport).toBe('streamable-http');
  expect(card.tools).toContain('export_playground');
  const guide = await (await request.get('/api/v1/guide')).json();
  expect(JSON.stringify(guide)).toContain('utm_campaign=devrel&utm_source=third-party-content');
});

test('MCP: initialize, list tools, run a script', async ({ request }) => {
  const rpc = async (method: string, params: object = {}) =>
    (await (await request.post('/mcp', { data: { jsonrpc: '2.0', id: 1, method, params } })).json()).result;
  expect((await rpc('initialize', { protocolVersion: '2025-06-18' })).serverInfo.name).toBe('mongodb-playground');
  const tools = (await rpc('tools/list')).tools.map((t: { name: string }) => t.name);
  expect(tools).toEqual(expect.arrayContaining(['run_query', 'explain_query', 'export_playground', 'get_setup_guide']));
  const res = await rpc('tools/call', {
    name: 'run_query',
    // a bare array is the single collection named `collection`
    arguments: { script: 'db.collection.find({n: {$gt: 1}}, {_id: 0})', config: '[{n: 1}, {n: 2}]' },
  });
  expect(res.isError).toBe(false);
  expect(res.structuredContent.statements[0].results).toEqual([{ n: 2 }]);
});

test('REST: run, then export the same playground as a mongosh script', async ({ request }) => {
  const body = { script: 'db.people.find({age: {$gt: 40}}, {_id: 0})', config: 'db = { people: [{name: "Linus", age: 55}, {name: "Tim", age: 30}] }' };
  const run = await (await request.post('/api/v1/run', { data: body })).json();
  expect(run.ok).toBe(true);
  expect(run.statements[0].results).toEqual([{ name: 'Linus', age: 55 }]);
  const exp = await (await request.post('/api/v1/export', { data: { ...body, format: 'mongosh' } })).json();
  expect(exp.content).toContain('insertMany');
  expect(exp.content).toContain('db.people.find');
});
