import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

export async function appWords(page) {
  const language = await page.locator('html').getAttribute('lang');
  return JSON.parse(await readFile(new URL(`../../src/locales/${language === 'es' ? 'es' : 'en'}.json`, import.meta.url), 'utf8'));
}

export async function destination(page, name) {
  const t = await appWords(page);
  const link = page.getByTestId(`nav-${name}`);
  if (!await link.isVisible()) await page.getByRole('button', { name: t['app.menu'], exact: true }).click();
  await link.click();
  await page.waitForFunction(name => window.__ready?.route === `/${name}` && window.__ready.ready, name);
}

// Run only with a fresh, task-owned profile; never create/reset a user's existing vault.
export async function firstUseNotes(page) {
  const t = await appWords(page);
  await destination(page, 'notes');
  const setup = page.locator('details').filter({ has: page.getByText(t['notes.create'], { exact: true }).first() });
  await setup.locator('summary').click();
  await setup.getByLabel(t['notes.passphrase'], { exact: true }).fill('tonalli-gui-fixture-only');
  await setup.getByLabel(t['notes.confirmPassphrase'], { exact: true }).fill('tonalli-gui-fixture-only');
  await page.getByTestId('notes-create').click();
  const recovery = await page.getByTestId('notes-recovery-key').innerText();
  assert.match(recovery, /^[0-9a-f]{64}$/i);
  assert.equal(await page.getByTestId('notes-title').count(), 0, 'key confirmation precedes editor');
  await page.getByRole('button', { name: t['notes.savedKey'], exact: true }).click();
  await page.getByTestId('notes-title').fill('Offline checklist');
  await page.getByTestId('notes-body').fill('Water, maps, and a book to read.');
  await page.getByTestId('notes-add').click();
  await page.getByTestId('notes-selected').waitFor();
  await page.getByTestId('notes-lock').click();
  await page.getByTestId('notes-credential').waitFor();
  await page.getByTestId('notes-credential').fill('wrong-fixture-passphrase');
  await page.getByTestId('notes-unlock').click();
  await page.getByRole('alert').filter({ hasText: t['notes.error.unlock_failed'] }).waitFor();
  await page.getByTestId('notes-credential').fill(recovery);
  await page.getByLabel(t['notes.unlockWith']).selectOption('recovery_key');
  // Changing unlock method clears the previous input.
  await page.getByTestId('notes-credential').fill(recovery);
  await page.getByTestId('notes-unlock').click();
  await page.getByTestId('notes-row').filter({ hasText: 'Offline checklist' }).click();
  assert.equal(await page.getByTestId('notes-selected').locator('p').innerText(), 'Water, maps, and a book to read.');
  await destination(page, 'library');
  await destination(page, 'notes');
  await page.getByTestId('notes-credential').waitFor();
  assert.equal(await page.getByTestId('notes-selected').count(), 0);
  return { create: true, recoveryConfirmed: true, save: true, wrongPassphraseRefused: true, recoveryUnlock: true, departureClears: true };
}
