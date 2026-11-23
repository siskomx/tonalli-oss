import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { createServer } from 'vite';
import { chromium } from 'playwright-core';

// Same owned Vite/Playwright fixture pattern as chat-ui.spec.mjs; no real packs/models.
let vite, browser, origin;
test.before(async () => {
  vite = await createServer({ root: fileURLToPath(new URL('../../', import.meta.url)), logLevel: 'error', server: { host: '127.0.0.1', port: 0 } });
  await vite.listen(); origin = `http://127.0.0.1:${vite.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.TONALLI_ARTICLE_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', headless: true });
});
test.after(async () => { await browser?.close(); await vite?.close(); });
async function fixture(operation, { empty = false, locale = 'en-US' } = {}) {
  const context = await browser.newContext({ locale }); const page = await context.newPage(); page.setDefaultTimeout(5000);
  const calls = [], pending = []; let releaseOld;
  await page.route('**/assets/ui-build.json', route => route.fulfill({ json: { initialJsBytes: 1, initialCssBytes: 1 } }));
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url()); calls.push(url.pathname + url.search);
    assert.equal(route.request().headers()['x-tonalli-csrf'] || url.pathname === '/api/session', url.pathname === '/api/session' ? true : 'reader-csrf');
    if (url.pathname === '/api/session') return route.fulfill({ json: { csrf: 'reader-csrf', private_ai: false, private_notes: false } });
    if (url.pathname === '/api/packs') return route.fulfill({ json: { packs: empty ? [] : [{ id: 'first', title: 'First encyclopedia', total: 100001 }, { id: 'second', title: 'Second encyclopedia', total: 2 }] } });
    if (url.pathname === '/api/zim/titles') {
      const pack = url.searchParams.get('pack'), offset = Number(url.searchParams.get('offset')), total = pack === 'second' ? 2 : 100001;
      return route.fulfill({ json: { pack, offset, total, rows: Array.from({ length: Math.min(200, total - offset) }, (_, i) => ({ title: `${pack} title ${offset + i}`, url: `C/${pack}_${offset + i}` })) } });
    }
    if (url.pathname === '/api/zim/suggest') {
      const q = url.searchParams.get('q'), pack = url.searchParams.get('pack');
      if (q === 'old') { const task = new Promise(resolve => { releaseOld = resolve; }).then(() => route.fulfill({ json: { pack, hits: [{ title: 'Stale result', url: 'C/Stale' }] } })).catch(() => {}); pending.push(task); return task; }
      return route.fulfill({ json: { pack, hits: q === 'none' ? [] : [{ title: `${pack} canonical result`, url: `C/${pack}_canonical` }] } });
    }
    return route.fulfill({ status: 404 });
  });
  try { await page.goto(origin + '/'); await operation({ page, calls, release: () => releaseOld?.() }); }
  finally { releaseOld?.(); await context.close(); await Promise.allSettled(pending); }
}

test('ordinary root opens Library, chooses installed packs and reaches titles beyond100k', async () => {
  await fixture(async ({ page, calls }) => {
    await page.getByTestId('library-row').first().waitFor();
    assert.equal(new URL(page.url()).pathname, '/library');
    await page.getByLabel('Installed content', { exact: true }).selectOption('second');
    await page.getByText('second title 0', { exact: true }).waitFor();
    await page.getByLabel('Installed content', { exact: true }).selectOption('first');
    await page.getByLabel('Title number', { exact: true }).fill('99999');
    await page.getByRole('button', { name: 'Go to title', exact: true }).click();
    await page.getByText('first title 99998', { exact: true }).waitFor();
    const jumped = await page.getByText('first title 99998', { exact: true }).boundingBox();
    const viewport = await page.getByTestId('library-list').boundingBox();
    assert.ok(jumped.y >= viewport.y && jumped.y < viewport.y + viewport.height, 'requested non-boundary title must be visible');
    // A second jump within that same browse window must also change scroll position.
    await page.getByLabel('Title number', { exact: true }).fill('99001');
    await page.getByRole('button', { name: 'Go to title', exact: true }).click();
    await page.getByText('first title 99000', { exact: true }).waitFor();
    await page.getByLabel('Title number', { exact: true }).fill('100001');
    await page.getByRole('button', { name: 'Go to title', exact: true }).click();
    await page.getByText('first title 100000', { exact: true }).waitFor();
    const commit = await page.evaluate(() => window.__libraryCommit);
    assert.equal(commit.pack, 'first');
    assert.equal(commit.visibleLimit, 1);
    assert.ok(commit.rows.some(row => row.loaded === true));
    assert.ok(calls.some(url => url.includes('offset=100000')));
    assert.ok(await page.locator('[data-testid="library-list"] > div').evaluate(node => node.offsetHeight <= 400000));
    assert.equal(await page.getByTestId('nav-notes').count(), 0);
  });
});
test('title search rejects stale results after query and pack changes, preserves canonical links', async () => {
  await fixture(async ({ page, release }) => {
    const query = page.getByLabel('Search article titles', { exact: true });
    await query.fill('old'); await page.waitForRequest(request => new URL(request.url()).searchParams.get('q') === 'old');
    await query.fill('new'); await page.getByText('first canonical result', { exact: true }).waitFor();
    release();
    assert.equal(await page.getByText('Stale result', { exact: true }).count(), 0);
    const link = page.getByRole('link', { name: 'first canonical result', exact: true });
    assert.equal(new URL(await link.getAttribute('href'), origin).searchParams.get('url'), 'C/first_canonical');
    await page.getByLabel('Installed content', { exact: true }).selectOption('second');
    assert.equal(await query.inputValue(), '');
    assert.equal(await page.getByText('first canonical result', { exact: true }).count(), 0);
    await query.fill('none'); await page.getByText('No matching article titles.', { exact: true }).waitFor();
  });
});
test('empty Spanish library guides native Open content without setup IDs or title calls', async () => {
  await fixture(async ({ page, calls }) => {
    await page.getByText('No hay contenido instalado.', { exact: true }).waitFor();
    assert.ok((await page.locator('main').innerText()).includes('Abrir contenido'));
    assert.equal(calls.filter(url => url.startsWith('/api/zim/')).length, 0);
    assert.equal(await page.getByTestId('nav-notes').count(), 0);
  }, { empty: true, locale: 'es-MX' });
});
