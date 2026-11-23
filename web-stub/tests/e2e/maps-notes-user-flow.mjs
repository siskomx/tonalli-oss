import assert from 'node:assert/strict';
import { destination, appWords } from './consumer-gui-flow.mjs';

export const fixturePassphrase = 'tonalli-maps-notes-recovered-only';
export async function notesManagement(page, { fresh = true } = {}) {
  const t = await appWords(page);
  await destination(page,'notes');
  if (fresh === false) {
    await page.getByTestId('notes-credential').fill(fixturePassphrase);
    await page.getByTestId('notes-unlock').click();
    await page.getByTestId('notes-row').filter({hasText:'Offline checklist — México'}).click();
    assert.equal(await page.getByTestId('notes-selected').locator('p').innerText(),'Water, maps, café y un libro.');
    await page.getByTestId('notes-lock').click();
    return { restartUnlock: true, editedNoteRetained: true };
  }
  let recovery;
  let currentPassphrase = 'tonalli-maps-notes-fixture-only';
  if (fresh === true) {
  const setup = page.locator('details').filter({has:page.getByText(t['notes.create'],{exact:true}).first()});
  if (!await setup.evaluate(element=>element.open)) await setup.locator('summary').click();
  await setup.getByLabel(t['notes.passphrase'],{exact:true}).fill('tonalli-maps-notes-fixture-only');
  await setup.getByLabel(t['notes.confirmPassphrase'],{exact:true}).fill('tonalli-maps-notes-fixture-only');
  await page.getByTestId('notes-create').click();
  recovery = await page.getByTestId('notes-recovery-key').innerText();
  assert.match(recovery,/^[a-f0-9]{64}$/);
  await page.getByRole('button',{name:t['notes.savedKey'],exact:true}).click();
  await page.getByTestId('notes-title').fill('Offline checklist');
  await page.getByTestId('notes-body').fill('Water and a map.');
  await page.getByTestId('notes-add').click();
  } else {
    if (await page.getByTestId('notes-lock').isVisible()) await page.getByTestId('notes-lock').click();
    for (const candidate of ['tonalli-maps-notes-fixture-only','tonalli-maps-notes-changed-only',fixturePassphrase]) {
      await page.getByLabel(t['notes.unlockWith']).selectOption('passphrase');
      await page.getByTestId('notes-credential').fill(candidate);
      await page.getByTestId('notes-unlock').click();
      await page.waitForFunction(()=>document.querySelector('[data-testid="notes-lock"]') || document.querySelector('[role="alert"]'));
      if (await page.getByTestId('notes-lock').isVisible()) { currentPassphrase=candidate;break; }
    }
    await page.getByTestId('notes-row').filter({hasText:'Offline checklist — México'}).click();
  }
  await page.getByTestId('notes-selected').waitFor();
  await page.getByTestId('notes-edit').click();
  await page.getByTestId('notes-title').fill('Offline checklist — México');
  await page.getByTestId('notes-body').fill('Water, maps, café y un libro.');
  await page.getByTestId('notes-add').click();
  await page.getByTestId('notes-selected').getByRole('heading',{name:'Offline checklist — México',exact:true}).waitFor();
  const security = page.locator('details').filter({has:page.getByText(t['notes.protection'],{exact:true})});
  await security.locator('summary').click();
  await security.getByLabel(t['notes.currentPassphrase'],{exact:true}).fill(currentPassphrase);
  await page.getByTestId('notes-export-recovery').click();
  const exported = await page.getByTestId('notes-recovery-key').innerText();
  assert.match(exported,/^[a-f0-9]{64}$/);
  if (recovery) assert.equal(exported,recovery); else recovery=exported;
  await page.getByRole('button',{name:t['notes.savedKey'],exact:true}).click();
  const protection = page.locator('details').filter({has:page.getByText(t['notes.protection'],{exact:true})});
  if (!await protection.evaluate(element=>element.open)) await protection.locator('summary').click();
  await protection.getByLabel(t['notes.currentPassphrase'],{exact:true}).fill(currentPassphrase);
  await protection.getByLabel(t['notes.newPassphrase'],{exact:true}).fill('tonalli-maps-notes-changed-only');
  await protection.getByLabel(t['notes.confirmPassphrase'],{exact:true}).fill('tonalli-maps-notes-changed-only');
  await page.getByTestId('notes-change-passphrase').click();
  await page.getByRole('status').filter({hasText:t['notes.passphraseChanged']}).waitFor();
  await page.getByTestId('notes-lock').click();
  await page.getByTestId('notes-credential').fill('tonalli-maps-notes-fixture-only');
  await page.getByTestId('notes-unlock').click();
  await page.getByRole('alert').filter({hasText:t['notes.error.unlock_failed']}).waitFor();
  await page.getByTestId('notes-credential').fill('tonalli-maps-notes-changed-only');
  await page.getByTestId('notes-unlock').click();
  await page.getByTestId('notes-lock').waitFor();
  const protect = page.locator('details').filter({has:page.getByText(t['notes.protection'],{exact:true})});
  await protect.locator('summary').click();
  await protect.getByLabel(t['notes.confirmWith']).selectOption('recovery_key');
  await protect.getByLabel(t['notes.recovery'],{exact:true}).fill(recovery);
  await protect.getByLabel(t['notes.newPassphrase'],{exact:true}).fill(fixturePassphrase);
  await protect.getByLabel(t['notes.confirmPassphrase'],{exact:true}).fill(fixturePassphrase);
  await page.getByTestId('notes-change-passphrase').click();
  await page.getByRole('status').filter({hasText:t['notes.passphraseChanged']}).waitFor();
  await page.getByTestId('notes-title').fill('Synthetic deletion fixture');
  await page.getByTestId('notes-body').fill('Only this task-created note is deleted.');
  await page.getByTestId('notes-add').click();
  await page.getByTestId('notes-selected').getByRole('button',{name:t['notes.delete'],exact:true}).click();
  await page.getByTestId('notes-confirm-delete').click();
  await page.getByTestId('notes-search').fill('México');
  await page.getByTestId('notes-search-submit').click();
  await page.getByTestId('notes-row').filter({hasText:'Offline checklist — México'}).click();
  assert.equal(await page.getByTestId('notes-selected').locator('p').innerText(),'Water, maps, café y un libro.');
  await page.getByTestId('notes-lock').click();
  assert.equal(await page.getByTestId('notes-selected').count(),0);
  return {create:fresh===true,reusedFixture:fresh!==true,edit:true,exportRecovery:true,changePassphrase:true,oldPassphraseRefused:true,recoveryReset:true,deleteSynthetic:true,search:true,lockClears:true};
}

export async function countryDownload(page, id, { pause = false } = {}) {
  const t = await appWords(page);
  await destination(page,'map');
  await page.getByRole('button',{name:t[`map.country.${id}`]}).click();
  const panel = page.locator('details').filter({has:page.getByText(t['map.downloadCountries'],{exact:true})});
  const cancel = panel.getByRole('button',{name:t['map.cancel'],exact:true});
  if (await cancel.isVisible() && await cancel.isEnabled()) {await cancel.click();await cancel.waitFor({state:'hidden',timeout:90000});}
  await panel.getByRole('combobox').selectOption('8');
  await panel.getByRole('button',{name:t['map.checkSize'],exact:true}).click();
  await panel.getByRole('button',{name:t['map.download'],exact:true}).waitFor({timeout:180000});
  await panel.getByRole('button',{name:t['map.download'],exact:true}).click();
  if (pause) {
    await panel.getByRole('button',{name:t['map.pause'],exact:true}).click();
    await panel.getByRole('button',{name:t['map.resume'],exact:true}).waitFor({timeout:90000});
    await panel.getByRole('button',{name:t['map.resume'],exact:true}).click();
  }
  await panel.getByRole('status').filter({hasText:t['map.job.completed']}).waitFor({timeout:300000});
  await page.locator('#pack-select option').filter({hasText:t[`map.country.${id}`]}).waitFor({state:'attached',timeout:30000});
  await page.locator('#pack-select').selectOption({label:t[`map.country.${id}`]});
  await page.waitForFunction(()=>document.querySelector('#map')?.dataset.renderEvidence?.includes('labels'),null,{timeout:60000});
  return {country:id,download:true,pausedAndResumed:pause,installedAndRendered:true};
}
