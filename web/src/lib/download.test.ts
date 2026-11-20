import { describe, expect, it } from 'vitest';
import { toCsv } from './download';

describe('toCsv', () => {
  it('uses the union of top-level keys as the header', () => {
    expect(toCsv([{ a: 1 }, { b: 2, a: 3 }])).toBe('a,b\n1,\n3,2');
  });

  it('quotes commas, quotes and newlines, and JSON-encodes nested values', () => {
    expect(toCsv([{ s: 'a,b', q: 'say "hi"', n: { x: 1 }, l: 'two\nlines' }])).toBe('s,q,n,l\n"a,b","say ""hi""","{""x"":1}","two\nlines"');
  });

  it('handles scalars and empty input', () => {
    expect(toCsv([1, 2])).toBe('value\n1\n2');
    expect(toCsv([])).toBe('');
  });
});
