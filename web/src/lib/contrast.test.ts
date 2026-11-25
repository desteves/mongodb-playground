import { describe, expect, it } from 'vitest';
// @ts-expect-error: node types are not installed; vitest runs in node, so the module exists
import { readFileSync } from 'node:fs';
import { darkSyntax, lightSyntax } from './editorColors';

// WCAG 2.x contrast ratio between two #rrggbb colours.
const channel = (v: number) => {
  const c = v / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
};
const luminance = (hex: string) => {
  const n = parseInt(hex.slice(1), 16);
  return 0.2126 * channel((n >> 16) & 255) + 0.7152 * channel((n >> 8) & 255) + 0.0722 * channel(n & 255);
};
const ratio = (a: string, b: string) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

/** The `--name: #hex` variables of one theme block in app.css. */
const css: string = readFileSync(new URL('../app.css', import.meta.url), 'utf8');

function variables(selector: string): Record<string, string> {
  const start = css.indexOf(selector + ' {');
  const block = css.slice(start, css.indexOf('}', start));
  return Object.fromEntries([...block.matchAll(/(--[\w-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)].map((m) => [m[1], m[2]]));
}

const themes = {
  light: { vars: variables(':root'), syntax: lightSyntax },
  dark: { vars: variables(":root[data-theme='dark']"), syntax: darkSyntax },
} as const;

describe.each(Object.entries(themes))('%s theme contrast (WCAG AA, 4.5:1 for text)', (_name, { vars, syntax }) => {
  // The editors sit on the panel shades; their text must be readable on all of them.
  const editorBackgrounds = ['--editor-bg', '--pane-data', '--pane-shell', '--pane-result'];

  it('has the variables the check needs', () => {
    for (const v of [...editorBackgrounds, '--bg', '--surface', '--fg', '--muted', '--link', '--accent']) expect(vars[v], v).toBeTruthy();
  });

  it.each(Object.entries(syntax))('the %s colour in the editors', (_token, colour) => {
    for (const bg of editorBackgrounds) expect(ratio(colour, vars[bg]), `${colour} on ${bg} ${vars[bg]}`).toBeGreaterThanOrEqual(4.5);
  });

  it('page text, secondary text and links on the page and panel shades', () => {
    const surfaces = ['--bg', '--surface', '--pane-data-head', '--pane-shell-head', '--pane-result-head', '--editor-gutter'];
    for (const bg of surfaces) {
      for (const fg of ['--fg', '--muted']) expect(ratio(vars[fg], vars[bg]), `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
    }
    for (const bg of ['--bg', '--surface']) {
      for (const fg of ['--link', '--accent']) expect(ratio(vars[fg], vars[bg]), `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
    }
  });
});
