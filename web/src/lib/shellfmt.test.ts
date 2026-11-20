import { describe, expect, it } from 'vitest';
import { fmt } from './shellfmt';

describe('fmt (mongosh-style printing)', () => {
  it('prints unquoted keys and keeps short values on one line', () => {
    expect(fmt({ name: 'Ada', age: 36 })).toBe('{ name: "Ada", age: 36 }');
    expect(fmt([1, 2, 3])).toBe('[ 1, 2, 3 ]');
  });

  it('quotes keys that are not identifiers', () => {
    expect(fmt({ 'a.b': 1, 'c-d': 2 })).toBe('{ "a.b": 1, "c-d": 2 }');
  });

  it('turns extended JSON into shell constructors', () => {
    expect(fmt({ $oid: '507f1f77bcf86cd799439011' })).toBe('ObjectId("507f1f77bcf86cd799439011")');
    expect(fmt({ $date: '2024-01-01T00:00:00Z' })).toBe('ISODate("2024-01-01T00:00:00Z")');
    expect(fmt({ $uuid: 'x' })).toBe('UUID("x")');
    expect(fmt({ $numberLong: '7' })).toBe('Long("7")');
    expect(fmt({ $regularExpression: { pattern: '^a', options: 'i' } })).toBe('/^a/i');
  });

  it('prints dates before 1970 as ISO strings, not raw milliseconds', () => {
    expect(fmt({ $date: { $numberLong: '-4861728000000' } })).toBe('ISODate("1815-12-10T00:00:00.000Z")');
  });

  it('indents nested values that do not fit on one line', () => {
    const out = fmt({ items: Array.from({ length: 8 }, (_, i) => ({ name: `item number ${i}`, qty: i })) });
    expect(out.startsWith('{\n  items: [\n')).toBe(true);
    expect(out.split('\n').length).toBeGreaterThan(8);
  });

  it('handles null, empty containers and booleans', () => {
    expect(fmt(null)).toBe('null');
    expect(fmt({})).toBe('{}');
    expect(fmt([])).toBe('[]');
    expect(fmt({ ok: true })).toBe('{ ok: true }');
  });
});
