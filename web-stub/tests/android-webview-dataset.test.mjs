import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { routeReady } from '../scripts/perf-math.mjs';
import { usable, reloadFirstRender, transition } from '../scripts/web-perf-steps.mjs';
import * as adapter from '../scripts/android-webview-adapter.mjs';

const ready = { route: '/library', ready: true, library: { total: 371615, renderedRows: 20 } };

test('an explicitly bound Medicine count qualifies without changing the full default', () => {
  assert.equal(routeReady(ready, '/library', 371615), true);
  assert.equal(routeReady(ready, '/library'), false);
  assert.equal(routeReady({ ...ready, library: { ...ready.library, total: 19191219 } }, '/library'), true);
  for (const count of [371614, 99999, NaN, Infinity, 2 ** 53]) {
    assert.equal(routeReady(ready, '/library', count), false);
  }
  assert.equal(routeReady({ ...ready, ready: false }, '/library', 371615), false);
  assert.equal(routeReady({ ...ready, error: 'failed' }, '/library', 371615), false);
  assert.equal(routeReady({ ...ready, library: { total: 371615, renderedRows: 0 } }, '/library', 371615), false);
});

test('shared readiness helpers propagate the explicit dataset count across reload and returns', async () => {
  const sample = { dataReadyMs: 1, foregroundEvents: [], focused: true, visibility: 'visible' };
  const page = {
    async waitForFunction() {}, async reload() {},
    locator: () => ({ async click() {} }),
    async evaluate(fn) {
      if (fn.toString().includes('window.__ready)')) return ready;
      if (fn.toString() === '() => window.__ready') return ready;
      return sample;
    },
  };
  assert.equal(await usable(page, '/library', 371615), ready);
  assert.equal(await reloadFirstRender(page, 371615), sample);
  assert.equal(await transition({ page }, 'known-link', '/library', 371615), sample);
});

test('measurement metadata binds the actual packaged source/table pins and an explicit large count', async () => {
  const raw = await readFile(new URL('../../android/app/src/main/assets/benchmark-medicine.json', import.meta.url), 'utf8');
  const parsed = adapter.parseMeasurementDataset(raw, { dataset: 'medicine', expectedTitleCount: '371615' });
  assert.equal(parsed.expectedTitleCount, 371615);
  assert.equal(parsed.zim.sha256, '718fadad40b3491be3fa8f95181a948e4d726f97a79a14e0ef1b90c6b2b03625');
  assert.equal(parsed.titleIndex.sha256, '3e4f64512d3aaaa99004841ebd48fb79d5d569cdc5f3c754dc39c4f11373a598');
  assert.throws(() => adapter.parseMeasurementDataset(raw, { dataset: 'full', expectedTitleCount: '371615' }));
  for (const expectedTitleCount of ['99999', 'NaN', '371615x', '9007199254740992']) {
    assert.throws(() => adapter.parseMeasurementDataset(raw, { dataset: 'medicine', expectedTitleCount }));
  }
  const invalid = JSON.parse(raw); invalid.title_index.sha256 = 'untrusted';
  assert.throws(() => adapter.parseMeasurementDataset(JSON.stringify(invalid), { dataset: 'medicine', expectedTitleCount: '371615' }));
});

test('row99999 identity is derived from a successful bound title API response, not a full-Wikipedia literal', () => {
  const response = { pack: 'probe', offset: 99999, total: 371615, rows: [{ title: 'Observed title', url: 'C/Observed_article' }] };
  assert.deepEqual(adapter.qualifyMeasurementTitle(response, 371615), {
    pack: 'probe', index: 99999, total: 371615, title: 'Observed title', url: 'C/Observed_article',
  });
  for (const change of [{ pack: 'other' }, { offset: 0 }, { total: 19191219 }, { rows: [] },
    { rows: [{ title: 'Wrong authority', url: 'https://untrusted.invalid/article' }] }]) {
    assert.throws(() => adapter.qualifyMeasurementTitle({ ...response, ...change }, 371615));
  }
});
