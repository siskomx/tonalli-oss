import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { createMapStyle } from '../../map-style.js';

const kind = process.argv[2];
const unit = { angles: '<', script: '<script>', style: '<style>' }[kind];
assert.ok(unit);
const attribution = unit.repeat(Math.floor(524288 / unit.length));
const start = performance.now();
const style = createMapStyle({ version: 8, sources: {}, layers: [] }, [{
  id: 'bounded', bounds: [-1, -1, 1, 1], min_zoom: 0, max_zoom: 8, center: [0, 0], attribution,
}]);
const credit = style.sources['pack:bounded'].attribution;
assert.ok(credit.includes('OpenStreetMap'));
assert.ok(!credit.includes('<script') && !credit.includes('<style'));
if (kind === 'angles') assert.ok(credit.endsWith('&lt;'.repeat(attribution.length)));
console.log(JSON.stringify({ kind, bytes: attribution.length, milliseconds: performance.now() - start }));
