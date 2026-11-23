import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { createServer } from 'vite';
import { chromium } from 'playwright-core';

let vite, browser, origin;
test.before(async () => {
  vite = await createServer({ root: fileURLToPath(new URL('../../', import.meta.url)), logLevel: 'error',
    server: { host: '127.0.0.1', port: 0 } });
  await vite.listen();
  origin = `http://127.0.0.1:${vite.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.TONALLI_ARTICLE_BROWSER
    || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', headless: true });
});
test.after(async () => { await browser?.close(); await vite?.close(); });

async function fixture(operation, locale = 'en-US', options = {}) {
  const context = await browser.newContext({ locale });
  const page = await context.newPage();
  page.setDefaultTimeout(3000);
  const state = { unlocked: false, locks: 0, requests: [], noteModules: [], hold: null,
    notes: [{ id: 1, title: 'Synthetic private title', body: '<img src=x onerror=alert(1)> synthetic private body' }] };
  page.on('request', (request) => {
    if (new URL(request.url()).pathname.startsWith('/src/features/notes/')) state.noteModules.push(request.url());
  });
  await page.route('**/assets/ui-build.json', (route) => route.fulfill({ json: { initialJsBytes: 1, initialCssBytes: 1 } }));
  await page.route('**/api/**', async (route) => {
    const request = route.request(), path = new URL(request.url()).pathname, method = request.method();
    if (path === '/api/session') {
      await options.sessionGate;
      return route.fulfill({ json: { csrf: 'notes-test-csrf', private_notes: true, ...options.session } });
    }
    assert.equal(request.headers()['x-tonalli-csrf'], 'notes-test-csrf');
    state.requests.push({ path, method });
    if (path === '/api/packs') return route.fulfill({ json: { packs: [] } });
    if (path === '/api/zim/titles') return route.fulfill({ json: { pack: 'shared', offset: 0, total: 0, rows: [] } });
    if (path === '/api/notes/lock') {
      state.locks++;
      if (state.failLock) return route.fulfill({ status: 503, json: { error: 'notes_unavailable' } });
      state.unlocked = false;
      return route.fulfill({ status: 204 });
    }
    if (path === '/api/notes/unlock') {
      const body = request.postDataJSON();
      assert.equal(Object.keys(body).length, 1);
      if (Object.values(body)[0] === 'wrong') return route.fulfill({ status: 401, json: { error: 'unlock_failed' } });
      state.unlocked = true;
      return route.fulfill({ json: { unlocked: true } });
    }
    if (!state.unlocked) return route.fulfill({ status: 423, json: { error: 'notes_locked' } });
    if (state.hold && state.hold.match(path, method)) {
      const hold = state.hold;
      state.hold = null;
      hold.enter();
      await hold.promise;
      try { return await route.fulfill({ status: hold.status || 200, json: hold.body }); } catch { return; }
    }
    if (path === '/api/notes' && method === 'POST') {
      const note = { id: state.notes.length + 1, ...request.postDataJSON() };
      state.notes.push(note);
      return route.fulfill({ status: 201, json: { id: note.id } });
    }
    if (/^\/api\/notes\/\d+$/.test(path)) return route.fulfill({ json: state.notes.find((n) => n.id === Number(path.split('/').at(-1))) });
    const query = new URL(request.url()).searchParams.get('q');
    return route.fulfill({ json: { notes: state.notes.filter((n) => !query || n.body.includes(query)) } });
  });
  try { await page.goto(`${origin}/notes`, { waitUntil: 'domcontentloaded' }); await operation({ page, state, context }); }
  finally { await context.close(); }
}

for (const private_notes of [false, undefined, 'true', 1, null]) {
  test(`public session with capability ${JSON.stringify(private_notes)} never mounts Notes on direct or SPA navigation`, async () => {
    await fixture(async ({ page, state }) => {
      const libraryReady = () => page.waitForFunction(() => location.pathname === '/library'
        && window.__ready?.route === '/library' && window.__ready.ready);
      const noNotes = async () => {
        assert.equal(await page.getByTestId('nav-notes').count(), 0);
        assert.equal(await page.getByTestId('notes-credential').count(), 0);
        assert.equal(await page.locator('input[type="password"], textarea').count(), 0);
        assert.deepEqual(state.requests.filter(({ path }) => path.startsWith('/api/notes')), []);
        assert.deepEqual(state.noteModules, [], 'private feature code must not mount or load');
      };
      await libraryReady();
      await noNotes();
      await page.evaluate(() => { history.pushState({}, '', '/notes'); dispatchEvent(new PopStateEvent('popstate')); });
      await libraryReady();
      await noNotes();
    }, 'en-US', { session: { private_notes } });
  });
}

test('Notes stays hidden until the private capability bootstrap completes', async () => {
  let release;
  const sessionGate = new Promise((resolve) => { release = resolve; });
  try {
    await fixture(async ({ page, state }) => {
      await page.getByRole('status').waitFor();
      assert.equal(await page.getByTestId('nav-notes').count(), 0);
      assert.equal(await page.getByTestId('notes-credential').count(), 0);
      assert.deepEqual(state.requests, []);
      release();
      await page.getByTestId('notes-credential').waitFor();
      assert.equal(await page.getByTestId('nav-notes').count(), 1);
      await unlock(page);
      assert.equal(state.unlocked, true);
      await page.waitForFunction(() => window.__ready?.route === '/notes' && window.__ready.ready);
    }, 'en-US', { sessionGate });
  } finally { release(); }
});

async function unlock(page, secret = 'synthetic-passphrase') {
  await page.getByTestId('notes-credential').fill(secret);
  await page.getByTestId('notes-unlock').click();
  await page.getByTestId('notes-lock').waitFor();
  await page.getByTestId('notes-row').first().waitFor();
}
function hold(state, match, body, status) {
  let release, enter;
  const entered = new Promise((resolve) => { enter = resolve; });
  const promise = new Promise((resolve) => { release = resolve; });
  state.hold = { match, body, status, enter, promise };
  return { entered, release };
}

test('notes unlock, add, list, search, selected get, lock and secret-free diagnostics', async () => fixture(async ({ page, state }) => {
  await unlock(page);
  await page.getByTestId('notes-row').first().click();
  await page.getByTestId('notes-selected').waitFor();
  assert.match(await page.getByTestId('notes-selected').innerText(), /<img src=x onerror=alert\(1\)>/);
  assert.equal(await page.getByTestId('notes-selected').locator('img').count(), 0);
  await page.getByTestId('notes-title').fill('New synthetic title');
  await page.getByTestId('notes-body').fill('distinct synthetic body');
  await page.getByTestId('notes-add').click();
  await page.waitForFunction(() => document.querySelector('[data-testid="notes-selected"]')?.textContent.includes('distinct synthetic body'));
  await page.getByTestId('notes-search').fill('distinct');
  await page.getByTestId('notes-search-submit').click();
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="notes-row"]').length === 1);
  assert.match(await page.getByTestId('notes-row').innerText(), /New synthetic title/);
  const exposed = await page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage },
    url: location.href, ready: window.__ready, perf: window.__perf }));
  for (const text of ['synthetic-passphrase', 'distinct synthetic body', 'New synthetic title']) assert.ok(!exposed.includes(text));
  await page.getByTestId('notes-lock').click();
  await page.getByTestId('notes-credential').waitFor();
  assert.ok(!(await page.locator('body').innerText()).includes('synthetic private'));
  assert.equal(state.unlocked, false);
}));

for (const kind of ['list', 'get', 'add']) test(`late ${kind} response cannot repaint after lock`, async () => fixture(async ({ page, state }) => {
  await unlock(page);
  const pending = hold(state, (path, method) => kind === 'list' ? path === '/api/notes/search'
    : kind === 'get' ? path === '/api/notes/1' : path === '/api/notes' && method === 'POST',
  kind === 'add' ? { id: 99 } : kind === 'get' ? state.notes[0] : { notes: state.notes }, kind === 'add' ? 201 : 200);
  if (kind === 'list') await page.getByTestId('notes-search-submit').click();
  if (kind === 'get') await page.getByTestId('notes-row').first().click();
  if (kind === 'add') {
    await page.getByTestId('notes-title').fill('stale synthetic draft');
    await page.getByTestId('notes-body').fill('stale synthetic body');
    await page.getByTestId('notes-add').click();
  }
  try {
    await pending.entered;
    await page.getByTestId('notes-lock').click();
    await page.getByTestId('notes-credential').waitFor();
  } finally { pending.release(); }
  await page.waitForTimeout(100);
  assert.equal(await page.getByTestId('notes-row').count(), 0);
  assert.equal(await page.getByTestId('notes-selected').count(), 0);
  assert.ok(!(await page.locator('body').innerText()).includes('stale synthetic'));
}));

test('leaving Notes clears content while a public lazy module is held', async () => fixture(async ({ page, state }) => {
  await unlock(page);
  let release, enter;
  const holding = new Promise((resolve) => { release = resolve; });
  const entered = new Promise((resolve) => { enter = resolve; });
  await page.route('**/src/features/library/Library.jsx', async (route) => { enter(); await holding; await route.continue(); });
  try {
    await page.getByTestId('nav-library').click();
    await entered;
    await page.waitForFunction(() => !document.querySelector('[data-testid="notes-row"]'));
    assert.ok(!(await page.locator('body').innerText()).includes('Synthetic private title'));
    await page.waitForFunction(() => !document.querySelector('[data-testid="notes-lock"]'));
    assert.ok(state.locks > 0, 'departure sends a separate best-effort vault lock');
  } finally { release(); }
}));

test('wrong unlock remains locked and a lost API session clears all content', async () => fixture(async ({ page, state }) => {
  await page.getByTestId('notes-credential').fill('wrong');
  await page.getByTestId('notes-unlock').click();
  await page.getByRole('alert').waitFor();
  assert.equal(await page.getByTestId('notes-credential').inputValue(), '');
  await unlock(page);
  await page.getByTestId('notes-title').fill('discard draft');
  const pending = hold(state, (path) => path === '/api/notes/1', { error: 'session_failure_private_detail' }, 403);
  await page.getByTestId('notes-row').first().click();
  await pending.entered;
  pending.release();
  await page.getByTestId('notes-credential').waitFor();
  assert.equal(await page.getByTestId('notes-row').count(), 0);
  assert.ok(!(await page.locator('body').innerText()).includes('private_detail'));
}));

test('failed lock clears data and offers an honest retry instead of claiming server lock', async () => fixture(async ({ page, state }) => {
  await unlock(page);
  state.failLock = true;
  await page.getByTestId('notes-lock').click();
  await page.getByTestId('notes-retry-lock').waitFor();
  assert.equal(await page.getByTestId('notes-row').count(), 0);
  assert.equal(state.unlocked, true, 'failed transport/backend lock cannot be called locked');
  assert.match(await page.getByRole('alert').innerText(), /unconfirmed/);
  state.failLock = false;
  await page.getByTestId('notes-retry-lock').click();
  await page.waitForFunction(() => !document.querySelector('[data-testid="notes-retry-lock"]'));
  assert.equal(state.unlocked, false);
}));

test('Spanish controls support recovery unlock without retaining the recovery input', async () => fixture(async ({ page }) => {
  await page.getByLabel('Desbloquear con').selectOption('recovery_key');
  await unlock(page, 'synthetic-recovery');
  assert.equal(await page.getByTestId('notes-credential').count(), 0);
  assert.equal(await page.getByTestId('nav-notes').innerText(), 'Notas');
  assert.equal(await page.getByTestId('notes-lock').innerText(), 'Bloquear y limpiar notas');
}, 'es-MX'));
