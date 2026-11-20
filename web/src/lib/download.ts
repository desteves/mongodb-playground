/** Save text as a file in the browser. */
export function download(filename: string, text: string, mime = 'text/plain') {
  const url = URL.createObjectURL(new Blob([text], { type: `${mime};charset=utf-8` }));
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

const cell = (v: unknown): string => {
  const s = v === null || v === undefined ? '' : typeof v === 'object' ? JSON.stringify(v) : String(v);
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
};

/** Documents as CSV: the union of top-level keys is the header; nested values become JSON. */
export function toCsv(rows: unknown[]): string {
  const docs = rows.map((r) => (r && typeof r === 'object' && !Array.isArray(r) ? (r as Record<string, unknown>) : { value: r }));
  const keys: string[] = [];
  for (const d of docs) for (const k of Object.keys(d)) if (!keys.includes(k)) keys.push(k);
  return [keys.map(cell).join(','), ...docs.map((d) => keys.map((k) => cell(d[k])).join(','))].join('\n');
}
