import type { ScriptOutput, StatementOutput } from './api';
import { fmt } from './shellfmt';

const WRITES = new Set(['insert', 'update', 'delete']);

/** A mongosh-style transcript: each statement, then what it printed. */
export function transcript(out: ScriptOutput): string {
  return out.statements.map(one).join('\n\n');
}

function one(s: StatementOutput): string {
  const lines: string[] = [`> ${s.statement}`];
  if (s.error) {
    lines.push(`// error: ${s.error.message}`);
    if (s.error.hint) lines.push(`// hint: ${s.error.hint}`);
    return lines.join('\n');
  }
  if (s.explain) {
    const e = s.explain.summary;
    lines.push(
      fmt({
        stages: e.stages,
        indexUsed: e.indexUsed,
        collscan: e.collscan,
        keysExamined: e.keysExamined,
        docsExamined: e.docsExamined,
        returned: e.returned,
        executionTimeMs: e.executionTimeMs,
      }),
    );
    for (const a of e.advice) lines.push(`// ${a}`);
    return lines.join('\n');
  }
  if (s.write) {
    // Like the shell: the acknowledgement, not the whole collection. Follow with a find to see it.
    lines.push(fmt({ acknowledged: true, ...s.write }));
    if (WRITES.has(s.op)) {
      lines.push(`// ${s.collection} now has ${s.results.length} document${s.results.length === 1 ? '' : 's'}`);
    }
    return lines.join('\n');
  }
  if (s.op === 'count') lines.push(fmt((s.results[0] as { count: number })?.count ?? 0));
  else lines.push(fmt(s.results));
  if (s.truncated) lines.push('// truncated: sample reads are capped');
  return lines.join('\n');
}
