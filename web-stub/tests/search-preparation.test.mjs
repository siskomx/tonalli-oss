import assert from 'node:assert/strict';
import test from 'node:test';
import { loadPreparation, startPreparation, controlPreparation } from '../src/features/library/preparation-client.js';

const initial = { pack: 'book', jobId: null, epoch: null, state: 'unprepared', phase: null,
  processedArticles: null, committedArticles: null, reason: null };
const paused = { ...initial, jobId: 'a'.repeat(32), epoch: '18446744073709551615', state: 'paused',
  phase: 'indexing', processedArticles: 12, committedArticles: 12 };

test('preparation status stays private and abortable; unknown counters stay unknown', async () => {
  const controller = new AbortController(); let request;
  const status = await loadPreparation('book', 'csrf', controller.signal, async (url, options) => {
    request = { url, options }; return Response.json(initial);
  });
  assert.equal(status.processedArticles, null);
  assert.equal(request.url, '/api/zim/index-status?pack=book');
  assert.equal(request.options.headers['X-Tonalli-Csrf'], 'csrf');
  assert.equal(request.options.credentials, 'same-origin');
  assert.equal(request.options.cache, 'no-store');
  assert.equal(request.options.signal, controller.signal);
});

test('start sends only a registered book ID; control preserves the exact u64 epoch as text', async () => {
  const sent = [];
  const fetcher = async (url, options) => { sent.push({ url, ...options }); return Response.json(paused); };
  await startPreparation('book', 'csrf', undefined, fetcher);
  await controlPreparation('book', paused.jobId, paused.epoch, 'resume', 'csrf', undefined, fetcher);
  assert.equal(sent[0].url, '/api/zim/index-start');
  assert.equal(sent[0].method, 'POST');
  assert.deepEqual(JSON.parse(sent[0].body), { pack: 'book' });
  assert.equal(sent[1].url, '/api/zim/index-control');
  assert.deepEqual(JSON.parse(sent[1].body), { jobId: paused.jobId, epoch: paused.epoch, action: 'resume' });
  assert.equal(sent[1].headers['Content-Type'], 'application/json');
});

test('bad identifiers, epochs and actions are rejected before any request', async () => {
  let calls = 0; const fetcher = () => { calls++; throw Error('must not fetch'); };
  for (const id of ['', '../book', 'https://outside.invalid', 'x'.repeat(129)])
    await assert.rejects(loadPreparation(id, 'csrf', undefined, fetcher), /invalid_preparation_request/);
  for (const epoch of [0, 1, '0', '-1', '01', '18446744073709551616', '1e3'])
    await assert.rejects(controlPreparation('book', paused.jobId, epoch, 'pause', 'csrf', undefined, fetcher), /invalid_preparation_request/);
  await assert.rejects(controlPreparation('book', paused.jobId, '1', 'finish', 'csrf', undefined, fetcher), /invalid_preparation_request/);
  assert.equal(calls, 0);
});

test('incoherent, cross-book and path-bearing status cannot become preparation UI state', async () => {
  for (const invalid of [null, { ...initial, pack: 'other' }, { ...initial, path: '/private' },
    { ...initial, state: 'complete' }, { ...paused, epoch: 1 }, { ...paused, epoch: null },
    { ...initial, processedArticles: 1 }, { ...paused, processedArticles: -1 },
    { ...paused, committedArticles: 13 }, { ...paused, phase: 'estimated' },
    { ...paused, reason: '/private/error' }, { ...paused, jobId: 'unknown' }]) {
    await assert.rejects(loadPreparation('book', 'csrf', undefined, async () => Response.json(invalid)), /invalid_preparation_response/);
  }
});

test('bounded error codes never display arbitrary server text; cancellation reaches fetch', async () => {
  await assert.rejects(startPreparation('book', 'csrf', undefined,
    async () => Response.json({ error: '/private/path token=secret' }, { status: 500 })), error => error.code === 'failed');
  await assert.rejects(startPreparation('book', 'csrf', undefined,
    async () => Response.json({ error: 'busy' }, { status: 409 })), error => error.code === 'busy');
  const controller = new AbortController();
  const pending = loadPreparation('book', 'csrf', controller.signal, (_url, { signal }) =>
    new Promise((_resolve, reject) => signal.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), { once: true })));
  controller.abort(); await assert.rejects(pending, error => error.name === 'AbortError');
});
