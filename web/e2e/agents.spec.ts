import { expect, test } from '@playwright/test';

// Agents never open a browser: everything here is plain HTTP.
test('discovery: llms.txt, OpenAPI, MCP card, setup guide', async ({ request }) => {
  expect(await (await request.get('/llms.txt')).text()).toContain('/api/v1/run');
  const spec = await (await request.get('/api/v1/openapi.json')).json();
  expect(spec.openapi).toMatch(/^3\.1/);
  expect(Object.keys(spec.paths)).toEqual(expect.arrayContaining(['/api/v1/run', '/api/v1/export', '/api/v1/verify']));
  const card = await (await request.get('/.well-known/mcp.json')).json();
  expect(card.transport).toBe('streamable-http');
  expect(card.tools).toContain('export_playground');
  const guide = await (await request.get('/api/v1/guide')).json();
  expect(JSON.stringify(guide)).toContain('utm_source=mongoplayground');
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
