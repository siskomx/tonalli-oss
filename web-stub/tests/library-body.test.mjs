import assert from 'node:assert/strict';
import test from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { loadBodySearch } from '../src/features/library/client.js';
import BodyResults from '../src/features/library/BodyResults.js';

test('body search uses its own endpoint and preserves authenticated cancellation', async () => {
  const controller = new AbortController(); let sent;
  const result = await loadBodySearch('medicine', 'heart & lung', 'csrf', controller.signal, async (url, options) => {
    sent = { url, options }; return Response.json({ pack: 'medicine', hits: [{ title: 'Heart', url: 'C/Heart', score: 1.25 }] });
  });
  const url = new URL(sent.url, 'http://localhost');
  assert.equal(url.pathname, '/api/zim/search'); assert.equal(url.searchParams.get('q'), 'heart & lung');
  assert.equal(url.searchParams.get('limit'), '20'); assert.equal(sent.options.signal, controller.signal);
  assert.equal(sent.options.headers['X-Tonalli-Csrf'], 'csrf'); assert.equal(sent.options.cache, 'no-store');
  assert.equal(result.hits[0].url, 'C/Heart');
});

test('body search refuses malformed hits, pack mismatches and invalid bounded inputs', async () => {
  for (const hit of [null, { title: 'bad', url: 'https://outside.invalid', score: 1 }, { title: 'bad', url: '../bad', score: 1 },
    { title: 'bad', url: 'C/ok', score: null }, { title: 'bad', url: 'C/ok', score: '1' }, { title: 'bad', url: 'C/ok', score: 1, path: '/private' }]) {
    await assert.rejects(loadBodySearch('medicine', 'q', 'csrf', undefined, async () => Response.json({ pack: 'medicine', hits: [hit] })), /invalid_body_search/);
  }
  for (const body of [{ pack: 'other', hits: [] }, { pack: 'medicine', hits: [], path: '/private' },
    { pack: 'medicine', hits: [{ title: 'X', url: 'C/X', score: 1 }, { title: 'Y', url: 'C/X', score: 2 }] },
    { pack: 'medicine', hits: Array(21).fill({ title: 'X', url: 'C/X', score: 1 }) }]) {
    await assert.rejects(loadBodySearch('medicine', 'q', 'csrf', undefined, async () => Response.json(body)), /invalid_body_search/);
  }
  for (const query of ['', '\u0000', 'é'.repeat(2049)]) {
    await assert.rejects(loadBodySearch('medicine', query, 'csrf', undefined, () => { throw Error('must not fetch'); }), /invalid_body_search/);
  }
});

test('typed body failures never become title results or arbitrary server messages', async () => {
  for (const [status, code] of [[503, 'index_unavailable'], [503, 'media_removed'], [403, 'csrf_invalid']]) {
    await assert.rejects(loadBodySearch('medicine', 'q', 'csrf', undefined, async () => Response.json({ error: code }, { status })), new RegExp(code === 'csrf_invalid' ? 'library_403' : code));
  }
  await assert.rejects(loadBodySearch('medicine', 'q', 'csrf', undefined, async () => Response.json({ error: 'private server text' }, { status: 503 })), /library_503/);
});

test('body search abort reaches the owned fetch without a title fallback', async () => {
  const controller = new AbortController(); let calls = 0;
  const request = loadBodySearch('medicine', 'heart', 'csrf', controller.signal, (_url, { signal }) => {
    calls++;
    return new Promise((_resolve, reject) => signal.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), { once: true }));
  });
  controller.abort();
  await assert.rejects(request, error => error.name === 'AbortError');
  assert.equal(calls, 1);
});

test('body presentation gives explicit setup guidance and canonical article navigation', () => {
  const render = props => renderToStaticMarkup(React.createElement(MemoryRouter, null, React.createElement(BodyResults, {
    t: key => key, article: row => ({ to: `/article?pack=medicine&url=${encodeURIComponent(row.url)}` }), ...props,
  })));
  const missing = render({ error: new Error('index_unavailable') });
  assert.match(missing, /library.bodyIndexMissing/); assert.doesNotMatch(missing, /library.titleOnly|library-row|library.searchFailed/);
  assert.match(render({ error: new Error('media_removed') }), /library.bodyMediaRemoved/);
  const refetch = render({ pending: true, error: new Error('index_unavailable') });
  assert.match(refetch, /app.loading/); assert.doesNotMatch(refetch, /bodyIndexMissing/);
  const hits = render({ data: { hits: [{ title: '<script>literal</script>', url: 'C/Heart', score: 1 }] } });
  assert.match(hits, /article\?pack=medicine&amp;url=C%2FHeart/); assert.match(hits, /&lt;script&gt;literal/);
  assert.doesNotMatch(hits, /data-testid="library-row"/);
});
