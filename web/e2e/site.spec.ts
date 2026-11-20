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
  await expect(page).toHaveTitle(/Online MongoDB Shell/);
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
    expect(u.searchParams.get('utm_source')).toBe('mongoplayground');
    expect(u.searchParams.get('utm_medium')).toBe('referral');
    expect(u.searchParams.get('utm_campaign')).toBe('playground-free-cluster');
    expect(u.searchParams.get('utm_content')).toBeTruthy();
  }
  const placements = new Set(hrefs.map((h) => new URL(h).searchParams.get('utm_content')));
  expect(placements).toContain('header');
  expect(placements).toContain('footer');
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
  for (const t of [/Run locally with the Atlas CLI/, /Free-forever Atlas cluster/, /Ephemeral Atlas cluster/, /Docker Compose/]) {
    await expect(page.getByRole('heading', { name: t })).toBeVisible();
  }
});

test('search engines get what they need', async ({ page, request }) => {
  await page.goto('/');
  await expect(page.locator('meta[name="description"]')).toHaveAttribute('content', /online MongoDB shell/i);
  await expect(page.locator('meta[name="keywords"]')).toHaveAttribute('content', /online mongodb shell.*mongodb playground/i);
  await expect(page.locator('link[rel="canonical"]')).toHaveAttribute('href', 'https://mongoplayground.com/');
  const ld = JSON.parse((await page.locator('script[type="application/ld+json"]').textContent()) ?? '{}');
  expect(ld['@type']).toBe('WebApplication');
  const robots = await request.get('/robots.txt');
  expect(await robots.text()).toContain('Sitemap: https://mongoplayground.com/sitemap.xml');
  const sitemap = await request.get('/sitemap.xml');
  expect(await sitemap.text()).toContain('/docs/mcp');
});

test('unknown pages say so', async ({ page }) => {
  await page.goto('/no/such/page');
  await expect(page.getByRole('heading', { name: 'Page not found' })).toBeVisible();
});
