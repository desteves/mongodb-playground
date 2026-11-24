import { expect, test } from '@playwright/test';

test('the left nav reaches every page and the head follows', async ({ page }) => {
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: 'Site' });
  const pages: [string, string, RegExp][] = [
    ['Quickstart', '/docs/quickstart', /Quickstart/],
    ['REST API', '/docs/api', /REST API/],
    ['MCP server', '/docs/mcp', /MCP Server/],
    ['Run it yourself', '/docs/run-it-yourself', /Run MongoDB Playground/],
    ['About', '/about', /About MongoDB Playground/],
  ];
  for (const [label, path, title] of pages) {
    await nav.getByRole('link', { name: label }).click();
    await expect(page).toHaveURL(new RegExp(`${path}$`));
    await expect(page).toHaveTitle(title);
    await expect(page.locator('h1')).toBeVisible();
  }
  await nav.getByRole('link', { name: 'Playground' }).click();
  await expect(page).toHaveTitle(/Online MongoDB Playground/);
});

test('About says which Atlas cluster tier it runs on', async ({ page }) => {
  await page.goto('/about');
  await expect(page.getByRole('heading', { name: /runs on a MongoDB Atlas M\d+ cluster/i })).toBeVisible();
});

test('every "free Atlas cluster" link carries UTM parameters', async ({ page }) => {
  await page.goto('/');
  const links = page.locator('a[href*="atlas/register"]');
  await expect(links.first()).toBeVisible();
  const hrefs = await links.evaluateAll((els) => els.map((e) => (e as HTMLAnchorElement).href));
  expect(hrefs.length).toBeGreaterThanOrEqual(3); // header, sidebar, footer
  for (const href of hrefs) {
    const u = new URL(href);
    expect(u.search).toContain(
      'utm_campaign=devrel&utm_source=third-party-content&utm_medium=cta&utm_content=mongodb-playground&utm_term=diana.esteves',
    );
  }
});

test('every link back to mongodb.com carries the UTM parameters, on every page', async ({ page }) => {
  for (const path of ['/', '/about', '/docs/quickstart', '/docs/api', '/docs/mcp', '/docs/run-it-yourself']) {
    await page.goto(path);
    const hrefs = await page.locator('a[href]').evaluateAll((els) => els.map((e) => (e as HTMLAnchorElement).href));
    for (const href of hrefs) {
      const host = new URL(href).hostname;
      if (host === 'mongodb.com' || host.endsWith('.mongodb.com')) {
        expect(href, `${path} -> ${href}`).toContain('utm_campaign=devrel&utm_source=third-party-content');
      }
    }
  }
});

test('the API and MCP pages show how to connect', async ({ page }) => {
  await page.goto('/docs/mcp');
  await expect(page.getByText('claude mcp add --transport http mongodb-playground')).toBeVisible();
  await expect(page.getByText('export_playground')).toBeVisible(); // tool list comes from the live server
  await page.goto('/docs/api');
  await expect(page.getByText('/api/v1/run').first()).toBeVisible();
  await expect(page.locator('table')).toContainText('/api/v1/export');
});

test('the run-it-yourself page lists local, free and ephemeral Atlas options', async ({ page }) => {
  await page.goto('/docs/run-it-yourself');
  for (const t of [/Which one should I use/, /Run locally with the Atlas CLI/, /Free-forever Atlas cluster/, /Ephemeral Atlas cluster/, /Docker Compose/]) {
    await expect(page.getByRole('heading', { name: t })).toBeVisible();
  }
});

test('search engines get what they need', async ({ page, request }) => {
  await page.goto('/');
  await expect(page.locator('meta[name="description"]')).toHaveAttribute('content', /online MongoDB playground/i);
  await expect(page.locator('meta[name="keywords"]')).toHaveAttribute('content', /online mongodb playground.*mongodb playground/i);
  await expect(page.locator('link[rel="canonical"]')).toHaveAttribute('href', 'https://mongoplayground.com/');
  // The home page carries a WebApplication and a FAQPage block.
  const blocks = await page.locator('script[type="application/ld+json"]').allTextContents();
  expect(blocks.map((b) => JSON.parse(b)['@type']).sort()).toEqual(['FAQPage', 'WebApplication']);
  // Crawlers that do not run JavaScript get each page's own text, title and structured data in the HTML.
  const about = await (await request.get('/about')).text();
  expect(about).toContain('<h1>About MongoDB Playground</h1>');
  expect(about).toContain('"@type":"AboutPage"');
  await expect(page.locator('meta[property="og:image"]')).toHaveAttribute('content', 'https://mongoplayground.com/og.png');
  const png = await request.get('/og.png');
  expect(png.headers()['content-type']).toContain('image/png');
  const robots = await request.get('/robots.txt');
  expect(await robots.text()).toContain('Sitemap: https://mongoplayground.com/sitemap.xml');
  const sitemap = await request.get('/sitemap.xml');
  expect(await sitemap.text()).toContain('/docs/mcp');
});

test('unknown pages say so', async ({ page }) => {
  const res = await page.goto('/no/such/page');
  // A real 404, not a 200: made-up URLs must not look like pages of the site to a search engine.
  expect(res?.status()).toBe(404);
  await expect(page.getByRole('heading', { name: 'Page not found' })).toBeVisible();
});

test('About links to the GitHub repository', async ({ page }) => {
  await page.goto('/about');
  const link = page.getByRole('link', { name: /github\.com\/desteves\/mongodb-playground/ });
  await expect(link).toHaveAttribute('href', 'https://github.com/desteves/mongodb-playground/tree/main');
});

test('"Which one should I use?" links jump to their option, and links read as words, not URLs', async ({ page }) => {
  await page.goto('/docs/run-it-yourself');
  await page.getByRole('heading', { name: 'Which one should I use?' }).waitFor();

  // Each entry scrolls its option into view and puts the fragment in the address.
  await page.locator('.choose a', { hasText: 'Free-forever Atlas cluster' }).click();
  await expect(page).toHaveURL(/#atlas-free$/);
  await expect(page.locator('#atlas-free')).toBeInViewport();

  // A direct link with a fragment lands there too.
  await page.goto('/docs/run-it-yourself#docker');
  await expect(page.locator('#docker')).toBeInViewport();

  // No link is shown as a bare URL; the Atlas sign-up step reads as its label.
  const texts = await page.locator('.page a').allTextContents();
  expect(texts.filter((t) => /^\s*(https?:\/\/|www\.)/.test(t))).toEqual([]);
  await expect(page.locator('#atlas-free a', { hasText: 'Register for Atlas' })).toBeVisible();
});
