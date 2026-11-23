import assert from 'node:assert/strict';
import { destination } from './consumer-gui-flow.mjs';

// Only run in an owned synthetic profile or after verifying no Admin state exists.
const password = 'Synthetic Admin check 2026';
async function openAdvanced(page) {
  await destination(page, 'settings');
  await page.getByRole('tab', { name: 'Advanced', exact: true }).click();
}
export async function adminFlow(page, stage) {
  await openAdvanced(page);
  const panel = page.getByTestId('admin-settings');
  const create = panel.getByRole('button', {name:'Create Admin password',exact:true});
  const unlock = panel.getByRole('button', {name:'Unlock Admin',exact:true});
  await (stage === 'setup' ? create : unlock).waitFor();
  if (stage === 'setup') {
    await panel.getByLabel('Admin password', {exact:true}).fill(password);
    await panel.getByLabel('Confirm password', {exact:true}).fill(password + ' mismatch');
    await create.click();
    await panel.getByRole('alert').filter({hasText:'The passwords do not match.'}).waitFor();
    await panel.getByLabel('Confirm password', {exact:true}).fill(password);
    await create.click();
  } else {
    if (stage !== 'prepare') {
      await page.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
      assert.equal(await page.getByTestId('nav-map').count(), 0);
      assert.equal(await page.getByTestId('nav-notes').count(), 0);
    }
    await panel.getByLabel('Admin password', {exact:true}).fill('wrong synthetic password');
    await unlock.click();
    await panel.getByRole('alert').filter({hasText:'Incorrect Admin password.'}).waitFor();
    await new Promise(resolve => setTimeout(resolve, 2100));
    await panel.getByLabel('Admin password', {exact:true}).fill(password);
    await unlock.click();
  }
  const appearance = panel.getByLabel('Appearance', {exact:true}); await appearance.waitFor();
  assert.equal(await panel.locator('input[type=password]').count(), 0);
  const dark = stage !== 'restart';
  await appearance.selectOption(dark ? 'dark' : 'system');
  await panel.getByRole('checkbox', {name:'Maps',exact:true}).setChecked(!dark);
  await panel.getByRole('checkbox', {name:'Notes',exact:true}).setChecked(!dark);
  await panel.getByRole('button', {name:'Save settings',exact:true}).click();
  await panel.getByRole('status').filter({hasText:'Settings saved on this device.'}).waitFor();
  if (dark) {
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
    assert.equal(await page.getByTestId('nav-map').count(), 0);
    await destination(page, 'library'); await openAdvanced(page);
    await unlock.waitFor(); assert.equal(await appearance.count(), 0);
  } else {
    await panel.getByRole('button', {name:'Lock Admin',exact:true}).click(); await unlock.waitFor();
  }
  assert.equal(await page.getByRole('alert').count(), 0);
  return {stage, setup:stage==='setup', wrongPassword:stage!=='setup', themeAndMenuSaved:true, navigationLocks:true};
}
