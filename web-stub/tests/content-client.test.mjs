import assert from 'node:assert/strict';
import test from 'node:test';
import { loadCatalog, loadJobs, preflight, startDownload, controlJob, removeInstalled } from '../src/features/content/client.js';

const entry = { id: 'medicine', title: 'Medicine', kind: 'zim', language: 'en', version: '2026-10', bytes: 1000,
  installedVersion: null, installedReadable: null };
const catalog = { revision: 7, entries: [entry], installed: [], downloadAvailable: false };
const job = { id: 'job-1', entryId: 'medicine', state: 'verifying', downloadedBytes: 1000, totalBytes: 1000, verifiedBytes: 400, errorCode: null };

test('verified model entries use the same bounded catalog without model paths or URLs', async () => {
  const model = { ...entry, id: 'local-model', title: 'Local model', kind: 'model' };
  const result = await loadCatalog('csrf', undefined, async () => Response.json({ ...catalog, entries: [model] }));
  assert.equal(result.entries[0].kind, 'model');
  await assert.rejects(loadCatalog('csrf', undefined, async () => Response.json({ ...catalog,
    entries: [{ ...model, path: '/private/model.gguf' }] })), /invalid_response/);
});

test('private catalog metadata is authenticated, abortable, and carries no path or URL authority', async () => {
  const controller = new AbortController(); let sent;
  assert.deepEqual(await loadCatalog('csrf', controller.signal, async (url, options) => {
    sent = { url, options }; return Response.json(catalog);
  }), catalog);
  assert.equal(sent.url, '/api/content/catalog');
  assert.equal(sent.options.signal, controller.signal);
  assert.equal(sent.options.credentials, 'same-origin');
  assert.equal(sent.options.cache, 'no-store');
  assert.equal(sent.options.headers['X-Tonalli-Csrf'], 'csrf');
  for (const bad of [
    { ...catalog, entries: [{ ...entry, url: 'https://untrusted.invalid/file' }] },
    { ...catalog, entries: [{ ...entry, id: '../private' }] },
    { ...catalog, entries: [entry, entry] },
    { ...catalog, entries: [{ ...entry, bytes: -1 }] },
    { ...catalog, entries: [{ ...entry, installedVersion: '2026-10', installedReadable: null }] },
    { ...catalog, entries: [{ ...entry, installedReadable: true }] },
    { ...catalog, installed: [{ ...entry, path: '/private/notes.db', readable: true }] },
    { ...catalog, downloadAvailable: 'true' },
    { ...catalog, entries: Array.from({ length: 257 }, (_, index) => ({ ...entry, id: `pack-${index}` })) },
    { ...catalog, installed: Array.from({ length: 257 }, (_, index) => ({ id: `pack-${index}`, title: 'Medicine', kind: 'zim', language: 'en', version: '2026-10', bytes: 1000, readable: true })) },
  ]) await assert.rejects(loadCatalog('csrf', undefined, async () => Response.json(bad)), /invalid_response/);
});

test('jobs preserve authoritative verification and pending-stop states instead of inventing completion', async () => {
  for (const state of ['verifying', 'pausing', 'cancelling']) {
    const result = await loadJobs('csrf', undefined, async () => Response.json({ jobs: [{ ...job, state }] }));
    assert.equal(result.jobs[0].state, state);
  }
  for (const bad of [{ ...job, id: '../../cancel' }, { ...job, state: 'invented' }, { ...job, verifiedBytes: -1 }, { ...job, filepath: '/private' },
    { ...job, downloadedBytes: 1001 }, { ...job, verifiedBytes: 1001 }, { ...job, downloadedBytes: 0, verifiedBytes: 0, totalBytes: 0 }]) {
    await assert.rejects(loadJobs('csrf', undefined, async () => Response.json({ jobs: [bad] })), /invalid_response/);
  }
  await assert.rejects(loadJobs('csrf', undefined, async () => Response.json({ jobs: Array.from({ length: 65 }, (_, index) => ({ ...job, id: `job-${index}`, state: 'completed', verifiedBytes: 1000 })) })), /invalid_response/);
});

test('preflight sends only catalog identity and preserves backend allowance without recomputing policy', async () => {
  let sent;
  const budget = { entryId: 'medicine', availableBytes: 5000, downloadBytes: 500, stagingBytes: 0,
    requiredBytes: 1500, allowed: false, reason: 'waiting_wifi' };
  const result = await preflight('medicine', 7, 'csrf', undefined, async (url, options) => {
    sent = { url, options }; return Response.json(budget);
  });
  assert.equal(sent.url, '/api/content/preflight');
  assert.equal(sent.options.method, 'POST');
  assert.deepEqual(JSON.parse(sent.options.body), { entryId: 'medicine', catalogRevision: 7 });
  assert.equal(result.allowed, false);
  assert.equal(result.requiredBytes, 1500);
});

test('job mutations retain returned identities and pending acknowledgments, never optimistic stopped state', async () => {
  const requests = [];
  const fetcher = async (url, options) => { requests.push({ url, options }); return Response.json({ job: { ...job, state: requests.length === 1 ? 'queued' : 'cancelling' } }); };
  assert.equal((await startDownload('medicine', 7, 'csrf', undefined, fetcher)).job.state, 'queued');
  assert.deepEqual(JSON.parse(requests[0].options.body), { entryId: 'medicine', catalogRevision: 7 });
  assert.equal((await controlJob('job-1', 'cancel', 'csrf', undefined, fetcher)).job.state, 'cancelling');
  assert.equal(requests[1].url, '/api/content/jobs/job-1/cancel');
  assert.equal(requests[1].options.method, 'POST');
  await assert.rejects(controlJob('../other', 'cancel', 'csrf', undefined, fetcher), /invalid_request/);
  await assert.rejects(controlJob('job-1', 'delete_all', 'csrf', undefined, fetcher), /invalid_request/);
  assert.equal(requests.length, 2);
});

test('removal preserves actual in-use refusal and only addresses an installed ID', async () => {
  let sent;
  const result = await removeInstalled('medicine', 'csrf', undefined, async (url, options) => {
    sent = { url, options }; return Response.json({ removed: false, reason: 'in_use' });
  });
  assert.equal(sent.url, '/api/content/installed/medicine');
  assert.equal(sent.options.method, 'DELETE');
  assert.deepEqual(result, { removed: false, reason: 'in_use' });
});

test('errors keep stable codes and discard backend paths and arbitrary exception text', async () => {
  await assert.rejects(loadCatalog('csrf', undefined, async () => Response.json({ error: 'C:/private/notes.db denied' }, { status: 503 })), error => {
    assert.equal(error.code, 'failed'); assert.doesNotMatch(error.message, /private|notes\.db/); return true;
  });
  await assert.rejects(loadCatalog('csrf', undefined, async () => new Response('', { status: 401 })), error => error.code === 'session');
  await assert.rejects(startDownload('medicine', 7, 'csrf', undefined, async () => Response.json({ error: 'catalog_changed' }, { status: 409 })), error => error.code === 'catalog_changed');
  await assert.rejects(loadCatalog('csrf', undefined, async () => { throw null; }), error => error.code === 'failed');
});
