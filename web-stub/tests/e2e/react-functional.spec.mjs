import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { redact, root, startArticleHost } from './article-host.mjs';
import { verifiedReactPacks, wiki } from './react-fixtures.mjs';

let host;
let dataset;
let current;
const evidence = [];
const owners = new WeakMap();
const pending = new Set();
const branding = JSON.parse(await readFile(join(root, 'branding.json'), 'utf8'));
const catalogs = Object.fromEntries(await Promise.all(['en', 'es'].map(async (language) => [language,
  JSON.parse(await readFile(join(root, `web-stub/src/locales/${language}.json`), 'utf8'))])));

async function start(locale = 'en-US') {
  return startArticleHost({ locale, extraPacks: dataset.packs,
    titleIndexes: [{ id: 'wiki', file: 'target/full-wikipedia.titles.idx' }], startupTimeoutMs: 60000 });
}

test.before(async () => {
  dataset = await verifiedReactPacks();
  host = await start();
  host.context.on('request', (request) => {
    let owner = 'unknown';
    try { owner = request.frame() === host.page.mainFrame() ? 'parent' : 'article'; } catch {}
    owners.set(request, owner);
    current?.requests.push({ url: redact(request.url()), owner, type: request.resourceType() });
  });
  host.context.on('response', (response) => {
    const sample = current;
    if (!sample) return;
    sample.responses.push({ url: redact(response.url()), owner: owners.get(response.request()), status: response.status() });
    if (new URL(response.url()).pathname === '/api/zim/titles' && response.status() === 200) {
      const task = response.json().then((body) => {
        assert.ok(body.rows.length <= 200, 'real title pages stay bounded');
        sample.pages.push({ pack: body.pack, offset: body.offset, total: body.total, count: body.rows.length });
      }).catch((error) => { sample.apiTraceErrors.push(error.message); });
      pending.add(task);
      task.finally(() => pending.delete(task)).catch(() => {});
    }
  });
});

test.beforeEach((context) => {
  current = { name: context.name, requests: [], responses: [], pages: [], apiTraceErrors: [] };
  evidence.push(current);
});
test.afterEach(async () => {
  await Promise.all([...pending]);
  if (!host) return;
  assert.deepEqual(current.apiTraceErrors, []);
  assert.equal(host.blocked.length, 0, 'functional routes attempted no external egress');
  assert.deepEqual(current.responses.filter((response) => response.owner !== 'parent'
    && new URL(response.url).pathname.startsWith('/api/') && response.status >= 200 && response.status < 300), []);
});
test.after(async () => {
  if (!host) return;
  try {
    await writeFile(process.env.TONALLI_REACT_TRACE || join(root, 'docs/spike/react-functional-trace.json'),
      `${JSON.stringify({ browserPath: host.browserPath, browserVersion: host.context.browser().version(),
        cliSha256: host.cliSha256, distSha256: host.distSha256, baseline: host.baseline, dataset: dataset.receipt,
        cache: 'functional only; routing isolation disables HTTP cache; no performance series', cases: evidence }, null, 2)}\n`, 'utf8');
  } finally { await host.close(); }
});

async function navigate(path, route) {
  const response = await host.page.goto(`${host.origin}${path}`, { waitUntil: 'load' });
  assert.equal(response.status(), 200, `SPA route ${path} must load`);
  assert.ok(await host.page.evaluate(() => window.__ready), 'React entry must expose readiness before waiting for data');
  return readyRoute(route);
}

async function readyRoute(route) {
  await host.page.waitForFunction((name) => String(window.__ready?.route || '').replace(/^\/+/, '') === name
    && (window.__ready.ready || window.__ready.error), route, { timeout: 20000 });
  const ready = await host.page.evaluate(() => window.__ready);
  assert.equal(ready.error, null);
  assert.equal(ready.ready, true);
  return ready;
}

async function api(path, page = host.page) {
  return page.evaluate(async (path) => {
    const session = await fetch('/api/session', { credentials: 'same-origin', cache: 'no-store' });
    const { csrf } = await session.json();
    const response = await fetch(path, { credentials: 'same-origin', cache: 'no-store', headers: { 'X-Tonalli-Csrf': csrf } });
    return { status: response.status, body: await response.json() };
  }, path);
}

test('React library exposes genuine readiness, central branding and unmeasured FPS', async () => {
  const ready = await navigate('/library?pack=wiki', 'library');
  assert.equal(await host.page.getByTestId('app-name').first().textContent(), branding.displayName);
  assert.equal(ready.library.total, wiki.titleCount);
  assert.equal(ready.library.visibleLimit, 100000);
  assert.ok(ready.library.loadedRows > 0);
  const metrics = await host.page.evaluate(() => window.__perf);
  for (const key of ['bundleKb', 'firstRenderMs', 'routeSwitchMs', 'listScrollFps']) assert.ok(Object.hasOwn(metrics, key));
  const build = await host.page.evaluate(async () => {
    const response = await fetch('/assets/ui-build.json', { cache: 'no-store' });
    if (!response.ok) throw new Error(`build metadata status ${response.status}`);
    return response.json();
  });
  assert.equal(metrics.bundleKb, (build.initialJsBytes + build.initialCssBytes) / 1024);
  assert.ok(Number.isFinite(metrics.firstRenderMs) && metrics.firstRenderMs > 0);
  assert.equal(metrics.listScrollFps, null, 'Step2 must not invent a measured FPS value');
  current.build = build;
  current.readiness = ready;
});

test('a held lazy map module cannot finish readiness for the previous library', async () => {
  let release;
  let entered;
  let finished;
  let triggered = false;
  const hold = new Promise((accept) => { release = accept; });
  const waiting = new Promise((accept) => { entered = accept; });
  const continued = new Promise((accept) => { finished = accept; });
  const pattern = /\/assets\/Map-[^/]+\.js$/;
  const handler = async (route) => {
    triggered = true;
    entered();
    try { await hold; await route.continue(); } finally { finished(); }
  };
  await host.page.route(pattern, handler);
  await host.page.evaluate(() => sessionStorage.setItem('test-map-race', 'armed'));
  await host.page.addInitScript(() => {
    if (sessionStorage.getItem('test-map-race') !== 'armed') return;
    sessionStorage.removeItem('test-map-race');
    const observer = new MutationObserver(() => {
      if (!document.querySelector('[data-testid="library-row"]')) return;
      const link = document.querySelector('[data-testid="nav-map"]');
      if (!link) return;
      observer.disconnect();
      window.__mapRace = { before: structuredClone(window.__ready) };
      link.click();
    });
    observer.observe(document, { childList: true, subtree: true });
  });
  try {
    await host.page.goto(`${host.origin}/library?pack=wiki`, { waitUntil: 'domcontentloaded' });
    await Promise.race([waiting, new Promise((_, reject) => setTimeout(() => reject(new Error('real Map module request never started')), 5000))]);
    // Let the previous route's two-RAF completion run while the target module stays unavailable.
    await host.page.waitForTimeout(350);
    const held = await host.page.evaluate(() => ({ path: location.pathname, ready: window.__ready,
      metrics: window.__perf, before: window.__mapRace.before,
      mapPresent: Boolean(document.querySelector('[data-testid="map-renderer"]')) }));
    current.heldMap = held;
    assert.equal(held.path, '/map');
    assert.equal(held.before.ready, false, 'navigation starts before initial library readiness');
    assert.equal(held.mapPresent, false);
    assert.equal(held.ready.ready, false, 'retained library cannot complete the pending map route');
    assert.equal(held.metrics.routeSwitchMs, null, 'pending lazy target has no completed route-switch timing');
    release();
    const completed = await readyRoute('map');
    assert.equal(completed.map.rendered, true);
    current.completedMap = completed;
  } finally {
    release();
    if (triggered) await continued;
    await host.page.unroute(pattern, handler);
    await host.page.evaluate(() => sessionStorage.removeItem('test-map-race'));
  }
});

test('virtual library renders real archive rows near zero, fifty thousand and ninety-nine thousand', async () => {
  await navigate('/library?pack=wiki', 'library');
  const list = host.page.getByTestId('library-list');
  for (const index of [0, 50000, 99999]) {
    const offset = Math.floor(index / 200) * 200;
    const response = await api(`/api/zim/titles?pack=wiki&offset=${offset}&limit=200`);
    assert.equal(response.status, 200);
    assert.equal(response.body.total, wiki.titleCount);
    assert.equal(response.body.rows.length, 200);
    const expected = response.body.rows[index - offset];
    await list.evaluate((element, index) => {
      const height = Number(element.dataset.rowHeight);
      if (!(height > 0)) throw new Error('library row height is required for deterministic functional jumps');
      element.scrollTop = Math.max(0, index - 3) * height;
    }, index);
    const row = host.page.locator(`[data-testid="library-row"][data-index="${index}"]`);
    await row.waitFor({ state: 'visible' });
    await host.page.waitForFunction(({ index, url }) => document.querySelector(`[data-testid="library-row"][data-index="${index}"]`)?.dataset.url === url,
      { index, url: expected.url });
    assert.equal(await row.getAttribute('data-url'), expected.url);
    assert.ok((await row.textContent()).includes(expected.title));
    const rendered = await host.page.getByTestId('library-row').count();
    assert.ok(rendered > 0 && rendered <= 200, `virtual DOM bound: ${rendered}`);
    current[`row${index}`] = { expected, rendered };
  }
});

test('loading real title data cannot be reported as a ready list', async () => {
  let release;
  let entered;
  const hold = new Promise((accept) => { release = accept; });
  const waiting = new Promise((accept) => { entered = accept; });
  const pattern = '**/api/zim/titles?*';
  const handler = async (route) => { entered(); await hold; await route.continue(); };
  await host.page.route(pattern, handler);
  try {
    await host.page.goto(`${host.origin}/library?pack=wiki`, { waitUntil: 'domcontentloaded' });
    assert.ok(await host.page.evaluate(() => window.__ready), 'React readiness must exist while data is loading');
    await Promise.race([waiting, new Promise((_, reject) => setTimeout(() => reject(new Error('real title request never started')), 5000))]);
    assert.equal(await host.page.evaluate(() => window.__ready.ready), false);
    release();
    await host.page.waitForFunction(() => String(window.__ready?.route || '').replace(/^\/+/, '') === 'library' && window.__ready.ready);
    assert.ok(await host.page.getByTestId('library-row').count() > 0);
  } finally { release(); await host.page.unroute(pattern, handler); }
});

test('a real high archive row opens as sandboxed HTML or records the Earth fallback', async () => {
  const page = await api('/api/zim/titles?pack=wiki&offset=99800&limit=200');
  assert.equal(page.status, 200);
  let selected = page.body.rows[199].url;
  let info = await api(`/api/zim/article?pack=wiki&url=${encodeURIComponent(selected)}`);
  assert.equal(info.status, 200);
  const candidate = { url: selected, mime: info.body.mime_type };
  if (!/^(text\/html|application\/xhtml\+xml)(;|$)/i.test(info.body.mime_type)) {
    selected = 'C/Earth';
    info = await api(`/api/zim/article?pack=wiki&url=${encodeURIComponent(selected)}`);
    assert.equal(info.status, 200);
  }
  assert.match(info.body.mime_type, /^(text\/html|application\/xhtml\+xml)(;|$)/i);
  let ready;
  if (selected === candidate.url) {
    await navigate('/library?pack=wiki', 'library');
    await host.page.getByTestId('library-list').evaluate((element) => { element.scrollTop = 99996 * Number(element.dataset.rowHeight); });
    const row = host.page.locator('[data-testid="library-row"][data-index="99999"]');
    await host.page.waitForFunction((url) => document.querySelector('[data-testid="library-row"][data-index="99999"]')?.dataset.url === url, selected);
    await row.click();
    ready = await readyRoute('article');
  } else ready = await navigate(`/article?${new URLSearchParams({ pack: 'wiki', url: selected })}`, 'article');
  assert.equal(ready.article.resourcesLoaded, true);
  const frame = host.page.locator('#article-frame');
  assert.equal(await frame.getAttribute('sandbox'), 'allow-same-origin');
  const src = new URL(await frame.getAttribute('src'), host.origin);
  assert.equal(src.origin, host.origin);
  assert.equal(decodeURIComponent(src.pathname), `/zim/wiki/${info.body.url}`);
  current.article = { candidate, selected, canonical: info.body.url, fallback: selected !== candidate.url };
});

test('React map waits for real installed-map rendering and all route links work', async () => {
  const maps = await api('/api/maps');
  assert.equal(maps.status, 200);
  assert.deepEqual(maps.body.packs.map((pack) => pack.id).sort(), ['ca', 'mx', 'us']);
  await navigate('/library?pack=wiki', 'library');
  await host.page.getByTestId('nav-map').click();
  const ready = await readyRoute('map');
  assert.equal(ready.map.rendered, true);
  const canvas = host.page.locator('canvas').first();
  await canvas.waitFor({ state: 'visible' });
  assert.ok(await canvas.evaluate((element) => element.width > 0 && element.height > 0));
  for (const name of ['library', 'map', 'settings']) assert.equal(await host.page.getByTestId(`nav-${name}`).count(), 1);
  assert.equal(await host.page.getByTestId('nav-article').count(), 0);
  await host.page.getByTestId('nav-library').click();
  await host.page.waitForFunction(() => String(window.__ready?.route || '').replace(/^\/+/, '') === 'library' && window.__ready.ready);
  assert.equal(new URL(host.page.url()).pathname, '/library');
  const routeSwitchMs = await host.page.evaluate(() => window.__perf.routeSwitchMs);
  assert.ok(Number.isFinite(routeSwitchMs), 'a real route switch must finish its timing hook');
  await host.page.getByTestId('nav-library').click();
  assert.equal(await host.page.evaluate(() => window.__perf.routeSwitchMs), routeSwitchMs,
    'clicking the current route must preserve its last completed timing');
  current.map = ready.map;
});

test('modified navigation clicks preserve the current tab readiness and timing', async () => {
  await navigate('/library?pack=wiki', 'library');
  current.modifiedClicks = [];
  for (const selector of ['[data-testid="nav-map"]', '[data-testid="library-row"]']) {
    const modified = await host.page.evaluate(async (selector) => {
      const before = { path: location.pathname + location.search, ready: structuredClone(window.__ready),
        routeSwitchMs: window.__perf.routeSwitchMs };
      // React handles the event on its root first. Cancel only the native new-tab
      // default afterward, so this check needs no popup or synthetic navigation.
      document.addEventListener('click', (event) => event.preventDefault(), { once: true });
      document.querySelector(selector).dispatchEvent(new MouseEvent('click',
        { bubbles: true, cancelable: true, button: 0, ctrlKey: true }));
      await new Promise((resolve) => setTimeout(resolve, 150));
      return { selector, before, after: { path: location.pathname + location.search, ready: window.__ready,
        routeSwitchMs: window.__perf.routeSwitchMs } };
    }, selector);
    current.modifiedClicks.push(modified);
    assert.equal(modified.after.path, modified.before.path);
    assert.equal(modified.after.ready.ready, true, 'opening another tab cannot strand the current route pending');
    assert.equal(modified.after.ready.route, modified.before.ready.route);
    assert.equal(modified.after.routeSwitchMs, modified.before.routeSwitchMs);
  }
});

test('React article preserves the hostile-page sandbox and trusted parent', async () => {
  await navigate(`/article?${new URLSearchParams({ pack: 'hostile', url: 'C/hostile/passive.html' })}`, 'article');
  assert.equal(await host.page.locator('#article-frame').getAttribute('sandbox'), 'allow-same-origin');
  assert.equal(await host.page.evaluate(() => Boolean(window.articleScriptRan)), false);
  assert.equal(await host.page.locator('body').getAttribute('data-compromised'), null);
});

for (const [locale, effective] of [['es-MX', 'es'], ['fr-FR', 'en']]) {
  test(`device locale ${locale} selects ${effective} catalogs with central branding`, async () => {
    const localized = await start(locale);
    try {
      await localized.page.goto(`${localized.origin}/library?pack=wiki`, { waitUntil: 'load' });
      assert.ok(await localized.page.evaluate(() => window.__ready), 'React readiness missing in localized app');
      await localized.page.waitForFunction(() => String(window.__ready?.route || '').replace(/^\/+/, '') === 'library' && window.__ready.ready);
      assert.equal(await localized.page.locator('html').getAttribute('lang'), effective);
      assert.equal(await localized.page.getByTestId('app-name').first().textContent(), branding.displayName);
      for (const route of ['library', 'map', 'settings']) {
        assert.equal((await localized.page.getByTestId(`nav-${route}`).textContent()).trim(), catalogs[effective][`nav.${route}`]);
      }
      assert.equal(localized.blocked.length, 0);
      current.locale = { locale, effective, cliSha256: localized.cliSha256, distSha256: localized.distSha256 };
    } finally { await localized.close(); }
  });
}
