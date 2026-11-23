import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { root, startArticleHost } from './article-host.mjs';

const en = JSON.parse(await readFile(join(root, 'web-stub/src/locales/en.json'), 'utf8'));
const evidence = [];
let host;

test.before(async () => { host = await startArticleHost({ bootstrap: 'request' }); });
test.after(async () => {
  if (!host) return;
  try {
    assert.deepEqual(host.blocked, [], 'Library must not attempt external requests');
    const directory = join(root, 'target/library-body-browser-r1');
    await mkdir(directory, { recursive: true });
    await writeFile(join(directory, 'result.json'), JSON.stringify({ label: 'desktop',
      cliSha256: host.cliSha256, distSha256: host.distSha256,
      browserVersion: host.context.browser().version(), evidence,
      limits: 'Missing-index case uses real Core. Delayed successful body responses are browser-intercepted fixtures, not index/query proof.' }, null, 2));
  } finally { await host.close(); }
});

async function library() {
  await host.page.goto(`${host.origin}/library?pack=benign`);
  await host.page.getByTestId('library-row').first().waitFor();
}
const mode = () => host.page.getByLabel(en['library.searchMode'], { exact: true });
const bodyInput = () => host.page.getByLabel(en['library.bodySearch'], { exact: true });

test('missing real index explains setup and title mode recovers', async () => {
  await library();
  await mode().selectOption('body');
  assert.equal(await host.page.getByTestId('library-list').count(), 0);
  const response = host.page.waitForResponse(response => new URL(response.url()).pathname === '/api/zim/search');
  await bodyInput().fill('fixture');
  assert.equal((await response).status(), 503);
  await host.page.getByRole('alert').filter({ hasText: en['library.bodyIndexMissing'] }).waitFor();
  await mode().selectOption('titles');
  await host.page.getByTestId('library-row').first().waitFor();
  assert.equal(await host.page.getByRole('alert').count(), 0);
  evidence.push({ case: 'real missing index and title recovery', passed: true });
});

test('late body responses cannot replace a newer query or restored title mode', async () => {
  await library();
  const requests = [];
  const held = new Map();
  const routePattern = '**/api/zim/search?*';
  const handler = async route => {
    const query = new URL(route.request().url()).searchParams.get('q');
    requests.push(query);
    if (query !== 'current') await new Promise(resolve => held.set(query, resolve));
    try {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ pack: 'benign',
        hits: [{ title: `Result ${query}`, url: 'C/en/index.html', score: 1 }] }) });
    } catch (error) {
      // A cancelled request may already be gone when its held fixture is released.
      assert.ok(route.request().failure(), `Unexpected route failure: ${error.message}`);
    }
  };
  await host.page.route(routePattern, handler);
  try {
    await mode().selectOption('body');
    await bodyInput().fill('old');
    await host.page.waitForRequest(request => new URL(request.url()).searchParams.get('q') === 'old');
    await bodyInput().fill('current');
    await host.page.getByRole('link', { name: 'Result current', exact: true }).waitFor();
    assert.ok(held.has('old'));
    held.get('old')();
    await host.page.waitForLoadState('networkidle');
    assert.equal(await host.page.getByRole('link', { name: 'Result old', exact: true }).count(), 0);
    await bodyInput().fill('departed');
    await host.page.waitForRequest(request => new URL(request.url()).searchParams.get('q') === 'departed');
    await mode().selectOption('titles');
    await host.page.getByTestId('library-row').first().waitFor();
    assert.ok(held.has('departed'));
    held.get('departed')();
    await host.page.waitForLoadState('networkidle');
    assert.equal(await host.page.getByRole('link', { name: 'Result departed', exact: true }).count(), 0);
    assert.equal(await mode().inputValue(), 'titles');
    evidence.push({ case: 'intercepted body query/departure races', requests, passed: true });
  } finally {
    for (const release of held.values()) release();
    await host.page.unroute(routePattern, handler);
  }
});
