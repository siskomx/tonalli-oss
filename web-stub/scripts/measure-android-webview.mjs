import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { mkdir, open, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { chromium } from 'playwright-core';
import { AndroidWebView, selectAppPage, qualifyFreshEntry, observeFreshEntryDocuments, parseMeasurementDataset, qualifyMeasurementTitle } from './android-webview-adapter.mjs';
import { nearestRank, scrollSummary } from './perf-math.mjs';
import { usable, baseline, reloadFirstRender, transition, scroll, installObservers } from './web-perf-steps.mjs';

const { values } = parseArgs({ options: { serial: { type: 'string' }, adb: { type: 'string' }, output: { type: 'string' },
  'apk-sha256': { type: 'string' }, 'ui-id': { type: 'string' }, 'dataset-manifest': { type: 'string' },
  'dataset-id': { type: 'string' }, 'expected-title-count': { type: 'string' }, 'fresh-pairs': { type: 'boolean' }, check: { type: 'boolean' } } });
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const safeHash = (value) => /^[a-f0-9]{64}$/.test(value || '');
if (!values.output || !values.serial || !safeHash(values['apk-sha256']) || !safeHash(values['ui-id'])
  || !values['dataset-manifest'] || !/^[a-z][a-z0-9_-]{0,31}$/.test(values['dataset-id'] || '')) {
  throw new Error('Require --serial, new --output, --apk-sha256, --ui-id, --dataset-id and --dataset-manifest.');
}
const output = path.resolve(values.output);
await mkdir(path.dirname(output), { recursive: true });
const reservation = await open(output, 'wx');
await reservation.writeFile('{"status":"reserved"}\n');
await reservation.close();
const result = { status: 'started', startedAt: new Date().toISOString(), platform: 'actual Android WebView only when executed on a device',
  mode: values.check ? 'one harness/capability attempt; not an acceptance series' : 'ten predeclared attempts; failures retained',
  cache: values['fresh-pairs'] ? 'fresh debug WebView suffix per pair; same-profile reload/revisits; OS file caches untouched'
    : 'existing WebView profile throughout; reload/revisits only; no fresh-profile or OS-cold claim',
  startupBoundary: 'document navigation-relative UI clock; native setup/launch handoff is not whole-app startup; inspect actual redirect/timeOrigin behavior on device',
  visitDefinition: 'first/revisit means within an attempt; existing-profile mode may already have loaded all modules before the attempt',
  app: 'org.tonalli.spike', apkSha256: values['apk-sha256'], uiId: values['ui-id'], registryId: 'probe',
  memoryBoundary: 'app main-process PSS with WebView loaded; renderer PSS requires separate manual attribution; not whole-app gate',
  scroll: { startIndex: 50160, endIndex: 50280, durationMs: 4000, pixelsPerSecond: 1200,
    boundary: '100000-row real virtual list; fixed 120-row subpath, not a sweep of all 100000',
    gapMeaning: 'RAF-sampled committed-DOM missing-row coverage; not exact painted blank duration' },
  capabilities: { workerCookies: 'manual inspection pending', article401503Faults: 'no automated fault-injection control in this driver',
    osCold: 'not measured; no purge performed' }, attempts: [] };
const save = () => writeFile(output, `${JSON.stringify(result, null, 2)}\n`, 'utf8');
try {
  if (process.env.DEBUG || process.env.PWDEBUG || process.env.ADB_TRACE) throw new Error('unsafe_debug_environment');
  const manifest = await readFile(values['dataset-manifest']);
  result.datasetManifestSha256 = hash(manifest);
  result.dataset = parseMeasurementDataset(manifest.toString('utf8'), { dataset: values['dataset-id'], expectedTitleCount: values['expected-title-count'] });
  const expectedTitleCount = result.dataset.expectedTitleCount;
  result.harness = await Promise.all(['measure-android-webview.mjs', 'android-webview-adapter.mjs', 'web-perf-steps.mjs', 'perf-math.mjs']
    .map(async (file) => ({ file, sha256: hash(await readFile(new URL(file, import.meta.url))) })));
  for (let trial = 1; trial <= (values.check ? 1 : 10); trial++) {
    const attempt = { trial, status: 'started', requests: [], timings: {}, externalAttempts: 0 };
    result.attempts.push(attempt);
    await save();
    const adb = new AndroidWebView({ serial: values.serial, ...(values.adb ? { adb: values.adb } : {}) });
    let browser, page, entrySession, stopEntryObservation, phase = 'installed_apk_identity';
    let networkHandler, held;
    const network = new WeakMap();
    try {
      assert.equal(await adb.installedApkHash(), result.apkSha256);
      const configuration = { profile: `m1-perf-${Date.now().toString(36)}-${randomBytes(4).toString('hex')}-${trial}`,
        dataset: values['dataset-id'], manifest: result.datasetManifestSha256 };
      if (values['fresh-pairs']) {
        phase = 'fresh_debug_profile_start';
        await adb.startFresh(configuration);
        attempt.profile = configuration.profile;
      }
      phase = 'owned_debug_socket';
      let endpoint;
      const deadline = Date.now() + 60000;
      while (!endpoint) {
        try { endpoint = await adb.forward(); }
        catch (error) {
          if (adb.port !== null || Date.now() >= deadline) throw error;
          await delay(1000);
        }
      }
      phase = 'native_dataset_binding';
      const expectedSession = { manifest: result.datasetManifestSha256, dataset: values['dataset-id'], ui: result.uiId,
        ...(values['fresh-pairs'] ? { profile: configuration.profile } : {}) };
      attempt.nativeSession = await adb.benchmarkSession(expectedSession);
      phase = 'cdp_attachment';
      browser = await chromium.connectOverCDP(endpoint, { noDefaults: true, timeout: 15000 });
      page = selectAppPage(browser.contexts().flatMap((context) => context.pages()), values['fresh-pairs']);
      page.setDefaultTimeout(20000);
      phase = 'ui_identity';
      await adb.assertUi(result.uiId);
      attempt.provider = browser.version();
      await installObservers(page);
      let networkOrigin = page.url() === 'about:blank' ? null : new URL(page.url()).origin;
      let collectingEntry = Boolean(values['fresh-pairs']);
      const entryEvents = [];
      if (collectingEntry) attempt.freshEntry = { qualified: false, documentSource: 'Page.frameNavigated',
        sameDocumentSource: 'Page.navigatedWithinDocument', events: entryEvents };
      const entryEvent = (event) => {
        if (!collectingEntry) return;
        if (entryEvents.length < 32) entryEvents.push(event);
        else attempt.freshEntry.limitExceeded = true;
      };
      if (collectingEntry) {
        phase = 'fresh_document_observer';
        // Playwright framenavigated also includes same-document History navigation.
        entrySession = await page.context().newCDPSession(page);
        stopEntryObservation = await observeFreshEntryDocuments(entrySession, entryEvent);
      }
      // Routing matches the desktop harness's disabled HTTP-cache condition.
      networkHandler = async (route) => {
        const url = new URL(route.request().url());
        if (networkOrigin === null && url.protocol === 'http:' && url.hostname === '127.0.0.1' && url.pathname === '/launch') networkOrigin = url.origin;
        if (url.origin === networkOrigin && url.protocol === 'http:') await route.continue();
        else { attempt.externalAttempts++; await route.fulfill({ status: 403, body: '' }); }
      };
      await page.route('**/*', networkHandler);
      page.on('request', (request) => {
        const url = new URL(request.url());
        // Never retain query strings, headers, POST data or launch credentials.
        const record = { phase, path: !['http:', 'https:'].includes(url.protocol) ? '[non-http-resource]'
          : url.pathname.startsWith('/api/notes') ? '/api/notes/[redacted]' : url.pathname,
          offset: /^\d+$/.test(url.searchParams.get('offset') || '') ? url.searchParams.get('offset') : null,
          resourceType: request.resourceType() };
        if (attempt.requests.length < 50000) { attempt.requests.push(record); network.set(request, record); }
        else attempt.requestLimitExceeded = true;
      });
      page.on('response', (response) => {
        const record = network.get(response.request());
        if (record) record.status = response.status();
        if (collectingEntry && response.request().isNavigationRequest()) {
          try {
            if (response.request().frame() === page.mainFrame()) entryEvent({ kind: 'response',
              path: new URL(response.url()).pathname, status: response.status(),
              mime: (response.headers()['content-type'] || '').split(';', 1)[0].trim().toLowerCase() });
          } catch { entryEvent({ kind: 'commit', path: '[unavailable-document]' }); }
        }
      });
      phase = 'idle_baseline';
      attempt.idleBaseline = await baseline(page);
      attempt.idleBaseline.summary = scrollSummary(attempt.idleBaseline.frames);
      assert.ok(attempt.idleBaseline.frames.every((f) => f.focused && f.visibility === 'visible'));
      phase = 'first_library_document';
      if (values['fresh-pairs']) {
        assert.equal(page.url(), 'about:blank');
        await adb.benchmarkAction('WEBVIEW_BENCHMARK_LOAD', configuration);
      } else {
        const origin = new URL(page.url()).origin;
        await page.goto(`${origin}/library?pack=probe`, { waitUntil: 'load' });
      }
      await usable(page, '/library', expectedTitleCount);
      if (values['fresh-pairs']) {
        if (attempt.freshEntry.limitExceeded) throw new Error('fresh_entry_not_unique');
        qualifyFreshEntry(entryEvents);
        attempt.freshEntry.qualified = true;
        collectingEntry = false;
      }
      attempt.timings[values['fresh-pairs'] ? 'freshProfileFirstRender' : 'existingProfileFirstRender'] = await page.evaluate(() => ({
        firstRenderMs: window.__perf.firstRenderMs, shellMs: window.__initialShellAt ?? null, readiness: window.__ready,
        focused: document.hasFocus(), visibility: document.visibilityState, foregroundEvents: window.__foregroundEvents }));
      for (const route of ['/api/session', '/api/zim/titles']) assert.ok(attempt.requests.some((r) => r.path === route && r.status === 200));
      phase = 'article_identity_setup';
      const title = await page.evaluate(async () => {
        const sessionResponse = await fetch('/api/session', { credentials: 'same-origin', cache: 'no-store', headers: { Accept: 'application/json' } });
        if (!sessionResponse.ok) throw new Error('title_identity_session_failed');
        const session = await sessionResponse.json();
        if (typeof session.csrf !== 'string' || !session.csrf) throw new Error('title_identity_session_failed');
        const response = await fetch('/api/zim/titles?offset=99999&limit=1&pack=probe', { credentials: 'same-origin', cache: 'no-store',
          headers: { Accept: 'application/json', 'X-Tonalli-Csrf': session.csrf } });
        if (!response.ok) throw new Error('title_identity_lookup_failed');
        return response.json();
      });
      const articleIdentity = qualifyMeasurementTitle(title, expectedTitleCount);
      if (result.articleIdentity) assert.deepEqual(articleIdentity, result.articleIdentity);
      else result.articleIdentity = articleIdentity;
      attempt.articleIdentitySetup = { ...articleIdentity,
        scope: 'bound title API lookup after first-library timing, excluded from article/route timings; warms title/OS caches; not OS-cold' };
      const host = { page };
      for (const [name, selector, route] of [
        ['articleFirstVisit', '[data-testid="library-row"][data-index="99999"]', '/article'],
        ['mapFirstVisit', '[data-testid="nav-map"]', '/map'],
        ['libraryReturn1', '[data-testid="nav-library"]', '/library'],
        ['articleRevisit', '[data-testid="library-row"][data-index="99999"]', '/article'],
        ['mapRevisit', '[data-testid="nav-map"]', '/map'],
        ['libraryReturn2', '[data-testid="nav-library"]', '/library'],
      ]) {
        phase = name;
        if (route === '/article') {
          await page.getByTestId('library-list').evaluate((element) => { element.scrollTop = 99996 * 40; });
          await page.waitForFunction((url) => document.querySelector('[data-testid="library-row"][data-index="99999"]')?.dataset.url === url, articleIdentity.url);
          await usable(page, '/library', expectedTitleCount);
        }
        attempt.timings[name] = await transition(host, selector, route, expectedTitleCount);
        if (route === '/article') {
          attempt.articleStatusCapability = await page.locator('#article-frame').evaluate((frame) => {
            const status = frame.contentWindow.performance.getEntriesByType('navigation')[0]?.responseStatus;
            return { responseStatus: Number.isInteger(status) ? status : null, valid2xx: Number.isInteger(status) && status >= 200 && status < 300,
              mime: frame.contentDocument.contentType };
          });
          assert.equal(attempt.articleStatusCapability.valid2xx, true);
        }
      }
      phase = 'same_profile_reload';
      attempt.timings.sameProfileReloadFirstRender = await reloadFirstRender(page, expectedTitleCount);
      phase = 'scroll_setup';
      await page.getByTestId('library-list').evaluate((element) => { element.scrollTop = 50160 * 40; });
      await page.waitForFunction(() => window.__libraryCommit?.rows.some((row) => row.index === 50160 && row.loaded));
      await usable(page, '/library', expectedTitleCount);
      const height = await page.getByTestId('library-list').evaluate((element) => element.clientHeight);
      attempt.scrollViewportHeight = height;
      if (values.check) {
        held = async (route) => { await delay(750); await route.continue(); };
        await page.route('**/api/zim/titles?*offset=50200&*', held);
      }
      phase = 'scrollUncached'; attempt.scrollUncached = await scroll(host, height, 'probe');
      if (held) {
        await page.unroute('**/api/zim/titles?*offset=50200&*', held);
        assert.ok(attempt.scrollUncached.visibleGapMs > 0 && attempt.scrollUncached.maxMissingRows > 0);
      }
      phase = 'scrollRepeatReset';
      await page.getByTestId('library-list').evaluate((element) => { element.scrollTop = 50160 * 40; });
      await page.waitForFunction(() => window.__libraryCommit?.rows.some((row) => row.index === 50160 && row.loaded));
      phase = 'scrollRepeat'; attempt.scrollRepeat = await scroll(host, height, 'probe');
      const nextPage = attempt.requests.filter((r) => r.phase === 'scrollUncached' && r.path === '/api/zim/titles' && r.offset === '50200' && r.status === 200);
      assert.ok(nextPage.length > 0);
      attempt.repeatCacheLabel = attempt.requests.some((r) => ['scrollRepeatReset', 'scrollRepeat'].includes(r.phase) && r.path === '/api/zim/titles')
        ? 'repeat with observed title-page refetches' : 'repeat; zero observed title-page requests during reset/pass';
      for (const pass of [attempt.scrollUncached, attempt.scrollRepeat]) assert.ok(pass.samples.every((s) => s.snapshotMatches && s.focused && s.visibility === 'visible'));
      for (const timing of Object.values(attempt.timings)) {
        assert.ok(Number.isFinite(timing.firstRenderMs ?? timing.dataReadyMs) && (timing.firstRenderMs ?? timing.dataReadyMs) > 0);
        assert.ok(timing.focused && timing.visibility === 'visible');
        assert.ok(timing.foregroundEvents.every((event) => event.focused && event.visibility === 'visible'));
      }
      assert.equal(attempt.externalAttempts, 0);
      assert.ok(!attempt.requestLimitExceeded);
      phase = 'app_pss_after_measurement';
      assert.deepEqual(await adb.benchmarkSession(expectedSession), attempt.nativeSession);
      attempt.memory = await adb.memory();
      attempt.status = 'complete';
    } catch (error) {
      attempt.status = 'failed_or_unsupported';
      attempt.failedPhase = phase;
      attempt.error = 'Capability or protocol check failed. No raw tool/browser error is retained; use the manual checklist.';
      if (attempt.freshEntry && /^fresh_entry_(preloaded|unexpected_response|not_unique)$/.test(error.message)) attempt.freshEntry.failure = error.message;
      if (page) {
        try {
          attempt.readinessCapability = await page.evaluate(() => ({ route: window.__ready?.route,
            ready: window.__ready?.ready === true, failed: Boolean(window.__ready?.error),
            libraryTotal: window.__ready?.library?.total ?? null, visibleLimit: window.__libraryCommit?.visibleLimit ?? null }));
        } catch { attempt.readinessCapability = 'unavailable'; }
      }
      if (phase.startsWith('article')) attempt.articleStatusCapability ??= { status: 'untested or unsupported; readiness remains fail-closed' };
    } finally {
      try { stopEntryObservation?.(); await entrySession?.detach(); }
      catch { attempt.entryObserverCleanup = 'unconfirmed'; }
      // connectOverCDP.close disconnects its transport in the pinned Playwright;
      // never close a remote Page/Context or send Browser.close over CDP.
      try {
        if (page && held) await page.unroute('**/api/zim/titles?*offset=50200&*', held);
        if (page && networkHandler) await page.unroute('**/*', networkHandler);
      } catch { attempt.routingCleanup = 'unconfirmed'; }
      try { await browser?.close(); } catch { attempt.disconnect = 'unconfirmed'; }
      try { await adb.close(); } catch { attempt.forwardCleanup = 'unconfirmed; inspect only the recorded owned local port'; attempt.ownedLocalPort = adb.port; }
      if (attempt.disconnect || attempt.routingCleanup || attempt.forwardCleanup || attempt.entryObserverCleanup) attempt.status = 'cleanup_unconfirmed';
      await save();
    }
  }
  result.summary = {};
  for (const key of ['freshProfileFirstRender', 'existingProfileFirstRender', 'sameProfileReloadFirstRender', 'articleFirstVisit', 'articleRevisit', 'mapFirstVisit', 'mapRevisit']) {
    const samples = result.attempts.filter((a) => a.status === 'complete').map((a) => a.timings[key]).filter(Boolean).map((s) => s.firstRenderMs ?? s.dataReadyMs);
    if (samples.length) result.summary[key] = { count: samples.length, p50: nearestRank(samples, .5), p95: nearestRank(samples, .95) };
  }
  for (const key of ['scrollUncached', 'scrollRepeat']) {
    const passes = result.attempts.filter((attempt) => attempt.status === 'complete').map((attempt) => attempt[key]);
    if (passes.length) result.summary[key] = { count: passes.length, fpsP50: nearestRank(passes.map((pass) => pass.fps), .5),
      fpsP95: nearestRank(passes.map((pass) => pass.fps), .95), fpsMin: Math.min(...passes.map((pass) => pass.fps)),
      frameTimeP95: nearestRank(passes.flatMap((pass) => pass.samples.slice(1).map((sample, index) => sample.at - pass.samples[index].at)), .95),
      visibleGapMsTotal: passes.reduce((sum, pass) => sum + pass.visibleGapMs, 0) };
  }
  result.status = result.attempts.every((a) => a.status === 'complete') ? 'protocol_complete_manual_capabilities_pending' : 'failed_or_unsupported';
} catch { result.status = 'setup_failed'; result.error = 'Pinned input or driver setup failed; no raw error retained.'; }
result.finishedAt = new Date().toISOString();
await save();
console.log(`Android WebView receipt: ${path.basename(output)}; ${result.status}`);
if (result.status !== 'protocol_complete_manual_capabilities_pending') process.exitCode = 1;
