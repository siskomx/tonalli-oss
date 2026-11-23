import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { redact, root, startArticleHost } from './article-host.mjs';

const mx = { id: 'mx', file: 'docs/spike/packs/mx-mexico-city-z8.pmtiles', bytes: 579622,
  sha256: '9a8d7ef3570d5cebd4e3ad24dd25ea88b22e578520dd19fbb9058c0248ab5238' };

async function ready(page, route) {
  await page.waitForFunction((route) => window.__ready?.route === route
    && (window.__ready.ready || window.__ready.error), route, { timeout: 20000 });
  const value = await page.evaluate(() => window.__ready);
  assert.equal(value.error, null);
  assert.equal(value.ready, true);
  return value;
}

async function withHeldModule(page, name, operation) {
  let release, entered, finished;
  let triggered = false;
  const hold = new Promise((resolve) => { release = resolve; });
  const waiting = new Promise((resolve) => { entered = resolve; });
  const continued = new Promise((resolve) => { finished = resolve; });
  const pattern = new RegExp(`/assets/${name}-[^/]+\\.js$`);
  const handler = async (route) => {
    triggered = true;
    entered();
    try { await hold; await route.continue(); } finally { finished(); }
  };
  await page.route(pattern, handler);
  try { await operation({ waiting, release }); }
  finally {
    release();
    if (triggered) await continued;
    await page.unroute(pattern, handler);
  }
}

test('departing article and map do not reopen while a different lazy route is pending', async () => {
  const host = await startArticleHost({ extraPacks: [mx] });
  const receipt = { distSha256: host.distSha256, cliSha256: host.cliSha256,
    browserVersion: host.context.browser().version(), cases: [] };
  let current;
  host.context.on('request', (request) => {
    current?.requests.push({ url: redact(request.url()), type: request.resourceType() });
  });
  try {
    await host.page.evaluate(() => sessionStorage.setItem('library-pack', 'benign'));
    await host.page.goto(`${host.origin}/article?${new URLSearchParams({ pack: 'benign', url: host.manifest.benign.entries.index, source: 'lifetime' })}`);
    const article = await ready(host.page, '/article');
    assert.equal(article.article.resourcesLoaded, true);
    const frame = host.page.frameLocator('#article-frame');
    assert.equal(await frame.locator('#image').evaluate((image) => image.naturalWidth), 2);
    assert.equal(await frame.locator('#external-css').evaluate((element) => getComputedStyle(element).color), 'rgb(17, 34, 51)');

    current = { from: 'article-query', target: 'same-article', requests: [] };
    receipt.cases.push(current);
    await host.page.getByTestId('nav-library').click();
    await host.page.getByRole('link', { name: 'Continue reading →' }).click();
    current.completed = await ready(host.page, '/article');
    current.departing = current.requests.filter(({ url }) => {
      const path = new URL(url).pathname;
      return path === '/api/zim/article' || path.startsWith('/zim/benign/');
    });

    current = { from: 'article', target: 'map', requests: [], positive: article };
    receipt.cases.push(current);
    await withHeldModule(host.page, 'Map', async ({ waiting, release }) => {
      await host.page.getByTestId('nav-map').click();
      await Promise.race([waiting, host.page.waitForTimeout(5000).then(() => { throw new Error('Map module was not requested'); })]);
      await host.page.waitForTimeout(350);
      current.held = await host.page.evaluate(() => ({ path: location.pathname, readiness: window.__ready }));
      current.departing = current.requests.filter(({ url }) => {
        const path = new URL(url).pathname;
        return path === '/api/zim/article' || path.startsWith('/zim/benign/');
      });
      assert.equal(current.held.path, '/map');
      assert.equal(current.held.readiness.ready, false);
      release();
      current.completed = await ready(host.page, '/map');
      assert.equal(current.completed.map.rendered, true);
    });

    current = { from: 'map', target: 'library', requests: [] };
    receipt.cases.push(current);
    await withHeldModule(host.page, 'Library', async ({ waiting, release }) => {
      await host.page.getByTestId('nav-library').click();
      await Promise.race([waiting, host.page.waitForTimeout(5000).then(() => { throw new Error('Library module was not requested'); })]);
      await host.page.waitForTimeout(350);
      current.held = await host.page.evaluate(() => ({ path: location.pathname, readiness: window.__ready }));
      current.departing = current.requests.filter(({ url }) => {
        const path = new URL(url).pathname;
        return path === '/api/maps' || path.startsWith('/assets/map/') || path.startsWith('/packs/');
      });
      assert.equal(current.held.path, '/library');
      assert.equal(current.held.readiness.ready, false);
      release();
      current.completed = await ready(host.page, '/library');
      assert.ok(current.completed.library.renderedRows > 0);
    });
    assert.deepEqual(host.blocked, []);
    assert.deepEqual(receipt.cases.map(({ from, departing }) => ({ from, departing })),
      [{ from: 'article-query', departing: [] }, { from: 'article', departing: [] }, { from: 'map', departing: [] }],
      'departing features must not reopen metadata/body/styles/tiles while the target module is held');
  } finally {
    await writeFile(process.env.TONALLI_LIFETIME_TRACE || join(root, 'docs/spike/departing-route-trace.json'),
      JSON.stringify(receipt, null, 2) + '\n', 'utf8');
    await host.close();
  }
});
