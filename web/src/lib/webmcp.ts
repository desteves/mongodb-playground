import { examples, explain, formatText, limits, run, type ScriptBody } from './api';

/**
 * WebMCP: lets a browsing agent call the playground's own functions as tools (the same calls the buttons make), instead of
 * driving the editors. Registered only where the browser offers `navigator.modelContext` (Chrome's experimental API).
 */
const body = {
  type: 'object',
  properties: {
    script: { type: 'string', description: 'mongosh-style statements, for example db.coll.find({}). Up to 25 statements.' },
    config: {
      type: 'string',
      description: 'The data: per-collection `{ coll: { documents: [...], indexes: [...] } }`, `db = { coll: [docs] }`, or a plain `[docs]` array.',
    },
  },
  required: ['script'],
  additionalProperties: false,
} as const;

const text = (value: unknown) => ({ content: [{ type: 'text', text: JSON.stringify(value) }] });
const failure = (e: any) => ({ isError: true, content: [{ type: 'text', text: `${e?.code ?? 'error'}: ${e?.message ?? e}${e?.hint ? ` (hint: ${e.hint})` : ''}` }] });
const guarded = (fn: (args: any) => Promise<unknown>) => async (args: any) => {
  try {
    return text(await fn(args));
  } catch (e) {
    return failure(e);
  }
};

export function registerWebMcpTools(): void {
  const mc = (navigator as any).modelContext;
  if (!mc?.registerTool) return;
  const tools = [
    { name: 'run_query', description: 'Run mongosh-style statements against the given data on a real MongoDB cluster and return each statement\'s result.', inputSchema: body, execute: guarded((a) => run(a as ScriptBody)) },
    { name: 'explain_query', description: 'Show the query plan (stages, index used, documents examined) for the statements.', inputSchema: body, execute: guarded((a) => explain(a as ScriptBody)) },
    {
      name: 'format_text',
      description: 'Tidy the indentation and line breaks of the data or shell text. Only whitespace and trailing commas change.',
      inputSchema: { type: 'object', properties: { text: { type: 'string', description: 'The text to format.' } }, required: ['text'], additionalProperties: false },
      execute: guarded((a) => formatText(String(a.text))),
    },
    { name: 'list_examples', description: 'List the ready-made examples (title, data and script) to start from.', inputSchema: { type: 'object', properties: {}, additionalProperties: false }, execute: guarded(() => examples()) },
    { name: 'get_limits', description: 'The limits (collections, documents, statements, indexes, timeouts) and what this cluster supports.', inputSchema: { type: 'object', properties: {}, additionalProperties: false }, execute: guarded(() => limits()) },
  ];
  for (const t of tools) {
    try {
      mc.registerTool(t);
    } catch {
      /* already registered, or the browser rejected the schema: the page works without it */
    }
  }
}
