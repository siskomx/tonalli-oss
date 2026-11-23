import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const assetRoot = new URL('../public/assets/map/', import.meta.url);
const assetCommit = '028c18f713baecad011301ff7a69acc39bcc2ae7';
const fonts = ['Noto Sans Regular', 'Noto Sans Medium', 'Noto Sans Italic', 'Noto Sans Devanagari Regular v1'];
const packs = [
  { id: 'mx', bounds: [-99.20, 19.38, -99.10, 19.48], min_zoom: 0, max_zoom: 8, center: [-99.15, 19.43], attribution: 'OpenStreetMap contributors; Protomaps' },
  { id: 'ca', bounds: [-73.62, 45.46, -73.52, 45.56], min_zoom: 0, max_zoom: 8, center: [-73.57, 45.51], attribution: 'OpenStreetMap contributors; Protomaps' },
];
const readJson = async (url) => JSON.parse(await readFile(url, 'utf8'));

function assertLocalUrls(value) {
  if (typeof value === 'string' && /^(?:https?:|\/\/|pmtiles:)/i.test(value)) {
    const url = value.replace(/^pmtiles:\/\//, '');
    assert.ok(url.startsWith('/') && !url.startsWith('//'), `external map URL: ${value}`);
  } else if (Array.isArray(value)) {
    value.forEach(assertLocalUrls);
  } else if (value && typeof value === 'object') {
    Object.values(value).forEach(assertLocalUrls);
  }
}

test('generated style uses local glyph, sprite and archive URLs', async () => {
  const base = await readJson(new URL('style.json', assetRoot));
  const { createMapStyle } = await import('../map-style.js');
  for (const installed of [[packs[0]], [packs[1]], packs]) {
    const style = createMapStyle(base, installed);
    assertLocalUrls(style);
    assert.equal(style.glyphs, '/assets/map/fonts/{fontstack}/{range}.pbf');
    assert.equal(style.sprite, '/assets/map/sprites/sprite');
    assert.deepEqual(Object.keys(style.sources).sort(), installed.map((pack) => `pack:${pack.id}`).sort());
    for (const pack of installed) {
      assert.equal(style.sources[`pack:${pack.id}`].url, `pmtiles:///packs/${pack.id}.pmtiles`);
      assert.match(style.sources[`pack:${pack.id}`].attribution, /OpenStreetMap/);
      assert.match(style.sources[`pack:${pack.id}`].attribution, /Protomaps/);
    }
    const ids = style.layers.map((layer) => layer.id);
    assert.equal(new Set(ids).size, ids.length, 'installed sources need distinct layer IDs');
    assert.ok(style.layers.some((layer) => layer.type === 'symbol' && layer.layout?.['text-field']));
    assert.ok(style.layers.some((layer) => layer.type === 'symbol' && layer.layout?.['icon-image']));
  }
  const single = createMapStyle(base, [packs[0]]);
  const combined = createMapStyle(base, packs);
  assert.deepEqual(combined.layers.filter((layer) => !layer.source), single.layers.filter((layer) => !layer.source));
});

test('complete font inventory includes Latin and accented ranges and verifies every BMP file', async () => {
  const manifest = await readJson(new URL('manifest.json', assetRoot));
  assert.equal(manifest.source_commit, assetCommit);
  const records = new Map(manifest.files.map((file) => [file.path, file]));
  assert.equal(records.size, manifest.files.length, 'duplicate component paths');
  for (const font of fonts) {
    for (let start = 0; start < 65536; start += 256) {
      const path = `fonts/${font}/${start}-${start + 255}.pbf`;
      const record = records.get(path);
      assert.ok(record, `missing glyph range ${path}`);
      assert.ok(record.url.includes(`/${assetCommit}/`), 'component source must be immutable');
      const bytes = await readFile(new URL(path, assetRoot));
      // Empty BMP blocks are valid upstream assets; selected Latin/accent blocks contain glyphs.
      if (start <= 256) assert.ok(bytes.length > 0);
      assert.equal(bytes.length, record.bytes);
      assert.equal(createHash('sha256').update(bytes).digest('hex'), record.sha256);
    }
    assert.ok(records.has(`fonts/${font}/0-255.pbf`));
    assert.ok(records.has(`fonts/${font}/256-511.pbf`));
  }
});

test('both sprite scales contain matching nonempty JSON and PNG atlases', async () => {
  const manifest = await readJson(new URL('manifest.json', assetRoot));
  for (const scale of ['', '@2x']) {
    const jsonPath = `sprites/sprite${scale}.json`;
    const pngPath = `sprites/sprite${scale}.png`;
    const atlas = await readJson(new URL(jsonPath, assetRoot));
    assert.ok(Object.keys(atlas).length > 0);
    const png = await readFile(new URL(pngPath, assetRoot));
    assert.deepEqual([...png.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
    const width = png.readUInt32BE(16);
    const height = png.readUInt32BE(20);
    for (const sprite of Object.values(atlas)) {
      assert.ok(sprite.width > 0 && sprite.height > 0);
      assert.ok(sprite.x >= 0 && sprite.y >= 0);
      assert.ok(sprite.x + sprite.width <= width && sprite.y + sprite.height <= height);
    }
    for (const path of [jsonPath, pngPath]) {
      const record = manifest.files.find((file) => file.path === path);
      assert.ok(record, `missing sprite provenance ${path}`);
      const bytes = await readFile(new URL(path, assetRoot));
      assert.equal(bytes.length, record.bytes);
      assert.equal(createHash('sha256').update(bytes).digest('hex'), record.sha256);
    }
  }
});

test('local attribution and retained font and icon licenses are present', async () => {
  const attribution = await readFile(new URL('attribution', assetRoot), 'utf8');
  assert.match(attribution, /OpenStreetMap/);
  assert.match(attribution, /Protomaps/);
  assert.match(attribution, /ODbL/i);
  const fontLicense = await readFile(new URL('fonts/OFL.txt', assetRoot), 'utf8');
  assert.match(fontLicense, /SIL OPEN FONT LICENSE Version 1\.1/);
  const spriteLicense = await readFile(new URL('sprites/LICENSE.txt', assetRoot), 'utf8');
  assert.match(spriteLicense, /Mapzen/);
  assert.match(spriteLicense, /Permission is hereby granted/);
});

test('installed-map discovery uses authenticated API fetch and rejects revoked sessions', async () => {
  const { loadInstalledMaps } = await import('../map-client.js');
  let request;
  const result = await loadInstalledMaps('session-csrf', async (url, options) => {
    request = { url, options };
    return new Response(JSON.stringify({ packs }));
  });
  assert.deepEqual(result, packs);
  assert.equal(request.url, '/api/maps');
  assert.equal(request.options.credentials, 'same-origin');
  assert.equal(request.options.cache, 'no-store');
  assert.equal(request.options.headers['X-Tonalli-Csrf'], 'session-csrf');
  await assert.rejects(loadInstalledMaps('session-csrf', async () => new Response('', { status: 401 })));
});

test('map view uses installed fixture centers and actual archive zoom limits', async () => {
  const { mapViewFor } = await import('../map-style.js');
  assert.equal(mapViewFor([], undefined), null);
  for (const pack of packs) {
    const view = mapViewFor(packs, pack.id);
    assert.deepEqual(view.center, pack.center);
    assert.equal(view.zoom, 8);
    assert.equal(view.maxZoom, 8);
  }
  assert.throws(() => mapViewFor(packs, 'not-installed'));
});

test('untrusted pack attribution remains escaped text inside the trusted map UI', async () => {
  const { createMapStyle } = await import('../map-style.js');
  const base = { version: 8, sources: {}, layers: [] };
  const malicious = '<img src="https://invalid.example/track" onerror="window.compromised=true"> & publisher';
  const style = createMapStyle(base, [{ ...packs[0], attribution: malicious }]);
  const credit = style.sources['pack:mx'].attribution;
  assert.ok(!credit.includes('invalid.example'));
  assert.ok(credit.includes('&amp; publisher'));
  assert.ok(!credit.includes('<img'));
  assert.ok(credit.includes('href="/assets/map/attribution"'));
  const ordinary = createMapStyle(base, [{ ...packs[0], attribution: '<a href="https://example.invalid/">&copy; Publisher</a> &amp; authors' }]);
  assert.ok(ordinary.sources['pack:mx'].attribution.includes('© Publisher &amp; authors'));
  assert.ok(!ordinary.sources['pack:mx'].attribution.includes('example.invalid'));
  const bodies = createMapStyle(base, [{ ...packs[0], attribution:
    '<SCRIPT>if (a < b) { ignored(); }<script>nested</ScRiPt>Publisher<style>ignored</style> &copy; <scripture>authors</scripture>' }]);
  const bodyCredit = bodies.sources['pack:mx'].attribution;
  assert.ok(bodyCredit.endsWith('Publisher © authors'));
  assert.ok(!bodyCredit.includes('ignored') && !bodyCredit.includes('nested'));
  const adjacent = createMapStyle(base, [{ ...packs[0], attribution: '<details open onload="1" ontoggle="window.compromised=true">Publisher</details>' }]);
  assert.ok(adjacent.sources['pack:mx'].attribution.endsWith('Publisher'));
  assert.ok(!adjacent.sources['pack:mx'].attribution.includes('<details'));
  assert.ok(!adjacent.sources['pack:mx'].attribution.includes('ontoggle'));
});

for (const kind of ['angles', 'script', 'style']) {
  test(`bounded attribution processing handles repeated unclosed ${kind}`, { timeout: 5000 }, () => {
    const child = spawnSync(process.execPath, [fileURLToPath(new URL('./fixtures/attribution-cost.mjs', import.meta.url)), kind], {
      timeout: 3000, windowsHide: true, encoding: 'utf8', maxBuffer: 64 * 1024,
    });
    assert.ifError(child.error);
    assert.equal(child.status, 0, child.stderr);
    const measured = JSON.parse(child.stdout);
    assert.equal(measured.kind, kind);
    assert.ok(measured.bytes >= 524280);
  });
}

test('every legal pack ID including inherited object names remains addressable', async () => {
  const { createMapStyle } = await import('../map-style.js');
  const installed = ['constructor', 'toString', '__proto__'].map((id) => ({ ...packs[0], id }));
  const style = createMapStyle({ version: 8, sources: {}, layers: [] }, installed);
  for (const pack of installed) {
    const sourceId = `pack:${pack.id}`;
    assert.ok(Object.hasOwn(style.sources, sourceId));
    assert.equal(Object.prototype[sourceId], undefined, 'MapLibre uses ordinary-object source caches');
    assert.equal(style.sources[sourceId].url, `pmtiles:///packs/${pack.id}.pmtiles`);
  }
  assert.throws(() => createMapStyle({ layers: [] }, [installed[0], installed[0]]));
});

test('source-bearing layer IDs cannot collide across hyphenated pack IDs', async () => {
  const { createMapStyle } = await import('../map-style.js');
  const base = { layers: [{ id: 'b-roads', source: 'protomaps' }, { id: 'roads', source: 'protomaps' }] };
  const style = createMapStyle(base, [{ ...packs[0], id: 'a' }, { ...packs[0], id: 'a-b' }]);
  assert.equal(new Set(style.layers.map((layer) => layer.id)).size, 4);
});

test('static map harness has a map mount, visible status and local module entry', async () => {
  const html = await readFile(new URL('../map.html', import.meta.url), 'utf8');
  assert.match(html, /id="map"/);
  assert.match(html, /role="status"/);
  assert.match(html, /type="module"[^>]+src="\/map\.js"/);
  assert.doesNotMatch(html, /(?:src|href)=["'](?:https?:|\/\/)/i);
});
