import { expect, test } from '@playwright/test';

test('phone layout: tabs, a drawer, big Run, no sideways scrolling', async ({ page }) => {
  await page.goto('/');
  // panels become tabs
  const tabs = page.getByRole('navigation', { name: 'Panels' });
  await expect(tabs.getByRole('button', { name: 'Data' })).toBeVisible();
  await expect(tabs.getByRole('button', { name: 'Shell' })).toBeVisible();

  // Run is large and a run switches to the Results tab
  const run = page.getByRole('button', { name: 'Run the script' });
  const box = await run.boundingBox();
  expect(box!.height).toBeGreaterThanOrEqual(40);
  await run.click();
  await expect(tabs.getByRole('button', { name: /Results/ })).toHaveClass(/on/);
  await expect(page.locator('.pane:not([hidden]) .cm-content')).toContainText('name: "Linus"');

  // the page does not scroll sideways
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);

  // the drawer holds the site nav
  await page.getByRole('button', { name: 'Open the menu' }).click();
  await page.getByRole('navigation', { name: 'Site' }).getByRole('link', { name: 'About' }).click();
  await expect(page).toHaveURL(/\/about$/);
  await expect(page.locator('h1')).toContainText('About');
});

test('editors use 16px text on phones so iOS does not zoom', async ({ page }) => {
  await page.goto('/');
  const size = await page.locator('.cm-editor').first().evaluate((e) => getComputedStyle(e).fontSize);
  expect(size).toBe('16px');
});
