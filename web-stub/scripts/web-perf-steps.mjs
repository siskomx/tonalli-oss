import assert from 'node:assert/strict';
import { routeReady, scrollSummary, visibleGapCount } from './perf-math.mjs';

async function usable(page, route, expectedTitleCount = 19191219) {
  await page.waitForFunction((route) => window.__ready?.route === route && (window.__ready.ready || window.__ready.error), route, { timeout: 20000 });
  const ready = await page.evaluate(() => window.__ready);
  assert.equal(routeReady(ready, route, expectedTitleCount), true, JSON.stringify(ready));
  return ready;
}

async function baseline(page) {
  await page.bringToFront();
  await page.evaluate(() => window.focus());
  return page.evaluate(() => new Promise((resolve, reject) => {
    const frames = [];
    const timeout = setTimeout(() => reject(new Error('idle RAF timeout')), 10000);
    const frame = (at) => {
      frames.push({ at, missing: 0, focused: document.hasFocus(), visibility: document.visibilityState });
      if (frames.length < 120) requestAnimationFrame(frame);
      else { clearTimeout(timeout); resolve({ warmupFrames: frames.slice(0, 30), frames: frames.slice(30), screen: { width: screen.width, height: screen.height,
        availableHeight: screen.availHeight, dpr: devicePixelRatio, innerWidth, innerHeight, outerWidth, outerHeight } }); }
    };
    requestAnimationFrame(frame);
  }));
}

async function reloadFirstRender(page, expectedTitleCount = 19191219) {
  await page.reload({ waitUntil: 'load' });
  await usable(page, '/library', expectedTitleCount);
  return page.evaluate(() => ({ firstRenderMs: window.__perf.firstRenderMs, shellMs: window.__initialShellAt ?? null,
    readiness: window.__ready, focused: document.hasFocus(), visibility: document.visibilityState,
    foregroundEvents: window.__foregroundEvents,
    navigation: performance.getEntriesByType('navigation')[0].toJSON() }));
}

async function transition(host, selector, route, expectedTitleCount = 19191219) {
  await host.page.evaluate((route) => {
    const selector = { '/library': '[data-testid="library-list"]', '/article': '#article-frame', '/map': '[data-testid="map-renderer"]' }[route];
    window.__routeActionAt = null;
    window.__routeShellAt = null;
    const observer = new MutationObserver(() => {
      if (location.pathname === route && document.querySelector(selector)) {
        window.__routeShellAt = performance.now(); observer.disconnect();
      }
    });
    document.addEventListener('click', () => {
      window.__routeActionAt = performance.now(); observer.observe(document, { childList: true, subtree: true });
    }, { capture: true, once: true });
  }, route);
  await host.page.locator(selector).click();
  await usable(host.page, route, expectedTitleCount);
  const sample = await host.page.evaluate(() => ({ dataReadyMs: window.__perf.routeSwitchMs,
    shellMs: window.__routeShellAt === null ? null : window.__routeShellAt - window.__routeActionAt,
    readiness: window.__ready, focused: document.hasFocus(), visibility: document.visibilityState,
    foregroundEvents: window.__foregroundEvents.filter((event) => event.at >= window.__routeActionAt) }));
  assert.ok(Number.isFinite(sample.dataReadyMs) && sample.dataReadyMs >= 0);
  return sample;
}

async function scroll(host, height, pack = 'wiki') {
  const source = visibleGapCount.toString();
  const collect = async ({ height, pack }) => {
    const gaps = GAP_FUNCTION;
    const list = document.querySelector('[data-testid="library-list"]');
    const rowHeight = Number(list.dataset.rowHeight);
    const samples = [];
    let top = 50160 * rowHeight;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('scroll RAF timeout')), 15000);
      let started;
      const frame = (at) => {
        started ??= at;
        const commit = window.__libraryCommit;
        samples.push({ at, top, missing: gaps(commit, { top, height, pack, rowHeight }),
          snapshotMatches: commit?.pack === pack && commit.visibleLimit === 100000 && commit.rowHeight === rowHeight,
          focused: document.hasFocus(), visibility: document.visibilityState });
        const elapsed = Math.min(at - started, 4000);
        top = 50160 * rowHeight + elapsed * 1.2;
        list.scrollTop = top;
        if (at - started < 4000) requestAnimationFrame(frame);
        else { clearTimeout(timeout); resolve(samples); }
      };
      requestAnimationFrame(frame);
    });
  };
  const samples = await host.page.evaluate(`(${collect.toString().replace('GAP_FUNCTION', source)})(${JSON.stringify({ height, pack })})`);
  const summary = scrollSummary(samples);
  await host.page.evaluate((fps) => { window.__perf.listScrollFps = fps; }, summary.fps);
  return { ...summary, samples };
}

export { usable, baseline, reloadFirstRender, transition, scroll };

export async function installObservers(page) {
  await page.addInitScript(() => {
    window.__foregroundEvents = [];
    const record = (event) => window.__foregroundEvents.push({ type: event.type, at: performance.now(),
      focused: document.hasFocus(), visibility: document.visibilityState });
    window.addEventListener('blur', record);
    window.addEventListener('focus', record);
    document.addEventListener('visibilitychange', record);
    const observer = new MutationObserver(() => {
      if (document.querySelector('[data-testid="library-list"]')) {
        window.__initialShellAt = performance.now(); observer.disconnect();
      }
    });
    observer.observe(document, { childList: true, subtree: true });
  });
}
