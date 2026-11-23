// Measures the already-running owned Tauri WebView2; never launches Edge.
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { parseArgs } from 'node:util';
import { chromium } from 'playwright-core';
import { usable, baseline, reloadFirstRender, transition, scroll, installObservers } from './web-perf-steps.mjs';
import { scrollSummary } from './perf-math.mjs';

const { values } = parseArgs({ options: { endpoint: { type: 'string' }, output: { type: 'string' }, pack: { type: 'string' } } });
assert.ok(values.endpoint && values.output && values.pack, '--endpoint, --output and --pack required');
assert.equal(new URL(values.endpoint).hostname, '127.0.0.1');
const result = { label: 'desktop', runtime: 'actual Tauri WebView2 through owned CDP',
  cache: 'fresh profile/process; OS caches untouched; same-profile reload and revisits separate', checks: [], timings: {}, errors: [] };
const browser = await chromium.connectOverCDP(values.endpoint);
let currentPhase = 'attach';
const phase = async name => { currentPhase = name; await writeFile(`${values.output}.phase`, name, 'utf8'); };
try {
  let pages = [];
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    pages = browser.contexts().flatMap(context => context.pages()).filter(page => {
      try { return new URL(page.url()).hostname === '127.0.0.1'; } catch { return false; }
    });
    if (pages.length) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.equal(pages.length, 1, 'exactly one owned loopback page');
  const page = pages[0]; page.setDefaultTimeout(30000);
  await page.getByTestId('nav-library').waitFor();
  const origin = new URL(page.url()).origin;
  result.observedRequests = 0;
  result.nonLocalHttpOrigins = [];
  result.titleResponses = [];
  page.context().on('request', request => {
    const url = new URL(request.url());
    result.observedRequests++;
    if (['http:', 'https:'].includes(url.protocol) && url.origin !== origin) result.nonLocalHttpOrigins.push(url.origin);
  });
  page.context().on('response', response => {
    const url = new URL(response.url());
    if (url.pathname === '/api/zim/titles') result.titleResponses.push({ phase: currentPhase, offset: url.searchParams.get('offset'), status: response.status() });
  });
  result.uiBuild = await page.evaluate(async () => {
    const response = await fetch('/assets/ui-build.json', { cache: 'no-store' });
    if (!response.ok) throw new Error('UI identity unavailable');
    return response.json();
  });
  page.on('pageerror', error => result.errors.push(error.message));
  await installObservers(page);
  result.browserVersion = browser.version();
  await phase('baseline');
  result.idle = await baseline(page);
  assert.ok(result.idle.frames.every(frame => frame.focused && frame.visibility === 'visible'), 'foreground baseline');
  result.idle.summary = scrollSummary(result.idle.frames);
  async function libraryReady() {
    await page.waitForFunction(() => window.__ready?.route === '/library' && (window.__ready.ready || window.__ready.error));
    const ready = await page.evaluate(() => window.__ready);
    assert.equal(ready.error, null);
    assert.equal(ready.ready, true);
    assert.equal(ready.library.pack, values.pack);
    assert.ok(ready.library.renderedRows > 0);
    if (ready.library.total === 19191219) return usable(page, '/library');
    result.functionalFixtureOnly = true;
    return ready;
  }
  await phase('libraryFirst');
  await page.goto(`${origin}/library?${new URLSearchParams({pack: values.pack})}`);
  const library = await libraryReady();
  assert.ok(library.library.total > 0);
  result.library = library.library;
  result.timings.firstMeasuredLibraryNavigation = await page.evaluate(() => ({ firstRenderMs: window.__perf.firstRenderMs,
    navigation: performance.getEntriesByType('navigation')[0].toJSON(), readiness: window.__ready,
    foregroundEvents: window.__foregroundEvents, focused: document.hasFocus(), visibility: document.visibilityState }));
  await phase('libraryReload');
  if (!result.functionalFixtureOnly) result.timings.sameProfileReload = await reloadFirstRender(page);
  else {
    await page.reload({ waitUntil: 'load' });
    await libraryReady();
    result.timings.sameProfileReload = await page.evaluate(() => ({ firstRenderMs: window.__perf.firstRenderMs,
      readiness: window.__ready, focused: document.hasFocus(), visibility: document.visibilityState }));
  }
  const host = { page };
  if (library.library.total >= 100000) {
    await phase('scrollSetup');
    await page.getByTestId('library-list').evaluate(list => { list.scrollTop = 50160 * 40; });
    await page.waitForFunction(() => window.__libraryCommit?.rows.some(row => row.index === 50160 && row.loaded));
    const height = await page.getByTestId('library-list').evaluate(list => list.clientHeight);
    result.scrollViewportHeight = height;
    await phase('scrollUncached');
    result.scroll = await scroll(host, height, values.pack);
    await phase('scrollRepeatReset');
    await page.getByTestId('library-list').evaluate(list => { list.scrollTop = 50160 * 40; });
    await page.waitForFunction(() => window.__libraryCommit?.rows.some(row => row.index === 50160 && row.loaded));
    await phase('scrollRepeat');
    result.scrollRepeat = await scroll(host, height, values.pack);
    assert.ok(result.titleResponses.some(row => row.phase === 'scrollUncached' && row.offset === '50200' && row.status === 200));
    for (const pass of [result.scroll, result.scrollRepeat]) assert.ok(pass.samples.every(sample => sample.focused && sample.visibility === 'visible'), 'foreground scroll');
  }
  else result.scroll = { notCovered: '100k-row performance dataset not installed; functional fixture only' };
  const selectedIndex = library.library.visibleLimit >= 100000 ? 99999 : 0;
  await page.getByTestId('library-list').evaluate((list, index) => { list.scrollTop = Math.max(0, index - 3) * 40; }, selectedIndex);
  const row = `[data-testid="library-row"][data-index="${selectedIndex}"]`;
  await page.locator(row).waitFor();
  result.selectedArticle = { index: selectedIndex, url: await page.locator(row).getAttribute('data-url') };
  await phase('articleFirst');
  result.timings.articleFirst = await transition(host, row, '/article');
  const iframe = page.frameLocator('#article-frame');
  assert.ok((await iframe.locator('body').innerText()).length > 20);
  result.checks.push('actual sandboxed article body');
  await phase('mapFirst');
  result.timings.mapFirst = await transition(host, '[data-testid="nav-map"]', '/map');
  assert.equal(result.timings.mapFirst.readiness.map.rendered, true);
  assert.ok(result.timings.mapFirst.readiness.map.labels.length > 0);
  result.checks.push('actual map renderer with labels');
  await page.getByTestId('nav-library').click();
  await page.waitForFunction(() => window.__ready?.route === '/library' && window.__ready.ready);
  await phase('articleRevisit');
  result.timings.articleRevisit = await transition(host, 'a.reading-return[href^="/article?"]', '/article');
  await phase('mapRevisit');
  result.timings.mapRevisit = await transition(host, '[data-testid="nav-map"]', '/map');
  for (const timing of Object.values(result.timings)) {
    assert.equal(timing.focused, true, 'foreground observation');
    assert.equal(timing.visibility, 'visible', 'visible observation');
    if (timing.foregroundEvents) assert.ok(timing.foregroundEvents.every(event => event.focused && event.visibility === 'visible'), 'route transiently lost foreground');
  }
  await page.screenshot({ path: values.output.replace(/\.json$/, '-map.png') });
  assert.deepEqual(result.errors, []);
  assert.ok(result.observedRequests > 0);
  assert.deepEqual(result.nonLocalHttpOrigins, [], 'app resources must remain offline/loopback');
  result.complete = true;
} catch (error) {
  result.complete = false;
  result.failure = error.message;
  process.exitCode = 1;
} finally {
  await writeFile(values.output, `${JSON.stringify(result, null, 2)}\n`, { flag: 'wx' });
  await browser.close();
}
