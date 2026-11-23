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

test('paired app saves a real map and renders its local copy after Core restart', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'tonalli-map-copy-')), receipt = join(temporary, 'private-ipc.json');
  const host = spawn(resolve('../target/debug/examples/remote-library-check.exe'), [resolve('dist'), receipt,
    resolve(process.env.TONALLI_COPY_MAP || '../docs/spike/packs/mx-mexico-city-z8.pmtiles')], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const iterator = createInterface({ input: host.stdout })[Symbol.asyncIterator]();
  assert.equal((await iterator.next()).value, 'ready');
  const info = JSON.parse(await readFile(receipt, 'utf8'));
  const browser = await chromium.launch({ executablePath: process.env.TONALLI_ARTICLE_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', headless: true });
  const page = await browser.newPage(), errors = []; page.setDefaultTimeout(15000); page.on('pageerror', e => errors.push(e.message));
  const mapReady = async () => {
    await page.waitForFunction(() => {
      const raw = document.querySelector('#map')?.dataset.renderEvidence;
      return raw && JSON.parse(raw).labels.length > 0;
    });
  };
  try {
    await page.goto(info.launch); await page.getByRole('link', { name: 'Connect to another device', exact: true }).click();
    const card = page.getByTestId('remote-library');
    await card.locator('[name="library-invitation"]').fill(info.invitation);
    await card.locator('code').filter({ hasText: info.fingerprint }).waitFor();
    await card.locator('[name="library-pairing-code"]').fill(info.code);
    await card.locator('[name="library-device-label"]').fill('Owned map reader');
    await card.getByRole('checkbox').check(); await card.getByRole('button', { name: 'Connect to another device', exact: true }).click();
    await page.getByTestId('nav-map').click(); await mapReady();
    const copy = page.getByTestId('remote-copy');
    await copy.getByRole('button', { name: 'Save map for offline use', exact: true }).click();
    await copy.getByRole('status').filter({ hasText: 'Completed' }).waitFor();
    await copy.getByRole('button', { name: 'Open in Maps', exact: true }).click();
    await page.getByTestId('nav-notes').waitFor(); await mapReady();
    const localId = new URL(page.url()).searchParams.get('pack');
    assert.match(localId, /^copy-[a-f0-9]{64}$/);
    await page.reload(); await mapReady();
    host.stdin.write('reopen\n'); assert.equal((await iterator.next()).value, 'reopened');
    const reopened = JSON.parse(await readFile(receipt, 'utf8')); await page.goto(reopened.launch);
    await page.getByTestId('nav-map').click(); await mapReady();
    await page.locator('#pack-select').selectOption(localId);
    assert.equal(new URL(page.url()).searchParams.get('pack'), localId);
    await page.reload(); await mapReady(); assert.equal(await page.locator('#pack-select').inputValue(), localId);
    assert.equal(await page.getByTestId('remote-copy').count(), 0); assert.deepEqual(errors, []);
    await removeOwnedNearbyCopy(page, host, iterator);
    host.stdin.write('reopen\n'); assert.equal((await iterator.next()).value, 'reopened');
    const removed = JSON.parse(await readFile(receipt, 'utf8')); await page.goto(removed.launch);
    await page.getByTestId('nav-map').click(); await page.getByText('No map packs installed', { exact: true }).waitFor();
    assert.deepEqual(errors, []);
  } finally {
    await browser.close(); const exit = once(host, 'exit'); host.stdin.end('stop\n'); await exit;
    assert.ok(temporary.startsWith(join(tmpdir(), 'tonalli-map-copy-')) && receipt.startsWith(temporary + sep));
    await rm(temporary, { recursive: true, force: true });
  }
});
