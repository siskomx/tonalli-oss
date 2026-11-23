import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { once } from 'node:events';
import { resolve, join, sep } from 'node:path';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { chromium } from 'playwright-core';
import { removeOwnedNearbyCopy } from './nearby-management-flow.mjs';

test('paired app saves a real shared archive and reads its local copy after Core restart', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'tonalli-copy-')), receipt = join(temporary, 'private-ipc.json');
  const host = spawn(resolve('../target/debug/examples/remote-library-check.exe'), [resolve('dist'), receipt], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const iterator = createInterface({ input: host.stdout })[Symbol.asyncIterator]();
  assert.equal((await iterator.next()).value, 'ready');
  const info = JSON.parse(await readFile(receipt, 'utf8'));
  const browser = await chromium.launch({ executablePath: process.env.TONALLI_ARTICLE_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', headless: true });
  const page = await browser.newPage(), errors = []; page.setDefaultTimeout(15000); page.on('pageerror', e => errors.push(e.message));
  try {
    await page.goto(info.launch); await page.getByRole('link', { name: 'Connect to another device', exact: true }).click();
    const card = page.getByTestId('remote-library');
    await card.locator('[name="library-invitation"]').fill(info.invitation);
    await card.locator('code').filter({ hasText: info.fingerprint }).waitFor();
    await card.locator('[name="library-pairing-code"]').fill(info.code);
    await card.locator('[name="library-device-label"]').fill('Owned offline reader');
    await card.getByRole('checkbox').check(); await card.getByRole('button', { name: 'Connect to another device', exact: true }).click();
    const copy = page.getByTestId('remote-copy');
    await copy.getByRole('button', { name: 'Save for offline reading', exact: true }).click();
    await copy.getByRole('status').filter({ hasText: 'Completed' }).waitFor();
    await copy.getByRole('button', { name: 'Open in Library', exact: true }).click();
    await page.getByTestId('nav-notes').waitFor();
    assert.equal(await page.getByText('Reading a nearby library', { exact: true }).count(), 0);
    await page.getByTestId('library-row').first().click();
    const article = () => page.waitForFunction(() => document.querySelector('iframe')?.contentDocument?.body?.textContent?.trim().length > 0);
    await article(); await page.reload(); await article();
    host.stdin.write('reopen\n'); assert.equal((await iterator.next()).value, 'reopened');
    const reopened = JSON.parse(await readFile(receipt, 'utf8')); await page.goto(reopened.launch);
    await page.getByTestId('library-row').first().click(); await article();
    assert.equal(await page.getByTestId('remote-copy').count(), 0); assert.deepEqual(errors, []);
    await removeOwnedNearbyCopy(page, host, iterator);
    host.stdin.write('reopen\n'); assert.equal((await iterator.next()).value, 'reopened');
    const removed = JSON.parse(await readFile(receipt, 'utf8')); await page.goto(removed.launch);
    await page.getByTestId('nav-content').click(); await page.getByTestId('nearby-copies').getByText('No nearby copies saved yet.', { exact: true }).waitFor();
    assert.deepEqual(errors, []);
  } finally {
    await browser.close(); const exit = once(host, 'exit'); host.stdin.end('stop\n'); await exit;
    assert.ok(temporary.startsWith(join(tmpdir(), 'tonalli-copy-')) && receipt.startsWith(temporary + sep));
    await rm(temporary, { recursive: true, force: true });
  }
});
