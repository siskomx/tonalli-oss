import assert from 'node:assert/strict';

// This helper runs only in the caller's owned temporary Core profile.
export async function removeOwnedNearbyCopy(page, host, iterator) {
  await page.getByTestId('nav-content').click();
  const panel = page.getByTestId('nearby-copies'), row = panel.locator('[data-copy-id]').first();
  await row.waitFor(); assert.equal(await row.getByRole('button', { name: 'Remove downloaded content', exact: true }).isDisabled(), true);
  const admin = panel.getByTestId('admin-settings');
  const secret = 'Test-only nearby management passphrase';
  await admin.getByLabel('Admin password', { exact: true }).fill(secret);
  await admin.getByLabel('Confirm password', { exact: true }).fill(secret);
  await admin.getByRole('button', { name: 'Create Admin password', exact: true }).click();
  const deletions = []; const observe = request => { if (request.method() === 'DELETE') deletions.push(request.url()); };
  page.on('request', observe);
  try {
    await row.getByRole('button', { name: 'Remove downloaded content', exact: true }).click();
    await row.getByRole('button', { name: 'Keep', exact: true }).click();
    assert.equal(deletions.length, 0, 'Keep must never submit removal');
    if (process.platform === 'win32') {
      host.stdin.write('hold-copy\n'); assert.equal((await iterator.next()).value, 'copy-held');
    }
    await row.getByRole('button', { name: 'Remove downloaded content', exact: true }).click();
    await row.getByRole('button', { name: 'Confirm removal', exact: true }).click();
    if (process.platform === 'win32') {
      await panel.getByRole('alert').filter({ hasText: 'Removal could not finish.' }).waitFor();
      await row.getByRole('button', { name: 'Keep', exact: true }).click();
      await panel.getByRole('button', { name: 'Refresh', exact: true }).click();
      await row.getByRole('status').filter({ hasText: 'Removal incomplete.' }).waitFor();
      assert.equal(await row.getByRole('link').count(), 0, 'A retired copy cannot be opened');
      host.stdin.write('release-copy\n'); assert.equal((await iterator.next()).value, 'copy-released');
      await row.getByRole('button', { name: 'Finish removal', exact: true }).click();
      await row.getByRole('button', { name: 'Confirm removal', exact: true }).click();
    }
    await panel.getByText('No nearby copies saved yet.', { exact: true }).waitFor();
    assert.equal(deletions.length, process.platform === 'win32' ? 2 : 1);
  } finally { page.off('request', observe); }
}
