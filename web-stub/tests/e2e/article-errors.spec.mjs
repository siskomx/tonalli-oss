import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { redact, root, startArticleHost } from './article-host.mjs';

// Deliberate HTTP fault injection only. The article metadata is obtained from
// the real writer-generated archive, and normal sandbox/CSP remain in force.
const csp = "default-src 'none'; script-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; media-src 'self'; object-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'";
let host;
const samples = [];
test.before(async () => { host = await startArticleHost(); });
test.after(async () => {
  if (!host) return;
  try {
    await writeFile(process.env.TONALLI_ARTICLE_ERROR_TRACE || join(root, 'docs/spike/article-error-trace.json'),
      `${JSON.stringify({ kind: 'controlled HTTP error injection; real metadata; no fixture substitution',
        cliSha256: host.cliSha256, distSha256: host.distSha256,
        browserVersion: host.context.browser().version(), samples }, null, 2)}\n`, 'utf8');
  } finally { await host.close(); }
});

for (const status of [401, 503]) {
  test(`mounted HTML HTTP ${status} is an error after successful real article metadata`, async () => {
    const mounted = `${host.origin}/zim/benign/C/en/index.html`;
    const handler = (route) => route.fulfill({ status, contentType: 'text/html; charset=utf-8',
      headers: { 'Content-Security-Policy': csp, 'X-Content-Type-Options': 'nosniff', 'Cache-Control': 'no-store' },
      body: '<!doctype html><title>Injected HTTP error</title><p id="injected-error">HTTP failure</p>' });
    await host.page.route(mounted, handler);
    try {
      const metadata = host.page.waitForResponse((response) => new URL(response.url()).pathname === '/api/zim/article');
      await host.page.goto(`${host.origin}/article?${new URLSearchParams({ pack: 'benign', url: 'C/en/index.html' })}`, { waitUntil: 'load' });
      assert.equal((await metadata).status(), 200, 'real metadata must succeed before the content fault');
      await host.page.waitForFunction(() => window.__ready?.ready || window.__ready?.error);
      const state = await host.page.evaluate(() => window.__ready);
      const frame = host.page.frames().find((frame) => frame !== host.page.mainFrame() && frame.url().startsWith(mounted));
      const navigation = await frame.evaluate(() => {
        const entry = performance.getEntriesByType('navigation')[0];
        return { responseStatus: entry?.responseStatus, mime: document.contentType };
      });
      samples.push({ injectedStatus: status, metadataStatus: 200, mounted: redact(mounted), navigation, state });
      assert.equal(navigation.responseStatus, status);
      assert.equal(state.ready, false, 'an iframe HTTP error must not be Article ready');
      assert.ok(state.error, 'the React route must expose its content failure');
      assert.notEqual(state.article?.resourcesLoaded, true);
      assert.equal(await host.page.locator('#article-frame').getAttribute('sandbox'), 'allow-same-origin');
      assert.equal(host.blocked.length, 0);
    } finally { await host.page.unroute(mounted, handler); }
  });
}
