import { describe, expect, it } from 'vitest';
import type { ScriptOutput, StatementOutput } from './api';
import { transcript } from './transcript';

const stmt = (o: Partial<StatementOutput>): StatementOutput => ({
  index: 0,
  line: 1,
  statement: 'db.c.find()',
  collection: 'c',
  op: 'find',
  results: [],
  elapsed_ms: 1,
  ...o,
});
const out = (statements: StatementOutput[]): ScriptOutput => ({ statements, ok: true, elapsed_ms: 1 });

describe('transcript', () => {
  it('points at each command by its line instead of repeating it', () => {
    const t = transcript(out([stmt({ results: [{ name: 'Ada' }] }), stmt({ line: 4, statement: 'db.c.countDocuments()', op: 'count', results: [{ count: 3 }] })]));
    expect(t).toBe('// results for the command on line 1\n[ { name: "Ada" } ]\n\n// results for the command on line 4\n3');
    expect(t).not.toContain('db.c.');
  });

  it('counts lines from the top of the Shell when only a highlighted part ran', () => {
    expect(transcript(out([stmt({ line: 1 })]), { lineOffset: 4 })).toContain('// results for the command on line 5');
  });

  it('can include the statement text, for exports that must read on their own', () => {
    const t = transcript(out([stmt({ results: [{ name: 'Ada' }] }), stmt({ statement: 'db.c.countDocuments()', op: 'count', results: [{ count: 3 }] })]), { echo: true });
    expect(t).toBe('> db.c.find()\n[ { name: "Ada" } ]\n\n> db.c.countDocuments()\n3');
  });

  it('prints write acknowledgements, not the whole collection', () => {
    const t = transcript(out([stmt({ statement: 'db.c.insertOne({a: 1})', op: 'insert', write: { inserted: 1 }, results: [{ a: 1 }, { a: 2 }] })]));
    expect(t).toContain('{ acknowledged: true, inserted: 1 }');
    expect(t).toContain('// c now has 2 documents');
    expect(t).not.toContain('a: 2');
  });

  it('prints errors and hints as comments', () => {
    const t = transcript(out([stmt({ line: 2, error: { code: 'database_error', message: 'unknown operator: $nope', hint: 'Fix the operator.' } })]));
    expect(t).toContain('// results for the command on line 2');
    expect(t).toContain('// error: unknown operator: $nope');
    expect(t).toContain('// hint: Fix the operator.');
  });

  it('summarises explain plans with their advice', () => {
    const t = transcript(
      out([
        stmt({
          op: 'explain',
          explain: { raw: {}, summary: { verbosity: 'executionStats', stages: ['COLLSCAN'], indexUsed: null, collscan: true, keysExamined: 0, docsExamined: 3, returned: 1, executionTimeMs: 0, advice: ['COLLSCAN: add an index.'] } },
        }),
      ]),
    );
    expect(t).toContain('collscan: true');
    expect(t).toContain('// COLLSCAN: add an index.');
  });

  it('flags truncated sample reads', () => {
    expect(transcript(out([stmt({ results: [{ a: 1 }], truncated: true })]))).toContain('// truncated');
  });
});
