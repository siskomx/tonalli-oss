import assert from 'node:assert/strict';
import { stat, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { redact, root, sha256, startArticleHost } from './article-host.mjs';
import { verifiedReactPacks } from './react-fixtures.mjs';

// Bounded visual evidence only; screenshots run after readiness, outside timings.
const dataset = await verifiedReactPacks();
const captures = [];
let english;
let spanish;

async function ready(host, route) {
  await host.page.waitForFunction((route) => window.__ready?.route === `/${route}`
    && (window.__ready.ready || window.__ready.error), route, { timeout: 20000 });
  const value = await host.page.evaluate(() => window.__ready);
  assert.equal(value.error, null);
  assert.equal(value.ready, true);
  return value;
}

async function capture(host, name, details = {}) {
  const file = `docs/spike/${name}.png`;
  const path = join(root, file);
  assert.equal(host.blocked.length, 0);
  await host.page.screenshot({ path, fullPage: true, animations: 'disabled' });
  captures.push({ file, bytes: (await stat(path)).size, sha256: await sha256(path),
    url: redact(host.page.url()), cliSha256: host.cliSha256, distSha256: host.distSha256,
    readiness: await host.page.evaluate(() => window.__ready), ...details });
}

try {
  english = await startArticleHost({ extraPacks: dataset.packs,
    titleIndexes: [{ id: 'wiki', file: 'target/full-wikipedia.titles.idx' }], startupTimeoutMs: 60000 });
  await english.page.goto(`${english.origin}/library?pack=wiki`, { waitUntil: 'load' });
  await ready(english, 'library');
  await english.page.getByTestId('library-list').evaluate((element) => {
    element.scrollTop = 99996 * Number(element.dataset.rowHeight);
  });
  await english.page.waitForFunction(() => document.querySelector('[data-testid="library-row"][data-index="99999"]')?.dataset.url
    === 'C/17_December_2013_arrests_in_Turkey');
  await ready(english, 'library');
  await capture(english, 'react-library', { actualRow: 99999,
    rowUrl: 'C/17_December_2013_arrests_in_Turkey', renderedRows: await english.page.getByTestId('library-row').count() });

  await english.page.goto(`${english.origin}/article?${new URLSearchParams({ pack: 'wiki', url: 'C/2013_corruption_scandal_in_Turkey' })}`,
    { waitUntil: 'load' });
  assert.equal((await ready(english, 'article')).article.resourcesLoaded, true);
  await capture(english, 'react-article');

  await english.page.goto(`${english.origin}/map`, { waitUntil: 'load' });
  await ready(english, 'map');
  await english.page.locator('#pack-select').selectOption('mx');
  await english.page.waitForFunction(() => window.__ready?.ready && window.__ready.map?.rendered
    && window.__ready.map.selected === 'mx');
  await capture(english, 'react-map');

  spanish = await startArticleHost({ locale: 'es-MX' });
  await spanish.page.goto(`${spanish.origin}/article?${new URLSearchParams({ pack: 'benign', url: 'C/es/Niñez_y_educación.html' })}`,
    { waitUntil: 'load' });
  assert.equal((await ready(spanish, 'article')).article.resourcesLoaded, true);
  assert.equal(await spanish.page.locator('html').getAttribute('lang'), 'es');
  await capture(spanish, 'react-spanish', { locale: 'es-MX', effectiveLanguage: 'es', dataset: 'official-writer benign-v1 fixture' });
  assert.equal(english.blocked.length, 0);
  assert.equal(spanish.blocked.length, 0);
  assert.equal(english.distSha256, spanish.distSha256);
  await writeFile(join(root, 'docs/spike/react-visual-trace.json'), `${JSON.stringify({
    kind: 'unmodified post-readiness screenshots; no performance series',
    browserVersion: english.context.browser().version(), browserPath: english.browserPath,
    headless: true, viewport: { width: 1280, height: 900 }, dataset: dataset.receipt,
    externalAttempts: [...english.blocked, ...spanish.blocked], captures,
  }, null, 2)}\n`, 'utf8');
  console.log(JSON.stringify(captures.map(({ file, bytes, sha256, distSha256 }) => ({ file, bytes, sha256, distSha256 })), null, 2));
} finally {
  await spanish?.close();
  await english?.close();
}
