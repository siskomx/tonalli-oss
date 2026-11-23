import assert from 'node:assert/strict';
import test from 'node:test';
import { loadPacks, loadSuggestions, loadTitlePage, titleWindow } from '../src/features/library/client.js';

test('installed packs use authenticated metadata only and reject malformed/duplicate identities', async () => {
  const controller = new AbortController(); let sent;
  const fetcher = async (url, options) => { sent = { url, options }; return Response.json({ packs: [{ id: 'medicine', title: 'Medicine', total: 371615 }] }); };
  assert.deepEqual(await loadPacks('csrf', controller.signal, fetcher), [{ id: 'medicine', title: 'Medicine', total: 371615 }]);
  assert.equal(sent.url, '/api/packs'); assert.equal(sent.options.signal, controller.signal);
  assert.equal(sent.options.headers['X-Tonalli-Csrf'], 'csrf');
  assert.equal(sent.options.credentials, 'same-origin'); assert.equal(sent.options.cache, 'no-store');
  for (const packs of [[{ id: 'x', title: 'X', total: -1 }], [{ id: 'x', title: 'X', total: 1 }, { id: 'x', title: 'Y', total: 2 }], [{ id: 'x', title: 'X', total: 1, path: '/private' }]]) {
    await assert.rejects(loadPacks('csrf', undefined, async () => Response.json({ packs })), /invalid_packs/);
  }
  assert.deepEqual(await loadPacks('csrf', undefined, async () => Response.json({ packs: [] })), []);
});

test('suggestions preserve the exact query and canonical article URLs with request cancellation', async () => {
  const controller = new AbortController(); let sent;
  const result = await loadSuggestions('medicine', 'Ácido & heart', 'csrf', controller.signal, async (url, options) => {
    sent = { url, options }; return Response.json({ pack: 'medicine', hits: [{ title: 'Ácido', url: 'C/Canonical_article' }] });
  });
  const url = new URL(sent.url, 'http://localhost');
  assert.equal(url.pathname, '/api/zim/suggest'); assert.equal(url.searchParams.get('q'), 'Ácido & heart');
  assert.equal(url.searchParams.get('pack'), 'medicine'); assert.equal(sent.options.signal, controller.signal);
  assert.equal(result.hits[0].url, 'C/Canonical_article');
  await assert.rejects(loadSuggestions('medicine', 'q', 'csrf', undefined, async () => Response.json({ pack: 'other', hits: [] })), /invalid_suggestions/);
  await assert.rejects(loadSuggestions('medicine', 'q', 'csrf', undefined, async () => Response.json({ pack: 'medicine', hits: [{ title: 'bad', url: 'https://outside.invalid/' }] })), /invalid_suggestions/);
});

test('bounded browse windows reach every title beyond100k without an unbounded scroll surface', () => {
  assert.deepEqual(titleWindow(19191219, 0), { start: 0, count: 10000, previous: null, next: 10000 });
  assert.deepEqual(titleWindow(100001, 100000), { start: 100000, count: 1, previous: 90000, next: null });
  assert.deepEqual(titleWindow(100001, 999999), { start: 100000, count: 1, previous: 90000, next: null });
  assert.deepEqual(titleWindow(0, 0), { start: 0, count: 0, previous: null, next: null });
  let reached = 0, cursor = 0;
  do { const window = titleWindow(100001, cursor); reached += window.count; cursor = window.next; } while (cursor !== null);
  assert.equal(reached, 100001);
});

test('title pages refuse a different selected pack and preserve caller abort signals', async () => {
  const controller = new AbortController(); let options;
  await assert.rejects(loadTitlePage('medicine', 100000, 'csrf', controller.signal, async (_url, received) => {
    options = received; return Response.json({ pack: 'other', total: 100001, offset: 100000, rows: [] });
  }), /invalid_title_page/);
  assert.equal(options.signal, controller.signal);
});
