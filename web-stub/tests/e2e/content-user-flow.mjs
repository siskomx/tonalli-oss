import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const english = JSON.parse(await readFile(new URL('../../src/locales/en.json', import.meta.url), 'utf8'));
const spanish = JSON.parse(await readFile(new URL('../../src/locales/es.json', import.meta.url), 'utf8'));
const escaped = text => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

async function words(page) {
  return await page.locator('html').getAttribute('lang') === 'es' ? spanish : english;
}

async function content(page, t) {
  if (!await page.getByTestId('nav-content').isVisible()) await page.getByRole('button', { name: t['app.menu'], exact: true }).click();
  await page.getByTestId('nav-content').click();
  await page.getByRole('heading', { name: t['content.catalog'], exact: true }).waitFor();
}

/** Use the real delivered app's page; this helper supplies no mocked API or catalog. */
export async function downloadCatalogPack(page, title, { pause = true, cancelAndRetry = true, events = [] } = {}) {
  const record = action => events.push({ action, time: new Date().toISOString() });
  const t = await words(page);
  await content(page, t);
  await page.getByRole('button', { name: new RegExp(escaped(title)) }).click();
  const download = page.getByRole('button', { name: t['content.download'], exact: true });
  await download.click();
  const downloads = page.getByRole('heading', { name: t['content.downloads'], exact: true }).locator('..');
  const card = downloads.getByRole('heading', { name: title, exact: true }).locator('../..');
  const progress = card.getByRole('progressbar');
  await progress.waitFor({ timeout: 20000 });
  record('visible_progress');
  const observation = { title, visibleProgress: true, pause: false, cancelled: false, resumed: false };
  if (pause) {
    await card.getByRole('button', { name: t['content.action.pause'], exact: true }).click();
    record('pause_requested');
    await card.getByText(t['content.state.paused'], { exact: true }).waitFor({ timeout: 90000 });
    record('paused');
    observation.pause = true;
    if (cancelAndRetry) {
      await card.getByRole('button', { name: t['content.action.cancel'], exact: true }).click();
      await card.getByRole('button', { name: t['content.confirmCancel'], exact: true }).click();
      record('cancel_requested');
      await card.getByText(t['content.state.cancelled'], { exact: true }).waitFor({ timeout: 90000 });
      record('cancelled');
      observation.cancelled = true;
      await card.getByRole('button', { name: t['content.action.retry'], exact: true }).click();
      record('retry_requested');
      await card.getByRole('button', { name: t['content.action.pause'], exact: true }).click();
      record('second_pause_requested');
      await card.getByText(t['content.state.paused'], { exact: true }).waitFor({ timeout: 90000 });
      record('paused_after_retry');
    }
    await card.getByRole('button', { name: t['content.action.resume'], exact: true }).click();
    record('resume_requested');
    observation.resumed = true;
  }
  await card.getByText(t['content.state.completed'], { exact: true }).waitFor({ timeout: 180000 });
  record('completed');
  observation.completed = true;
  await page.getByRole('tab', { name: new RegExp(`^${escaped(t['content.tab.installed'])}`) }).click();
  const installed = page.getByRole('listitem').filter({ has: page.getByRole('heading', { name: title, exact: true }) });
  await installed.getByRole('link', { name: t['content.openLibrary'], exact: true }).waitFor();
  return observation;
}

/** Run again after ordinary app restart to prove the installed content reopens. */
export async function openDownloadedArticle(page, title, query, expectedTitle = query) {
  const t = await words(page);
  await content(page, t);
  await page.getByRole('tab', { name: new RegExp(`^${escaped(t['content.tab.installed'])}`) }).click();
  const installed = page.getByRole('listitem').filter({ has: page.getByRole('heading', { name: title, exact: true }) });
  await installed.getByRole('link', { name: t['content.openLibrary'], exact: true }).click();
  await page.getByTestId('library-row').first().waitFor({ timeout: 30000 });
  await page.getByRole('searchbox', { name: t['library.search'], exact: true }).fill(query);
  const suggestion = page.getByRole('link', { name: expectedTitle, exact: true });
  await suggestion.waitFor({ timeout: 20000 });
  const articleTitle = await suggestion.textContent();
  await suggestion.click();
  await page.locator('#article-status').filter({ hasText: t['article.ready'] }).waitFor({ timeout: 30000 });
  const article = page.frameLocator('#article-frame');
  await article.locator('body').waitFor({ timeout: 30000 });
  const text = await article.locator('body').innerText();
  assert.ok(text.trim().length > 100, 'a real article must render, not a shell/loading page');
  assert.ok(new URL(page.url()).pathname === '/article');
  return { articleTitle, renderedCharacters: text.length, canonicalArticleOpened: true };
}
