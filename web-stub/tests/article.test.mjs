import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const metadata = { pack: 'benign', title: 'Niñez y educación', url: 'C/es/Niñez_y_educación.html', mime_type: 'text/html',
  iframe_url: '/zim/benign/C/es/Ni%C3%B1ez_y_educaci%C3%B3n.html' };

test('article metadata fetch encodes paths and uses full API credentials', async () => {
  const { loadArticleMetadata } = await import('../src/features/article/client.js');
  let request;
  const result = await loadArticleMetadata({ pack: 'benign', url: metadata.url }, 'session-csrf', async (url, options) => {
    request = { url, options };
    return Response.json(metadata);
  });
  assert.deepEqual(result, metadata);
  const url = new URL(request.url, 'http://127.0.0.1:1234');
  assert.equal(url.pathname, '/api/zim/article');
  assert.equal(url.searchParams.get('pack'), 'benign');
  assert.equal(url.searchParams.get('url'), metadata.url);
  assert.equal(request.options.credentials, 'same-origin');
  assert.equal(request.options.cache, 'no-store');
  assert.equal(request.options.headers['X-Tonalli-Csrf'], 'session-csrf');
});

test('omitted pack stays omitted and malformed or revoked article responses fail', async () => {
  const { loadArticleMetadata } = await import('../src/features/article/client.js');
  await loadArticleMetadata({ url: metadata.url }, 'csrf', async (url) => {
    assert.equal(new URL(url, 'http://127.0.0.1').searchParams.has('pack'), false);
    return Response.json(metadata);
  });
  await assert.rejects(loadArticleMetadata({ url: 'C/en/index.html' }, 'csrf', async () => new Response('', { status: 401 })));
  for (const payload of [{}, { ...metadata, iframe_url: null }, { ...metadata, pack: '../other' }]) {
    await assert.rejects(loadArticleMetadata({ url: 'C/en/index.html' }, 'csrf', async () => Response.json(payload)));
  }
});

test('trusted frame URLs are confined to the returned installed ZIM and local origin', async () => {
  const { articleFrameUrl } = await import('../src/features/article/client.js');
  assert.equal(articleFrameUrl(metadata, 'http://127.0.0.1:1234'), `http://127.0.0.1:1234${metadata.iframe_url}`);
  for (const iframe_url of ['https://invalid.example/steal', '//invalid.example/steal', '/api/ping', '/zim/other/C/en/index.html', 'javascript:alert(1)', '/zim/benign/../../api/ping']) {
    assert.throws(() => articleFrameUrl({ ...metadata, iframe_url }, 'http://127.0.0.1:1234'));
  }
});

test('article links stay inside the same ZIM and preserve fragments and accented paths', async () => {
  const { resolveArticleLink } = await import('../src/features/article/client.js');
  const current = 'http://127.0.0.1:1234/zim/benign/C/en/index.html';
  assert.deepEqual(resolveArticleLink('../es/Ni%C3%B1ez_y_educaci%C3%B3n.html#section', current, 'benign'), {
    kind: 'internal', url: metadata.url, hash: '#section',
  });
  assert.deepEqual(resolveArticleLink('#section', current, 'benign'), { kind: 'fragment', hash: '#section' });
  for (const link of ['https://invalid.example', '//invalid.example', 'javascript:alert(1)', 'data:text/html,attack', '/api/ping', '/zim/other/C/index.html', '/zim/benign/%']) {
    assert.deepEqual(resolveArticleLink(link, current, 'benign'), { kind: 'offline' });
  }
});

test('article harness embeds exactly the approved sandbox with a trusted parent sentinel', async () => {
  const html = await readFile(new URL('../article.html', import.meta.url), 'utf8');
  assert.match(html, /<iframe[^>]+id="article-frame"[^>]+sandbox="allow-same-origin"/);
  assert.doesNotMatch(html, /allow-scripts|allow-popups|allow-top-navigation|srcdoc=/);
  assert.match(html, /id="trusted-sentinel"/);
  assert.match(html, /type="module"[^>]+src="\/article\.js"/);
});

test('embedded document detection uses MIME and never treats HTML as a PDF by its name',async()=>{
  const {articleDocumentKind}=await import('../src/features/article/client.js');
  assert.equal(articleDocumentKind({mime_type:'application/pdf; version=1.7',url:'C/download'}),'pdf');
  assert.equal(articleDocumentKind({mime_type:'application/epub+zip',url:'C/book.epub'}),'book');
  assert.equal(articleDocumentKind({mime_type:'application/octet-stream',url:'C/comic.CBZ'}),'book');
  assert.equal(articleDocumentKind({mime_type:'text/html',url:'C/spoof.pdf'}),null);
  assert.equal(articleDocumentKind({mime_type:'image/png',url:'C/page.png'}),null);
});
