/** Print values the way mongosh does: unquoted keys, ObjectId("..."), ISODate("..."), /regex/i. */

const IDENT = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

export function fmt(value: unknown, indent = 0): string {
  const pad = '  '.repeat(indent);
  const pad1 = '  '.repeat(indent + 1);
  if (value === null || value === undefined) return 'null';
  if (typeof value === 'string') return JSON.stringify(value);
  if (typeof value !== 'object') return String(value);
  if (Array.isArray(value)) {
    if (value.length === 0) return '[]';
    const items = value.map((v) => pad1 + fmt(v, indent + 1));
    // Short arrays of scalars stay on one line.
    const flat = value.map((v) => fmt(v, 0));
    if (flat.every((f) => !f.includes('\n')) && flat.join(', ').length < 72) return `[ ${flat.join(', ')} ]`;
    return `[\n${items.join(',\n')}\n${pad}]`;
  }
  const o = value as Record<string, unknown>;
  const keys = Object.keys(o);
  if (keys.length === 1) {
    const k = keys[0];
    const v = o[k];
    if (k === '$oid') return `ObjectId(${JSON.stringify(v)})`;
    if (k === '$date') {
      // Dates before 1970 or far in the future come back as {"$numberLong": "<ms>"}.
      const ms = typeof v === 'object' && v !== null ? Number((v as { $numberLong?: string }).$numberLong) : NaN;
      const iso = Number.isFinite(ms) ? new Date(ms).toISOString() : v;
      return `ISODate(${JSON.stringify(iso)})`;
    }
    if (k === '$uuid') return `UUID(${JSON.stringify(v)})`;
    if (k === '$numberLong') return `Long(${JSON.stringify(v)})`;
    if (k === '$numberDecimal') return `Decimal128(${JSON.stringify(v)})`;
    if (k === '$regularExpression') {
      const r = v as { pattern: string; options: string };
      return `/${r.pattern}/${r.options}`;
    }
  }
  if (keys.length === 0) return '{}';
  const lines = keys.map((k) => `${pad1}${IDENT.test(k) ? k : JSON.stringify(k)}: ${fmt(o[k], indent + 1)}`);
  const flat = keys.map((k) => `${IDENT.test(k) ? k : JSON.stringify(k)}: ${fmt(o[k], 0)}`);
  if (flat.every((f) => !f.includes('\n')) && flat.join(', ').length < 72) return `{ ${flat.join(', ')} }`;
  return `{\n${lines.join(',\n')}\n${pad}}`;
}
