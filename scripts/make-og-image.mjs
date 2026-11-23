// Render web/public/og.svg to web/public/og.png (1200x630) with the Chromium that Playwright installs for the e2e tests.
// Usage: node scripts/make-og-image.mjs   (run from the repo root; `cd web && npx playwright install chromium` once)
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const { chromium } = createRequire(path.join(root, 'web', 'package.json'))('@playwright/test');
const svg = path.join(root, 'web', 'public', 'og.svg');
const out = path.join(root, 'web', 'public', 'og.png');

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
await page.goto(pathToFileURL(svg).href);
await page.screenshot({ path: out, clip: { x: 0, y: 0, width: 1200, height: 630 } });
await browser.close();
console.log('wrote', out);
