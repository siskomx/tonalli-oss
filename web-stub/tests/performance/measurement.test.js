import assert from 'node:assert/strict';
import { test } from 'vitest';
import { nearestRank, routeReady, scrollSummary, visibleGapCount } from '../../scripts/perf-math.mjs';

test('nearest rank keeps the slowest ten-sample p95 and rejects invalid samples', () => {
  assert.equal(nearestRank([10, 1, 8, 2, 7, 3, 6, 4, 5, 9], .5), 5);
  assert.equal(nearestRank([10, 1, 8, 2, 7, 3, 6, 4, 5, 9], .95), 10);
  assert.throws(() => nearestRank([], .95));
  assert.throws(() => nearestRank([NaN], .95));
});

test('readiness requires the destination real data, not a shell or retained route', () => {
  assert.equal(routeReady({ route: '/library', ready: true, library: { renderedRows: 2 } }, '/map'), false);
  assert.equal(routeReady({ route: '/article', ready: true, article: { resourcesLoaded: false } }, '/article'), false);
  assert.equal(routeReady({ route: '/map', ready: true, map: { rendered: true } }, '/map'), true);
  assert.equal(routeReady({ route: '/library', ready: true, library: { renderedRows: 2, total: 19191219 } }, '/library'), true);
  assert.equal(routeReady({ route: '/map', ready: true, error: 'failed', map: { rendered: true } }, '/map'), false);
});

test('visible gaps ignore unloaded overscan and include partial visible rows', () => {
  const commit = { pack: 'wiki', visibleLimit: 100000, rowHeight: 40,
    rows: Array.from({ length: 24 }, (_, index) => ({ index, loaded: index >= 8 && index <= 22 })) };
  assert.equal(visibleGapCount(commit, { top: 320, height: 600 }), 0);
  assert.equal(visibleGapCount(commit, { top: 340, height: 600 }), 1);
  assert.equal(visibleGapCount({ ...commit, rows: [{ index: 0, loaded: true }] }, { top: 20, height: 40 }), 1);
  assert.equal(visibleGapCount({ ...commit, pack: 'other' }, { top: 320, height: 600 }), 15);
  assert.equal(visibleGapCount({ ...commit, visibleLimit: 5 }, { top: 320, height: 600 }), 15);
  assert.throws(() => visibleGapCount(commit, { top: NaN, height: 600 }));
  assert.throws(() => visibleGapCount(commit, { top: 320, height: 0 }));
});

test('scroll summary measures cadence and elapsed gaps, not readiness flags', () => {
  const samples = [{ at: 0, missing: 0 }, { at: 10, missing: 2 }, { at: 30, missing: 1 }, { at: 50, missing: 0 }];
  const summary = scrollSummary(samples);
  assert.equal(summary.fps, 60);
  assert.equal(summary.frameTimeP95, 20);
  assert.equal(summary.visibleGapMs, 40);
  assert.equal(summary.maxMissingRows, 2);
  assert.throws(() => scrollSummary([{ at: 1, missing: 0 }, { at: 1, missing: 0 }]));
  assert.throws(() => scrollSummary([{ at: 1, missing: NaN }, { at: 2, missing: 0 }]));
});
