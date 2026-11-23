import assert from 'node:assert/strict';
import { open, writeFile } from 'node:fs/promises';
import test from 'node:test';
import { startArticleHost, trustedNotesRequest } from './article-host.mjs';

test('shared Notes UI uses the actual encrypted Rust vault and locks on departure', async () => {
  assert.ok(process.env.TONALLI_NOTES_REAL_TRACE, 'Set a new TONALLI_NOTES_REAL_TRACE output');
  const output = await open(process.env.TONALLI_NOTES_REAL_TRACE, 'wx');
  const checks = [];
  let host;
  try {
    host = await startArticleHost({ notesFixture: true, locale: process.env.TONALLI_NOTES_LOCALE || 'en-US' });
    const before = await host.notes.fingerprint();
    await host.page.getByTestId('nav-notes').click();
    await host.page.getByTestId('notes-credential').waitFor();
    assert.equal((await trustedNotesRequest(host.page)).status, 423);
    checks.push('actual vault initially locked');

    await host.page.getByTestId('notes-credential').fill('incorrect-test-input');
    const wrongUnlock = host.page.waitForResponse((response) => new URL(response.url()).pathname === '/api/notes/unlock');
    await host.page.getByTestId('notes-unlock').click();
    assert.equal((await wrongUnlock).status(), 401);
    await host.page.getByRole('alert').waitFor();
    assert.equal(await host.page.getByTestId('notes-credential').inputValue(), '');
    assert.deepEqual(await host.notes.fingerprint(), before);
    assert.equal((await trustedNotesRequest(host.page)).status, 423);
    checks.push('wrong unlock clears input and preserves encrypted files');

    async function unlock() {
      await host.page.getByTestId('notes-credential').fill('tonalli-browser-test-only');
      await host.page.getByTestId('notes-unlock').click();
      await host.page.getByTestId('notes-lock').waitFor();
      assert.equal(await host.page.getByTestId('notes-credential').count(), 0);
    }
    await unlock();
    assert.equal(await host.page.getByTestId('notes-row').count(), 1);
    const title = 'Prueba española <script>literal</script>';
    const body = 'Texto de prueba 100% <img src=x onerror="window.notesXss=true">';
    await host.page.getByTestId('notes-title').fill(title);
    await host.page.getByTestId('notes-body').fill(body);
    await host.page.getByTestId('notes-add').click();
    await host.page.getByTestId('notes-selected').waitFor();
    assert.ok((await host.page.getByTestId('notes-selected').innerText()).includes(body));
    assert.equal(await host.page.getByTestId('notes-selected').locator('img, script').count(), 0);
    assert.equal(await host.page.evaluate(() => Boolean(window.notesXss)), false);
    checks.push('actual add/list/get preserves Unicode and renders HTML as text');

    await host.page.getByTestId('notes-search').fill('%');
    await host.page.getByTestId('notes-search-submit').click();
    await host.page.waitForFunction(() => document.querySelectorAll('[data-testid="notes-row"]').length === 1);
    assert.equal(await host.page.getByTestId('notes-row').innerText(), title);
    await host.page.getByTestId('notes-row').click();
    await host.page.getByTestId('notes-selected').waitFor();
    checks.push('actual literal search and selected-note fetch');

    const locked = host.page.waitForResponse((response) => new URL(response.url()).pathname === '/api/notes/lock');
    await host.page.getByTestId('notes-lock').click();
    assert.equal(await host.page.getByTestId('notes-selected').count(), 0);
    assert.equal((await locked).status(), 204);
    assert.equal((await trustedNotesRequest(host.page)).status, 423);
    assert.equal(await host.page.locator('html').getAttribute('lang'), host.locale.startsWith('es') ? 'es' : 'en');
    await writeFile(`${process.env.TONALLI_NOTES_REAL_TRACE.replace(/\.json$/, '')}-locked.png`,
      await host.page.screenshot({ fullPage: true, type: 'png' }), { flag: 'wx' });
    checks.push('explicit lock clears UI and actual server returns 423');

    await unlock();
    const departed = host.page.waitForResponse((response) => new URL(response.url()).pathname === '/api/notes/lock');
    await host.page.getByTestId('nav-library').click();
    assert.equal(await host.page.getByTestId('notes-lock').count(), 0);
    assert.equal((await departed).status(), 204);
    assert.equal((await trustedNotesRequest(host.page)).status, 423);
    const after = await host.notes.fingerprint();
    assert.notEqual(after['notes.db'].sha256, before['notes.db'].sha256);
    assert.deepEqual(after['notes.keys'], before['notes.keys']);
    checks.push('departure clears private UI and locks actual vault; writes persisted without rewrapping key');
  } finally {
    try {
      await output.writeFile(`${JSON.stringify({ checks, complete: checks.length === 6,
        cliSha256: host?.cliSha256, uiSha256: host?.distSha256, locale: host?.locale,
        fixtureExecutableSha256: host?.notes?.fixtureExecutableSha256 }, null, 2)}\n`, 'utf8');
    } finally { await output.close(); await host?.close(); }
  }
});
