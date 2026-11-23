import assert from 'node:assert/strict';
import test from 'node:test';
import { createNotesClient, NotesError } from '../src/features/notes/client.js';

test('notes client uses the private session contract and JSON credentials only', async () => {
  const requests = [];
  const replies = [{ unlocked: true }, { notes: [] }, { notes: [] }, { id: 1 },
    { id: 1, title: 'Synthetic title', body: 'Synthetic body' }, null];
  const abort = new AbortController();
  const client = createNotesClient('fixture-csrf', async (url, options) => {
    requests.push({ url, options });
    const body = replies.shift();
    return body === null ? new Response(null, { status: 204 }) : Response.json(body);
  });
  await client.unlock('passphrase', 'private passphrase', abort.signal);
  await client.list(abort.signal);
  await client.search('literal & query', abort.signal);
  assert.deepEqual(await client.add('Synthetic title', 'Synthetic body', abort.signal), { id: 1 });
  assert.equal((await client.get(1, abort.signal)).body, 'Synthetic body');
  await client.lock({ keepalive: true });
  assert.deepEqual(requests.map((r) => r.url), ['/api/notes/unlock', '/api/notes',
    '/api/notes/search?q=literal+%26+query', '/api/notes', '/api/notes/1', '/api/notes/lock']);
  assert.equal(requests[0].options.body, JSON.stringify({ passphrase: 'private passphrase' }));
  for (const { url, options } of requests) {
    assert.equal(options.credentials, 'same-origin');
    assert.equal(options.cache, 'no-store');
    assert.equal(options.headers['X-Tonalli-Csrf'], 'fixture-csrf');
    assert.ok(!url.includes('private passphrase'));
  }
  assert.equal(requests[0].options.signal, abort.signal);
  assert.equal(requests.at(-1).options.keepalive, true);
  assert.equal(requests.at(-1).options.signal, undefined);
  assert.equal(requests.at(-1).options.body, undefined);
});

test('recovery unlock sends exactly one credential and validates note shapes', async () => {
  let sent;
  const client = createNotesClient('csrf', async (_, options) => {
    sent = JSON.parse(options.body);
    return Response.json({ unlocked: true });
  });
  await client.unlock('recovery_key', 'synthetic-recovery');
  assert.deepEqual(sent, { recovery_key: 'synthetic-recovery' });
  await assert.rejects(client.unlock('both', 'bad'), /invalid_request/);
  for (const body of [{ notes: [{ id: 1, title: 'missing body' }] }, { notes: [{ id: 1.5, title: '', body: '' }] }]) {
    await assert.rejects(createNotesClient('csrf', async () => Response.json(body)).list(), /invalid_response/);
  }
});

test('only stable errors escape the client, never raw server messages or secrets', async () => {
  for (const [status, body, code] of [[401, { error: 'unlock_failed' }, 'unlock_failed'],
    [423, { error: 'notes_locked' }, 'notes_locked'], [413, { error: 'input_too_large' }, 'input_too_large'],
    [500, { error: 'private path /secret and credential' }, 'notes_unavailable']]) {
    const client = createNotesClient('csrf', async () => Response.json(body, { status }));
    await assert.rejects(client.list(), (error) => error instanceof NotesError && error.status === status && error.code === code
      && !String(error).includes('/secret'));
  }
  await assert.rejects(createNotesClient('csrf', async () => { throw new Error('private transport detail'); }).list(),
    (error) => error.code === 'notes_unavailable' && !String(error).includes('transport'));
});
