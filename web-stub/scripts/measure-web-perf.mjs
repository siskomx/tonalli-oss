import assert from 'node:assert/strict';
import { cpus, freemem, release, totalmem } from 'node:os';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { packIdentity, root, sha256, startArticleHost } from '../tests/e2e/article-host.mjs';
import { verifiedReactPacks } from '../tests/e2e/react-fixtures.mjs';
import { nearestRank, scrollSummary } from './perf-math.mjs';
import { usable, baseline, reloadFirstRender, transition, scroll, installObservers } from './web-perf-steps.mjs';
import { preparePerfRun } from './perf-inputs.mjs';

const { values } = parseArgs({ options: { check: { type: 'boolean' }, output: { type: 'string' },
  web: { type: 'string' }, 'ui-sha256': { type: 'string' } }, allowPositionals: false });
if (!values.output) throw new Error('An explicit new --output path is required; existing receipts are never overwritten.');
const check = values.check ?? false;
const expectedCli = '5f0938f1c2d55819377a5d3984c9c85eda4dc68af17a60fa4ea087c81feb4031';
const expectedDist = values['ui-sha256'] || '1732a041581e334014ac8535a89d9bac84dc00d56034d2721cf0dcfc33681a37';
const output = resolve(root, values.output);
const frozenInput = await preparePerfRun({ output, webRoot: resolve(root, values.web || 'web-stub/dist'),
  artifactRoot: join(root, 'target/ui-perf-inputs'), expectedHash: expectedDist });
process.env.TONALLI_SPIKE_CLI ||= join(root, 'target/tools/ui-release-38c1bc9.exe');
assert.equal(await sha256(process.env.TONALLI_SPIKE_CLI), expectedCli);
const dataset = await verifiedReactPacks();
const titleIndexPath = join(root, 'target/full-wikipedia.titles.idx');
const titleIndexIdentity = await packIdentity(titleIndexPath);
const indexHashStarted = performance.now();
const titleIndexSha256 = await sha256(titleIndexPath);
const titleIndexHashMs = performance.now() - indexHashStarted;
assert.deepEqual(await packIdentity(titleIndexPath), titleIndexIdentity);
const result = { mode: check ? 'bounded harness positive controls; excluded from series' : 'ten predeclared paired trials',
  startedAt: new Date().toISOString(), host: { platform: process.platform, windows: release(), cpu: cpus()[0].model,
    logicalCpus: cpus().length, totalMemoryBytes: totalmem(), freeMemoryBytes: freemem() },
  cli: { file: process.env.TONALLI_SPIKE_CLI, sha256: expectedCli, mode: 'optimized release; default production features; no ping' },
  uiSha256: expectedDist, frozenInput, dataset: dataset.receipt,
  installedPacks: dataset.packs.map(({ id, file, version, bytes, sha256 }) => ({ id, file, version, bytes, sha256 })),
  article: { selectedTitlePosition: 99999, requestedUrl: 'C/17_December_2013_arrests_in_Turkey',
    canonicalUrl: 'C/2013_corruption_scandal_in_Turkey' },
  mapRegion: 'default selected ca/Montreal; actual mx/us/ca extracts registered',
  titleIndex: { identity: titleIndexIdentity, sha256: titleIndexSha256, setupHashMs: titleIndexHashMs },
  browser: { headed: true, viewport: { width: 1280, height: 800 }, dpr: 1,
    nativeWorkingArea: { width: 1707, height: 1019, source: 'System.Windows.Forms.Screen read before series' },
    args: ['--window-size=1320,980', '--window-position=0,0'] },
  cache: 'fresh profiles versus same-profile operations; OS caches untouched; no-store plus routing disables HTTP cache',
  scroll: { startIndex: 50160, endIndex: 50280, durationMs: 4000, pixelsPerSecond: 1200,
    boundary: '100000-row real virtual list; fixed 120-row subpath, not a sweep of all100000',
    gapMeaning: 'RAF-sampled committed-DOM missing-row coverage; not exact painted blank duration' }, attempts: [] };
result.harness = await Promise.all(['web-stub/scripts/measure-web-perf.mjs', 'web-stub/scripts/perf-math.mjs', 'web-stub/scripts/perf-inputs.mjs', 'web-stub/scripts/web-perf-steps.mjs',
  'web-stub/tests/e2e/article-host.mjs', 'web-stub/tests/e2e/react-fixtures.mjs'].map(async (file) => ({ file, sha256: await sha256(join(root, file)) })));

async function save() { await writeFile(output, `${JSON.stringify(result, null, 2)}\n`, 'utf8'); }
for (let trial = 1; trial <= (check ? 1 : 10); trial++) {
  const attempt = { trial, status: 'started', requests: [], timings: {}, freeMemoryBytes: freemem() };
  result.attempts.push(attempt);
  await save();
  let host;
  let deadline;
  let phase = 'setup';
  const requests = new WeakMap();
  try {
    assert.deepEqual(await packIdentity(titleIndexPath), titleIndexIdentity);
    host = await startArticleHost({ bootstrap: 'request', headless: false, webRoot: frozenInput.path,
      browserArgs: result.browser.args, viewport: result.browser.viewport,
      extraPacks: dataset.packs, titleIndexes: [{ id: 'wiki', file: 'target/full-wikipedia.titles.idx' }], startupTimeoutMs: 60000 });
    assert.equal(host.cliSha256, expectedCli); assert.equal(host.distSha256, expectedDist);
    deadline = setTimeout(() => { attempt.watchdog = '120-second trial deadline'; host.page.close().catch(() => {}); }, 120000);
    attempt.setup = host.setup; attempt.auth = host.baseline;
    result.authoredFixtures = host.manifest.provenance.packs.map(({ kind, path, bytes, sha256 }) => ({ id: kind, path, bytes, sha256 }));
    result.browser.path = host.browserPath; result.browser.version = host.context.browser().version();
    host.page.on('request', (request) => {
      const url = new URL(request.url());
      const record = { phase, path: url.pathname, offset: url.searchParams.get('offset'), at: Date.now(),
        resourceType: request.resourceType(), range: request.headers().range };
      attempt.requests.push(record); requests.set(request, record);
    });
    host.page.on('response', (response) => {
      const record = requests.get(response.request());
      if (record) { record.status = response.status(); record.cacheControl = response.headers()['cache-control'];
        record.contentLength = response.headers()['content-length']; }
    });
    host.page.on('requestfinished', (request) => {
      const record = requests.get(request);
      const timing = request.timing();
      if (record && timing.responseEnd >= timing.requestStart) record.networkMs = timing.responseEnd - timing.requestStart;
    });
    await installObservers(host.page);
    attempt.idleBaseline = await baseline(host.page);
    attempt.idleBaseline.summary = scrollSummary(attempt.idleBaseline.frames);
    assert.ok(attempt.idleBaseline.frames.every((frame) => frame.focused && frame.visibility === 'visible'), 'baseline not foreground');
    assert.equal(host.page.url(), 'about:blank', 'first browser document must be real library');
    phase = 'freshProfileFirstRender';
    await host.page.goto(`${host.origin}/library?pack=wiki`, { waitUntil: 'load' });
    await usable(host.page, '/library');
    attempt.timings.freshProfileFirstRender = await host.page.evaluate(() => ({ firstRenderMs: window.__perf.firstRenderMs,
      shellMs: window.__initialShellAt, readiness: window.__ready, focused: document.hasFocus(), visibility: document.visibilityState,
      foregroundEvents: window.__foregroundEvents,
      navigation: performance.getEntriesByType('navigation')[0].toJSON() }));
    for (const path of ['/api/session', '/api/zim/titles']) assert.ok(attempt.requests.some((request) => request.phase === phase && request.path === path && request.status === 200), `positive control ${path}`);
    phase = 'articleSelectionSetup';
    await host.page.getByTestId('library-list').evaluate((element) => { element.scrollTop = 99996 * 40; });
    await host.page.waitForFunction(() => document.querySelector('[data-testid="library-row"][data-index="99999"]')?.dataset.url === 'C/17_December_2013_arrests_in_Turkey');
    await usable(host.page, '/library');
    for (const [name, selector, route] of [
      ['articleFirstVisit', '[data-testid="library-row"][data-index="99999"]', '/article'],
      ['mapFirstVisit', '[data-testid="nav-map"]', '/map'],
      ['libraryReturn1', '[data-testid="nav-library"]', '/library'],
      ['articleRevisit', '[data-testid="library-row"][data-index="99999"]', '/article'],
      ['mapRevisit', '[data-testid="nav-map"]', '/map'],
      ['libraryReturn2', '[data-testid="nav-library"]', '/library'],
    ]) {
      if (name === 'articleRevisit') {
        phase = 'articleRevisitSelectionSetup';
        await host.page.getByTestId('library-list').evaluate((element) => { element.scrollTop = 99996 * 40; });
        await host.page.waitForFunction(() => document.querySelector('[data-testid="library-row"][data-index="99999"]')?.dataset.url === 'C/17_December_2013_arrests_in_Turkey');
        await usable(host.page, '/library');
      }
      phase = name; attempt.timings[name] = await transition(host, selector, route);
    }
    phase = 'sameProfileReloadFirstRender';
    attempt.timings.sameProfileReloadFirstRender = await reloadFirstRender(host.page);
    phase = 'scrollSetup';
    await host.page.getByTestId('library-list').evaluate((element) => { element.scrollTop = 50160 * 40; });
    await host.page.waitForFunction(() => window.__libraryCommit?.rows.some((row) => row.index === 50160 && row.loaded));
    await usable(host.page, '/library');
    const height = await host.page.getByTestId('library-list').evaluate((element) => element.clientHeight);
    attempt.scrollViewportHeight = height;
    let release = () => {};
    let held;
    if (check) {
      held = async (route) => { await new Promise((accept) => { release = accept; setTimeout(accept, 750); }); await route.continue(); };
      await host.page.route('**/api/zim/titles?*offset=50200&*', held);
    }
    phase = 'scrollUncached'; attempt.scrollUncached = await scroll(host, height);
    release();
    if (held) await host.page.unroute('**/api/zim/titles?*offset=50200&*', held);
    phase = 'scrollRepeatReset';
    await host.page.getByTestId('library-list').evaluate((element) => { element.scrollTop = 50160 * 40; });
    await host.page.waitForFunction(() => window.__libraryCommit?.rows.some((row) => row.index === 50160 && row.loaded));
    phase = 'scrollRepeat'; attempt.scrollRepeat = await scroll(host, height);
    attempt.scrollRequests = Object.fromEntries(['scrollUncached', 'scrollRepeatReset', 'scrollRepeat'].map((name) =>
      [name, attempt.requests.filter((request) => request.phase === name && request.path === '/api/zim/titles')]));
    attempt.repeatCacheLabel = attempt.scrollRequests.scrollRepeat.length || attempt.scrollRequests.scrollRepeatReset.length
      ? 'repeat with observed title-page refetches' : 'repeat; zero observed title-page requests during reset/pass';
    assert.ok(attempt.scrollRequests.scrollUncached.some((request) => request.offset === '50200' && request.status === 200), 'uncached pass must actually request next page');
    if (check) assert.ok(attempt.scrollUncached.visibleGapMs > 0 && attempt.scrollUncached.maxMissingRows > 0, 'held real page must produce visible-gap positive control');
    for (const pass of [attempt.scrollUncached, attempt.scrollRepeat]) assert.ok(pass.samples.every((sample) => sample.focused && sample.visibility === 'visible'), 'scroll lost foreground');
    for (const timing of Object.values(attempt.timings)) {
      assert.ok(Number.isFinite(timing.firstRenderMs ?? timing.dataReadyMs) && (timing.firstRenderMs ?? timing.dataReadyMs) > 0);
      assert.ok(timing.focused && timing.visibility === 'visible', 'route sample not foreground');
      assert.ok(timing.foregroundEvents.every((event) => event.focused && event.visibility === 'visible'), 'route transiently lost foreground');
    }
    assert.equal(host.blocked.length, 0); attempt.externalAttempts = host.blocked;
    attempt.status = 'complete';
  } catch (error) { attempt.status = 'failed'; attempt.failedPhase = phase; attempt.error = error.message; }
  finally {
    clearTimeout(deadline);
    try { if (host) await host.close(); }
    catch (error) { attempt.cleanupError = error.message; attempt.status = 'failed'; }
    try { assert.deepEqual(await packIdentity(titleIndexPath), titleIndexIdentity); }
    catch (error) { attempt.identityError = error.message; attempt.status = 'failed'; }
    finally { await save(); }
  }
  console.log(`trial ${trial}: ${attempt.status}${attempt.error ? ` ${attempt.error}` : ''}`);
}

result.completeTrials = result.attempts.filter((attempt) => attempt.status === 'complete').length;
result.failedTrials = result.attempts.length - result.completeTrials;
result.summary = {};
for (const key of ['freshProfileFirstRender', 'sameProfileReloadFirstRender', 'articleFirstVisit', 'mapFirstVisit', 'articleRevisit', 'mapRevisit']) {
  const values = result.attempts.filter((attempt) => attempt.status === 'complete').map((attempt) => attempt.timings[key].firstRenderMs ?? attempt.timings[key].dataReadyMs);
  if (values.length) result.summary[key] = { count: values.length, p50: nearestRank(values, .5), p95: nearestRank(values, .95) };
}
for (const key of ['scrollUncached', 'scrollRepeat']) {
  const passes = result.attempts.filter((attempt) => attempt.status === 'complete').map((attempt) => attempt[key]);
  if (passes.length) result.summary[key] = { count: passes.length, fpsP50: nearestRank(passes.map((pass) => pass.fps), .5),
    fpsP95: nearestRank(passes.map((pass) => pass.fps), .95), fpsMin: Math.min(...passes.map((pass) => pass.fps)),
    frameTimeP95: nearestRank(passes.flatMap((pass) => pass.samples.slice(1).map((sample, index) => sample.at - pass.samples[index].at)), .95),
    visibleGapMsTotal: passes.reduce((sum, pass) => sum + pass.visibleGapMs, 0) };
}
result.finishedAt = new Date().toISOString(); result.host.freeMemoryBytesEnd = freemem(); await save();
console.log(JSON.stringify(result.summary, null, 2));
if (result.attempts.some((attempt) => attempt.status !== 'complete')) process.exitCode = 1;
