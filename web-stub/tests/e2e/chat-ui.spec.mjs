import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { createServer } from 'vite';
import { chromium } from 'playwright-core';

let vite, browser, origin, active;
test.before(async () => {
  vite = await createServer({ root: fileURLToPath(new URL('../../', import.meta.url)), logLevel: 'error',
    server: { host: '127.0.0.1', port: 0 }, plugins: [{ name: 'owned-chat-fixture', configureServer(server) {
      server.middlewares.use('/api/ai/chat', (request, response) => {
        let raw = '';
        request.on('data', (chunk) => { raw += chunk; });
        request.on('end', () => {
          const body = JSON.parse(raw); active.requests.push(body);
          assert.equal(request.headers['x-tonalli-csrf'], 'chat-test-csrf');
          response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-store' });
          const send = (content, done, done_reason) => response.write(`data: ${JSON.stringify({ message: { content }, done, ...(done_reason && { done_reason }) })}\n\n`);
          if (active.hold) {
            active.hold = false; send('old partial', false);
            active.releaseStream = () => { send(' stale tail', true, 'stop'); response.end(); };
          } else {
            send(active.answer, false); send('', true, active.reason || 'stop'); response.end();
          }
        });
      });
    } }] });
  await vite.listen(); origin = `http://127.0.0.1:${vite.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.TONALLI_ARTICLE_BROWSER
    || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', headless: true });
});
test.after(async () => { await browser?.close(); await vite?.close(); });

async function fixture(operation, options = {}) {
  const capability = Object.hasOwn(options, 'capability') ? options.capability : true;
  const locale = options.locale || 'en-US';
  active = { requests: [], cancels: [], modules: [], api: [], answer: 'Local answer', hold: false };
  const state = active;
  const context = await browser.newContext({ locale }); const page = await context.newPage();
  page.setDefaultTimeout(5000);
  page.on('request', (request) => { if (new URL(request.url()).pathname.startsWith('/src/features/chat/')) state.modules.push(request.url()); });
  await page.route('**/assets/ui-build.json', (route) => route.fulfill({ json: { initialJsBytes: 1, initialCssBytes: 1 } }));
  await page.route('**/api/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/api/ai/chat') return route.continue();
    if (path === '/api/session') return route.fulfill({ json: { csrf: 'chat-test-csrf', private_ai: capability, private_notes: false } });
    state.api.push(path);
    if (path === '/api/ai/model') return route.fulfill({ json: { model: 'owned-fixture', kind: 'generation', file_bytes: 100,
      training_context_tokens: 4096, embedding_dimensions: 4, estimated_reserved_bytes: 1000 } });
    if (path === '/api/ai/cancel') {
      state.cancels.push(route.request().postDataJSON().request_id);
      if (state.holdCancel) { await state.holdCancel; return route.fulfill({ status: 503 }); }
      return route.fulfill({ status: 204 });
    }
    if (path === '/api/packs') return route.fulfill({ json: { packs: [] } });
    if (path === '/api/zim/titles') return route.fulfill({ json: { pack: 'shared', offset: 0, total: 0, rows: [] } });
    return route.fulfill({ status: 404 });
  });
  try { await page.goto(`${origin}/chat`); await operation({ page, state }); }
  finally { state.releaseStream?.(); await context.close(); }
}

test('missing, malformed and public capabilities deny before importing chat or calling AI', async () => {
  for (const capability of [undefined, false, 'true', 1, null]) await fixture(async ({ page, state }) => {
    await page.waitForFunction(() => location.pathname === '/library' && window.__ready?.ready);
    assert.equal(await page.getByTestId('nav-chat').count(), 0);
    assert.deepEqual(state.modules, []); assert.deepEqual(state.api.filter((path) => path.startsWith('/api/ai')), []);
  }, { capability });
});

test('reused composer sends completed context and starts fresh explicitly; Markdown has no remote navigation/images', async () => {
  await fixture(async ({ page, state }) => {
    state.answer = '**Answer** [remote](https://example.invalid/x) ![blocked](https://example.invalid/i.png)';
    state.reason = 'length'; const external = [];
    page.on('request', (request) => { if (new URL(request.url()).origin !== origin) external.push(request.url()); });
    await page.getByRole('textbox', { name: 'Question', exact: true }).fill('First question');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await page.getByText('This answer reached the output limit', { exact: false }).waitFor();
    assert.equal(await page.getByRole('button', { name: 'Continue', exact: true }).count(), 0);
    assert.equal(await page.locator('.chat-ui img, .chat-ui a[href]').count(), 0); assert.deepEqual(external, []);
    await page.getByRole('textbox', { name: 'Question', exact: true }).fill('Second question');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await page.waitForFunction(() => document.querySelectorAll('.chat-ui strong').length === 2);
    assert.deepEqual(state.requests.map((request) => request.messages), [[{ role: 'user', content: 'First question' }],
      [{ role: 'user', content: 'First question' }, { role: 'assistant', content: state.answer }, { role: 'user', content: 'Second question' }]]);
    assert.notEqual(state.requests[0].request_id, state.requests[1].request_id);
    assert.equal(await page.getByText('First question', { exact: true }).count(), 1);
    await page.getByRole('button', { name: 'New conversation', exact: true }).click();
    assert.equal(await page.getByText('First question', { exact: true }).count(), 0);
    await page.getByRole('textbox', { name: 'Question', exact: true }).fill('Fresh question');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await page.getByText('Fresh question', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'New conversation', exact: true }).waitFor({ state: 'visible' });
    assert.deepEqual(state.requests.at(-1).messages, [{ role: 'user', content: 'Fresh question' }]);
  });
});

test('Stop sends owned native cancellation and stale stream/cancel failure cannot repaint a newer turn', async () => {
  await fixture(async ({ page, state }) => {
    state.hold = true; let releaseCancel; state.holdCancel = new Promise((resolve) => { releaseCancel = resolve; });
    await page.getByRole('textbox', { name: 'Question', exact: true }).fill('Old question');
    await page.getByRole('button', { name: 'Send', exact: true }).click(); await page.getByText('old partial', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'Stop', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('textarea')?.disabled === false);
    state.answer = 'New answer';
    await page.getByRole('textbox', { name: 'Question', exact: true }).fill('New question');
    await page.getByRole('button', { name: 'Send', exact: true }).click(); await page.getByText('New answer', { exact: true }).waitFor();
    releaseCancel(); state.releaseStream(); state.releaseStream = null;
    await page.waitForTimeout(100);
    assert.deepEqual(state.cancels, [state.requests[0].request_id]);
    assert.deepEqual(state.requests[1].messages, [{ role: 'user', content: 'New question' }]);
    assert.equal(await page.getByText('stale tail', { exact: false }).count(), 0);
    assert.equal(await page.getByText('Native stop could not be confirmed.', { exact: false }).count(), 0);
  });
});

test('leaving chat for a held lazy public route cancels before that route is delivered', async () => {
  await fixture(async ({ page, state }) => {
    state.hold = true; let release;
    const held = new Promise((resolve) => { release = resolve; });
    await page.route('**/src/features/library/Library.jsx', async (route) => { await held; await route.continue(); });
    await page.getByRole('textbox', { name: 'Question', exact: true }).fill('Departing question');
    await page.getByRole('button', { name: 'Send', exact: true }).click(); await page.getByText('old partial', { exact: true }).waitFor();
    await page.getByTestId('nav-library').click();
    await page.waitForFunction(() => !document.querySelector('.chat-ui'));
    assert.deepEqual(state.cancels, [state.requests[0].request_id]);
    release(); state.releaseStream(); state.releaseStream = null;
  });
});

test('Spanish chat uses translated conversation controls', async () => {
  await fixture(async ({ page }) => {
    await page.getByRole('textbox', { name: 'Pregunta', exact: true }).fill('Una pregunta');
    await page.getByRole('button', { name: 'Enviar', exact: true }).click(); await page.getByText('Local answer', { exact: true }).waitFor();
  }, { locale: 'es-MX' });
});
