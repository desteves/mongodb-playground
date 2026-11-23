import { expect, test, type Page } from '@playwright/test';

// The three panels in DOM order: data, shell, results. Each editor is a CodeMirror view.
const editor = (page: Page, i: number) => page.locator('.pane').nth(i).locator('.cm-content');
const run = (page: Page) => page.getByRole('button', { name: 'Run all statements' });

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await expect(editor(page, 1)).toContainText('db.collection.find');
});

test('opens on the single-collection template, in the per-collection shape', async ({ page }) => {
  await expect(editor(page, 0)).toContainText('collection: {');
  await expect(editor(page, 0)).toContainText('documents: [');
  await expect(editor(page, 0)).toContainText('indexes: [');
  const nav = page.getByRole('navigation', { name: 'Site' });
  await expect(nav.getByRole('link', { name: 'Playground', exact: true })).toHaveAttribute('aria-current', 'page');
});

test('Run prints a shell-style transcript', async ({ page }) => {
  await run(page).click();
  await expect(editor(page, 2)).toContainText('> db.collection.find({ age: { $gt: 40 } })');
  await expect(editor(page, 2)).toContainText('name: "Linus"');
  await expect(editor(page, 2)).toContainText('name: "Grace"');
  await expect(editor(page, 2)).not.toContainText('name: "Ada"');
  await expect(page.locator('.pane').nth(2)).toContainText('1 statement');
});

test('several statements run in order and writes print acknowledgements', async ({ page }) => {
  await editor(page, 1).click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.type(
    'db.collection.insertOne({ name: "Tim", age: 30 })\ndb.collection.countDocuments({})\ndb.collection.find({ name: "Tim" }, { _id: 0 })',
  );
  await run(page).click();
  const out = editor(page, 2);
  await expect(out).toContainText('acknowledged: true');
  await expect(out).toContainText('inserted: 1');
  await expect(out).toContainText('{ name: "Tim", age: 30 }');
  await expect(page.locator('.pane').nth(2)).toContainText('3 statements');
});

test('a failing statement stops the script and says why', async ({ page }) => {
  await editor(page, 1).click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.type('db.collection.find({ age: { $nope: 1 } })\ndb.collection.countDocuments({})');
  await run(page).click();
  await expect(editor(page, 2)).toContainText('// error:');
  await expect(editor(page, 2)).not.toContainText('countDocuments');
  await expect(page.locator('.pane').nth(2)).toContainText('stopped at an error');
});

test('Explain shows the plan, and the index from the data panel is used', async ({ page }) => {
  await editor(page, 1).click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.type('db.collection.find({ age: 36 })');
  await page.getByRole('button', { name: 'Explain plan' }).click();
  await expect(editor(page, 2)).toContainText('indexUsed: "age_1"');
  await expect(editor(page, 2)).toContainText('collscan: false');
});

test('Clear empties a panel and Undo brings it back', async ({ page }) => {
  const before = await editor(page, 1).innerText();
  await page.locator('.pane').nth(1).getByRole('button', { name: 'Clear' }).click();
  await expect(editor(page, 1)).toHaveText(/^\s*$/);
  await page.getByRole('button', { name: 'Undo in the shell' }).click();
  await expect(editor(page, 1)).toHaveText(before);
});

test('templates: single and multiple use the same shape, and the multi one runs', async ({ page }) => {
  // below ~1100px the template picker lives in the "more" menu; above it the picker is inline
  const more = page.locator('.more-btn');
  if (await more.isVisible()) await more.click();
  await page.getByLabel('Load an example').selectOption('multi');
  await expect(editor(page, 0)).toContainText('orders: {');
  await expect(editor(page, 0)).toContainText('inventory: {');
  await expect(editor(page, 0)).toContainText('documents: [');
  await run(page).click();
  await expect(editor(page, 2)).toContainText('total:');
});

test('Share makes a /p/<id> link that reopens the same playground', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']).catch(() => {});
  await editor(page, 1).click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.type('db.collection.find({ city: /^lon/i }, { _id: 0, name: 1 })');
  await page.getByRole('button', { name: 'Share' }).click();
  await expect(page).toHaveURL(/\/p\/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  // a visible dialog shows the link, so Share is never silent
  const dialog = page.getByRole('dialog', { name: 'Your share link' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel('Share link')).toHaveValue(/\/p\/[0-9a-f-]{36}$/);
  await dialog.getByRole('button', { name: 'Close' }).click();
  await page.reload();
  await expect(editor(page, 1)).toContainText('/^lon/i');
  await run(page).click();
  await expect(editor(page, 2)).toContainText('name: "Ada"');
});

test('Export downloads a mongosh script that recreates the data', async ({ page }) => {
  await page.getByRole('button', { name: /^Export/ }).click();
  await page.getByRole('button', { name: 'Script for mongosh (.js)' }).click();
  // a preview first: nothing is downloaded until it is confirmed, and the text can be copied from the box
  const dialog = page.getByRole('dialog', { name: 'Script for mongosh' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel('Export preview')).toHaveValue(/use\("playground"\);/);
  await expect(dialog.getByRole('button', { name: 'Copy' })).toBeVisible();
  const [download] = await Promise.all([page.waitForEvent('download'), dialog.getByRole('button', { name: /^Download/ }).click()]);
  expect(download.suggestedFilename()).toBe('playground.js');
  const text = (await (await download.createReadStream()).toArray()).join('');
  expect(text).toContain('use("playground");');
  expect(text).toContain('insertMany');
  expect(text).toContain('createIndex({ age: 1 })');
  expect(text).toContain('db.collection.find({ age: { $gt: 40 } })');
});

test('Export results as CSV after a run', async ({ page }) => {
  await run(page).click();
  await expect(editor(page, 2)).toContainText('name: "Linus"');
  await page.getByRole('button', { name: /^Export/ }).click();
  await page.getByRole('button', { name: 'Last result as CSV' }).click();
  const dialog = page.getByRole('dialog', { name: 'Last result as CSV' });
  await expect(dialog.getByLabel('Export preview')).toHaveValue(/Linus/);
  const [download] = await Promise.all([page.waitForEvent('download'), dialog.getByRole('button', { name: /^Download/ }).click()]);
  expect(download.suggestedFilename()).toBe('results.csv');
  const csv = (await (await download.createReadStream()).toArray()).join('');
  expect(csv.split('\n')[0]).toContain('name');
  expect(csv).toContain('Linus');
});

test('sample data is off by default (feature flag)', async ({ page }) => {
  await expect(page.getByLabel('Load an example')).toBeAttached();
  await expect(page.getByText('Atlas sample data')).toHaveCount(0); // no data-source picker
  const options = await page.getByLabel('Load an example').locator('option').allTextContents();
  expect(options.some((o) => /sample_mflix|sample dataset/i.test(o))).toBe(false); // and no sample example to pick
});

test('the theme toggle switches between dark and light', async ({ page }) => {
  const before = await page.locator('html').getAttribute('data-theme');
  await page.getByRole('button', { name: /^Theme/ }).click(); // auto -> light
  await page.getByRole('button', { name: /^Theme/ }).click(); // light -> dark
  const after = await page.locator('html').getAttribute('data-theme');
  expect(['light', 'dark']).toContain(before);
  expect(after).toBe('dark');
});

test('each panel says what it is for', async ({ page }) => {
  await expect(page.locator('.pane').nth(0)).toContainText('Input data to test');
  await expect(page.locator('.pane').nth(1)).toContainText('Run mongosh commands');
  await expect(page.locator('.pane').nth(1)).toContainText('Vector Search');
  await expect(page.locator('.pane').nth(2)).toContainText('Output');
});

test('Run selection runs only the highlighted statements', async ({ page }) => {
  await editor(page, 1).click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.type('db.collection.countDocuments({})\ndb.collection.find({ name: "Ada" })');
  await expect(page.getByRole('button', { name: 'Run selection' })).toBeDisabled();
  // highlight the second line only
  await page.keyboard.press('Home');
  await page.keyboard.press('Shift+End');
  await expect(page.getByRole('button', { name: 'Run selection' })).toBeEnabled();
  await page.getByRole('button', { name: 'Run selection' }).click();
  await expect(page.locator('.pane').nth(2)).toContainText('1 statement');
  await expect(page.locator('.pane').nth(2)).toContainText('selection');
  await expect(editor(page, 2)).not.toContainText('countDocuments');
  // Run all runs both
  await page.getByRole('button', { name: 'Run all' }).click();
  await expect(page.locator('.pane').nth(2)).toContainText('2 statements');
});

test('a panel collapses to a rail and expands again', async ({ page, isMobile }) => {
  test.skip(!!isMobile, 'tabs layout has no collapse');
  const data = page.locator('.pane').nth(0);
  await data.getByRole('button', { name: 'Collapse the Data panel' }).click();
  await expect(data.getByRole('button', { name: 'Expand the Data panel' })).toBeVisible();
  await expect(data.locator('.cm-content')).toHaveCount(0);
  await data.getByRole('button', { name: 'Expand the Data panel' }).click();
  await expect(editor(page, 0)).toContainText('documents: [');
});

test('?example= opens an example', async ({ page }) => {
  await page.goto('/?example=movies-explore');
  await expect(editor(page, 0)).toContainText('The Matrix');
  await run(page).click();
  await expect(editor(page, 2)).toContainText('avgRating');
});

test('Run lives only in the Shell panel, not in the toolbar', async ({ page }) => {
  await expect(page.locator('.toolbar').getByRole('button', { name: /^Run/ })).toHaveCount(0);
  await expect(page.locator('.pane').nth(1).getByRole('button', { name: 'Run all statements' })).toBeVisible();
});

test('each panel has its own background shade', async ({ page }) => {
  const shade = (i: number) => page.locator('.pane').nth(i).evaluate((e) => getComputedStyle(e).backgroundColor);
  const [a, b, c] = [await shade(0), await shade(1), await shade(2)];
  expect(new Set([a, b, c]).size).toBe(3);
});

test('the example picker says "Load an example…"', async ({ page }) => {
  const more = page.locator('.more-btn');
  if (await more.isVisible()) await more.click();
  await expect(page.getByLabel('Load an example').locator('option').first()).toHaveText('Load an example…');
});

test('a Search query warns that the first run is slower', async ({ page }) => {
  await expect(page.getByRole('note')).toHaveCount(0);
  await editor(page, 1).click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.type('db.collection.aggregate([{ $search: { text: { query: "x", path: "name" } } }])');
  await expect(page.getByRole('note')).toContainText('can take longer the first time');
});

test('a running query shows a spinner and the seconds elapsed', async ({ page }) => {
  await page.route('**/api/v1/run', async (route) => {
    await new Promise((r) => setTimeout(r, 1500));
    await route.continue();
  });
  await run(page).click();
  await expect(page.getByRole('status').filter({ hasText: /Running/ })).toBeVisible();
  await expect(page.getByRole('progressbar')).toBeVisible();
  await expect(editor(page, 2)).toContainText('name: "Linus"');
  await expect(page.getByRole('progressbar')).toHaveCount(0);
});

test('the prebuilt dataset shows read-only in the Data panel', async ({ page }) => {
  // Pretend sample data is on and the prebuilt dataset exists (the local test server has neither).
  const definition = {
    database: 'sample_playground',
    collections: [
      { name: 'movies', documents: [{ title: 'Alien', year: 1979 }], searchIndex: { name: 'default', definition: { mappings: { dynamic: true } } } },
      { name: 'items', documents: [{ name: 'red apple', embedding: [0.9, 0.1, 0] }], searchIndex: { name: 'vector_index', type: 'vectorSearch', definition: { fields: [{ type: 'vector', path: 'embedding', numDimensions: 3, similarity: 'cosine' }] } } },
    ],
  };
  await page.route('**/api/v1/limits', async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    await route.fulfill({ response: res, json: { ...body, sampleSupported: true, features: { ...body.features, sampleData: true } } });
  });
  await page.route('**/api/v1/examples', async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    const prebuilt = { id: 'prebuilt-atlas-search', title: 'Atlas Search on a prebuilt dataset', kind: 'example', script: 'db.movies.find()', datasource: { sample: 'sample_playground' } };
    await route.fulfill({ response: res, json: [...body, prebuilt] });
  });
  await page.route('**/api/v1/samples', (route) =>
    route.fulfill({ json: { available: true, databases: [{ name: 'sample_playground', collections: ['movies', 'items'], definition: true }] } }),
  );
  await page.route('**/api/v1/samples/sample_playground', (route) => route.fulfill({ json: definition }));

  await page.goto('/?example=prebuilt-atlas-search');
  const data = page.getByTestId('sample-data');
  await expect(data).toContainText('db = {');
  await expect(data).toContainText('title: "Alien"');
  await expect(data).toContainText('vectorSearchIndexes');
  // Read-only: the editor does not accept typing, and there are no Undo / Clear controls for it.
  await expect(data.locator('.cm-content')).toHaveAttribute('contenteditable', 'false');
  await expect(page.getByRole('button', { name: 'Clear the data panel' })).toHaveCount(0);
  await expect(page.locator('[data-pane="data"] .desc')).toContainText('read-only');
});
