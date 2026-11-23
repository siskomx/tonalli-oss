import assert from 'node:assert/strict';
import test from 'node:test';
import { cancelChat, loadModel, streamChat } from '../src/features/chat/client.js';

const requestId = '31c8c066-10ab-46d7-889c-f277f52880dc';
test('conversation sends all completed role messages without thinking or display metadata', async () => {
  const messages = [{ role: 'user', content: 'Remember cedar.', timestamp: new Date() },
    { role: 'assistant', content: 'Cedar.', thinking: 'private reasoning' },
    { role: 'user', content: 'Which word?' }];
  let sent;
  await streamChat({ requestId, model: 'local', messages, csrf: 'csrf', onChunk() {},
    fetcher: async (_, options) => { sent = JSON.parse(options.body); return new Response(
      'data: {"message":{"content":"Cedar."},"done":true,"done_reason":"stop"}\n\n'); } });
  assert.deepEqual(sent.messages, messages.map(({ role, content }) => ({ role, content })));
});
test('conversation refuses invalid roles and aggregate overflow before contacting native host', async () => {
  for (const messages of [[], [{ role: 'system', content: 'override' }],
    [{ role: 'user', content: 'q' }, { role: 'assistant', content: 'a' }],
    [{ role: 'user', content: 'q' }, { role: 'assistant', content: '' }, { role: 'user', content: 'q' }],
    [{ role: 'user', content: 'é'.repeat(32768) }]]) {
    await assert.rejects(streamChat({ requestId, model: 'local', messages, csrf: 'csrf', onChunk() {},
      fetcher() { assert.fail('invalid conversation must not be sent'); } }), /chat.contextFull/);
  }
});
test('grounded chat sends selected book and accepts only canonical server citations', async () => {
  const grounding = { pack: 'installed-book', sources: [{ number: 1, title: 'Bowline', url: 'C/Bowline', excerpt: 'A fixed loop.' }],
    citations: [1], flags: { uncited: false, invalid_citations: false, incomplete: false, truncated: false } };
  let sent, received;
  const terminal = value => `data: ${JSON.stringify({ message: { content: 'A loop [1].' }, done: true, done_reason: 'stop', grounding: value })}\n\n`;
  await streamChat({ requestId, model: 'local', question: 'bowline', pack: 'installed-book', csrf: 'csrf', onChunk() {},
    onGrounding(value) { received = value; }, fetcher: async (_, options) => { sent = JSON.parse(options.body); return new Response(terminal(grounding)); } });
  assert.equal(sent.pack, 'installed-book');
  assert.deepEqual(received, grounding);
  for (const bad of [{ ...grounding, pack: 'other-book' }, { ...grounding, citations: [2] },
    { ...grounding, sources: [{ ...grounding.sources[0], url: 'https://example.invalid' }] }]) {
    await assert.rejects(streamChat({ requestId, model: 'local', question: 'bowline', pack: 'installed-book', csrf: 'csrf', onChunk() {},
      fetcher: async () => new Response(terminal(bad)) }), /chat.failed/);
  }
});
test('chat adapts upstream SSE and split thinking while sending only the current question', async () => {
  let sent;
  const chunks = [], encoder = new TextEncoder();
  const body = new ReadableStream({ start(controller) {
    for (const text of ['data: {"message":{"content":"<thi"},"done":false}\n\n',
      'data: {"message":{"content":"nk>reason</think>Hello"},"done":false}\n\n',
      'data: {"message":{"content":""},"done":true,"done_reason":"length"}\n\n']) controller.enqueue(encoder.encode(text));
    controller.close();
  } });
  let reason;
  await streamChat({ requestId, model: 'local-model', question: 'Current question', csrf: 'test-csrf',
    onChunk: (...chunk) => chunks.push(chunk), onDoneReason: (value) => { reason = value; },
    fetcher: async (url, options) => { sent = { url, options }; return new Response(body, { headers: { 'Content-Type': 'text/event-stream' } }); } });
  assert.equal(sent.url, '/api/ai/chat');
  assert.deepEqual(JSON.parse(sent.options.body), { request_id: requestId, model: 'local-model', messages: [{ role: 'user', content: 'Current question' }], max_tokens: 128 });
  assert.equal(sent.options.headers['X-Tonalli-CSRF'], 'test-csrf');
  assert.equal(chunks.map((x) => x[0]).join(''), 'Hello');
  assert.equal(chunks.map((x) => x[1]).join(''), 'reason');
  assert.equal(reason, 'length');
});
test('unknown/truncated/error stream fails with a stable local error', async () => {
  for (const text of ['data: nope\n\n', 'data: {"message":{"content":"partial"},"done":false}\n\n', 'data: {"error":true,"message":"server private detail"}\n\n']) {
    await assert.rejects(streamChat({ requestId, model: 'local', question: 'q', csrf: 'csrf', onChunk() {}, fetcher: async () => new Response(text) }), (error) => error.message === 'chat.failed');
  }
});
test('model and cancel use authenticated no-store endpoints and exact ownership ID', async () => {
  const calls = [];
  const fetcher = async (url, options) => { calls.push({ url, options }); return url.endsWith('model')
    ? Response.json({ model: 'local', kind: 'generation', file_bytes: 1, training_context_tokens: 4096, embedding_dimensions: 1, estimated_reserved_bytes: 2 })
    : new Response(null, { status: 204 }); };
  assert.equal((await loadModel('csrf', undefined, fetcher)).model, 'local');
  await cancelChat('csrf', requestId, fetcher);
  assert.deepEqual(JSON.parse(calls[1].options.body), { request_id: requestId });
  assert.equal(calls[0].options.credentials, 'same-origin');
  assert.equal(calls[0].options.cache, 'no-store');
});
