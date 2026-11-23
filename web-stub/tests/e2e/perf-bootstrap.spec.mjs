import assert from 'node:assert/strict';
import test from 'node:test';
import { startArticleHost } from './article-host.mjs';

test('production performance bootstrap authenticates without executing app HTML', async () => {
  const host = await startArticleHost({ bootstrap: 'request' });
  try {
    assert.equal(host.cliSha256, '5f0938f1c2d55819377a5d3984c9c85eda4dc68af17a60fa4ea087c81feb4031');
    assert.equal(host.page.url(), 'about:blank');
    assert.equal(host.baseline.launch, 302);
    assert.equal(host.baseline.session, 200);
    const successful = new Set();
    host.page.on('response', (response) => {
      if (response.status() === 200) successful.add(new URL(response.url()).pathname);
    });
    await host.page.goto(`${host.origin}/library?pack=benign`, { waitUntil: 'load' });
    await host.page.waitForFunction(() => window.__ready?.route === '/library' && window.__ready.ready);
    assert.ok(successful.has('/api/session'));
    assert.ok(successful.has('/api/zim/titles'));
    assert.equal(host.blocked.length, 0);
  } finally { await host.close(); }
});
