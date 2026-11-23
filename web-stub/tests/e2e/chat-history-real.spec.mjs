import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import test from 'node:test';
import { startArticleHost } from './article-host.mjs';

test('private history opens with AI off, retains canonical sources and deletes only after confirmation on real Core', async () => {
  const host = await startArticleHost({ bootstrap: 'request' });
  try {
    const headers = { 'Sec-Fetch-Dest': 'empty', Origin: host.origin };
    const session = await host.context.request.get(`${host.origin}/api/session`, { headers });
    const capability = await session.json();
    assert.equal(capability.private_ai, false);
    assert.equal(capability.private_ai_management, true, 'Use the real CLI compiled with ai; no model or test routes are needed');
    headers['X-Tonalli-Csrf'] = capability.csrf;
    const id = randomUUID().replaceAll('-', '');
    const title = 'Saved <img src=x> conversation';
    const metadata = { pack: 'benign', sources: [{ number: 1, title: 'Welcome article', url: host.manifest.benign.entries.index,
      excerpt: 'Saved article reference' }], citations: [1], flags: { uncited: false, invalid_citations: false, incomplete: false, truncated: false } };
    const written = await host.context.request.put(`${host.origin}/api/ai/history/${id}`, { headers, data: {
      revision: 0, title, model: 'local-test-model', pack: 'benign', messages: [
        { role: 'user', content: 'Open the saved article.', context: true, timestamp: Date.now(), truncated: false },
        { role: 'assistant', content: 'See the article [1].', context: true, timestamp: Date.now(), truncated: false, grounding: metadata },
      ],
    } });
    assert.equal(written.status(), 200);
    await host.page.goto(`${host.origin}/chat`);
    await host.page.getByText('AI is off or unavailable. Choose a downloaded model in Settings to begin.', { exact: true }).waitFor();
    assert.equal(await host.page.getByRole('alert').count(), 0, 'AI off is a normal state, not a duplicate error');
    await host.page.getByText('Saved conversations', { exact: true }).click();
    await host.page.getByRole('button', { name: title, exact: true }).click();
    await host.page.getByText('See the article [1].', { exact: true }).waitFor();
    assert.equal(await host.page.locator('.chat-ui img, .chat-ui script').count(), 0);
    assert.equal(await host.page.getByRole('button', { name: 'Send', exact: true }).isDisabled(), true);
    await host.page.reload();
    await host.page.getByText('Saved conversations', { exact: true }).click();
    await host.page.getByRole('button', { name: title, exact: true }).click();
    await host.page.getByRole('link', { name: '[1] Welcome article', exact: true }).click();
    await host.page.locator('#article-frame').waitFor();
    const frame = await host.page.locator('#article-frame').contentFrame();
    await frame.locator('body').waitFor();
    assert.ok((await frame.locator('body').innerText()).length > 20, 'source opens the real installed ZIM article');
    await host.page.goto(`${host.origin}/chat`);
    await host.page.getByText('Saved conversations', { exact: true }).click();
    await host.page.getByRole('button', { name: `Delete ${title}`, exact: true }).click();
    await host.page.getByRole('button', { name: 'Keep conversation', exact: true }).click();
    assert.equal((await host.context.request.get(`${host.origin}/api/ai/history/${id}`, { headers })).status(), 200);
    await host.page.getByRole('button', { name: `Delete ${title}`, exact: true }).click();
    const removed = host.page.waitForResponse(response => response.request().method() === 'DELETE'
      && new URL(response.url()).pathname === `/api/ai/history/${id}`);
    await host.page.getByRole('button', { name: 'Delete conversation', exact: true }).click();
    assert.equal((await removed).status(), 200);
    await host.page.getByText('No saved conversations yet.', { exact: true }).waitFor();
    assert.equal((await host.context.request.get(`${host.origin}/api/ai/history/${id}`, { headers })).status(), 404);
    assert.deepEqual(host.blocked, []);
  } finally { await host.close(); }
});
