import assert from 'node:assert/strict';
import { destination, appWords } from './consumer-gui-flow.mjs';

// Use an ordinary installed public book; never select personal notes or files.
export async function sharingHost(page, { title = 'Knots by Wikipedia', start = true } = {}) {
  const t = await appWords(page);
  await destination(page, 'settings');
  await page.getByRole('tab', { name: t['settings.tab.sharing'], exact: true }).click();
  const panel = page.getByTestId('sharing');
  await panel.getByRole('status').filter({ hasText: t['sharing.off'] }).waitFor();
  if (!start) return { off: true };
  await panel.getByRole('checkbox', { name: title, exact: true }).first().check();
  const networks = panel.getByRole('combobox');
  if (await networks.count()) await networks.selectOption({ label: 'Wi-Fi' });
  await panel.getByRole('button', { name: t['sharing.start'], exact: true }).click();
  await panel.getByRole('status').filter({ hasText: t['sharing.on'] }).waitFor({ timeout: 30000 });
  const url = await panel.getByTestId('sharing-address').getAttribute('href');
  assert.match(new URL(url).hostname, /^tonalli-[a-f0-9]{12}\.local$/);
  const code = await panel.getByTestId('sharing-code').innerText();
  await panel.getByText(t['sharing.certificate'], {exact:true}).click();
  const fingerprint = await panel.locator('code').innerText();
  const parsed = new URL(url);
  assert.equal(parsed.protocol, 'https:'); assert.equal(parsed.pathname, '/pair'); assert.ok(parsed.port);
  assert.ok(code.length > 0);
  assert.match(fingerprint, /^(?:[A-F0-9]{2}:){31}[A-F0-9]{2}$/);
  return { url, code, fingerprint, title };
}

export async function stopSharing(page) {
  const t = await appWords(page);
  const panel = page.getByTestId('sharing');
  await panel.getByRole('button', { name: t['sharing.stop'], exact: true }).click();
  await panel.getByRole('status').filter({ hasText: t['sharing.off'] }).waitFor();
}
