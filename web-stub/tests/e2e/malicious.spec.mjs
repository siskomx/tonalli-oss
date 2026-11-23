import assert from 'node:assert/strict';
import { open, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { redact, root, startArticleHost, trustedNotesRequest } from './article-host.mjs';

let host;
let current;
const evidence = [];
const owners = new WeakMap();
const withNotes = process.env.TONALLI_UNLOCKED_NOTES === '1';
const tracePath = withNotes ? process.env.TONALLI_NOTES_TRACE : join(root, 'docs/spike/article-browser-trace.json');
let traceFile;
let notesBaseline;
let unlockStatus;

async function trustedNotes(path = '/api/notes', body) {
  return trustedNotesRequest(host.page, path, body);
}

async function checkNotesUnchanged() {
  const response = await trustedNotes();
  assert.equal(response.status, 200, 'real vault must still be unlocked');
  assert.match(response.cache, /no-store/);
  assert.deepEqual(response.data, notesBaseline.contents, 'actual encrypted note contents must remain unchanged');
  const fingerprint = await host.notes.fingerprint();
  assert.deepEqual(fingerprint, notesBaseline.fingerprint, 'DB and key-envelope bytes must remain unchanged');
  current.notes = { unlocked: true, contentsUnchanged: true, fingerprint };
}

async function screenshot(name) {
  if (withNotes) {
    const png = await host.page.screenshot({ fullPage: true, type: 'png' });
    await writeFile(`${tracePath.replace(/\.json$/, '')}-${name}.png`, png, { flag: 'wx' });
  } else await host.page.screenshot({ path: join(root, `docs/spike/article-${name}.png`), fullPage: true, type: 'png' });
}

test.before(async () => {
  if (withNotes) {
    assert.ok(tracePath, 'TONALLI_NOTES_TRACE must name a new output file');
    traceFile = await open(tracePath, 'wx');
  }
  host = await startArticleHost({ notesFixture: withNotes });
  host.context.on('request', (request) => {
    let owner = 'unknown';
    try { owner = request.frame() === host.page.mainFrame() ? 'parent' : 'article'; } catch {}
    owners.set(request, owner);
    current?.requests.push({ url: redact(request.url()), type: request.resourceType(), owner });
  });
  host.context.on('response', (response) => current?.responses.push({ url: redact(response.url()), status: response.status(), owner: owners.get(response.request()) }));
  host.page.on('console', (message) => current?.console.push({ type: message.type(), text: redact(message.text()) }));
  host.page.on('pageerror', (error) => current?.errors.push(redact(error.message)));
  host.context.on('page', () => current?.popups.push('unexpected page'));
  if (withNotes) {
    const unlock = await trustedNotes('/api/notes/unlock', { passphrase: 'tonalli-browser-test-only' });
    unlockStatus = unlock.status;
    assert.equal(unlock.status, 200, 'the real preinitialized vault must unlock before any hostile case');
    assert.match(unlock.cache, /no-store/);
    const response = await trustedNotes();
    assert.equal(response.status, 200);
    assert.equal(response.data.notes.length, 1);
    assert.equal(response.data.notes[0].title, 'Tonalli M1 synthetic sentinel');
    assert.equal(response.data.notes[0].body, 'TONALLI_M1_NOTES_SENTINEL_20261111');
    notesBaseline = { contents: response.data, fingerprint: await host.notes.fingerprint() };
  }
});

test.beforeEach((context) => {
  current = { name: context.name, requests: [], responses: [], console: [], errors: [], popups: [] };
  evidence.push(current);
});

test.afterEach(async () => {
  if (!host) return;
  await host.page.waitForLoadState('networkidle', { timeout: 3000 }).catch(() => {});
  assert.equal(host.blocked.length, 0, 'no article resource or clicked link may attempt external egress');
  assert.equal(current.popups.length, 0);
  assert.equal(current.errors.length, 0);
  const apiSuccesses = current.responses.filter((response) => response.owner !== 'parent'
    && new URL(response.url).pathname.startsWith('/api/') && response.status >= 200 && response.status < 300);
  assert.deepEqual(apiSuccesses, [], 'untrusted article frames cannot make successful API requests');
  assert.equal(await host.page.locator('#trusted-sentinel').textContent(), 'Trusted parent');
  assert.equal(await host.page.locator('body').getAttribute('data-compromised'), null);
  assert.equal(await host.page.evaluate(() => Boolean(window.articleScriptRan)), false);
  if (notesBaseline) await checkNotesUnchanged();
});

test.after(async () => {
  if (!host && !traceFile) return;
  try {
    const report = `${JSON.stringify({
      browserPath: host?.browserPath, browserVersion: host?.context.browser().version(), cliSha256: host?.cliSha256,
      baseline: host?.baseline, blocked: host?.blocked,
      fixtures: host?.manifest.provenance,
      ...(withNotes ? { notes: { unlockStatus, fixtureExecutableSha256: host?.notes?.fixtureExecutableSha256,
        baseline: notesBaseline ? { fingerprint: notesBaseline.fingerprint, sentinelVerified: true } : null } } : {}),
      isolation: 'One isolated temporary Edge profile and owned two-ZIM CLI; exact-origin HTTP allowance, blocked WebSockets and service workers; content sandbox and response CSP unchanged.',
      cases: evidence,
    }, null, 2)}\n`;
    if (traceFile) await traceFile.writeFile(report, 'utf8');
    else await writeFile(tracePath, report, 'utf8');
  } finally { await traceFile?.close(); await host?.close(); }
});

async function article(pack, entry) {
  const query = new URLSearchParams({ pack, url: entry });
  const [metadata, content, parent] = await Promise.all([
    host.page.waitForResponse((response) => new URL(response.url()).pathname === '/api/zim/article'),
    host.page.waitForResponse((response) => new URL(response.url()).pathname.startsWith(`/zim/${pack}/`)
      && response.request().resourceType() === 'document'),
    host.page.goto(`${host.origin}/article.html?${query}`, { waitUntil: 'load' }),
  ]);
  assert.equal(metadata.status(), 200, 'the real article metadata route must succeed');
  assert.equal(content.status(), 200, 'the real ZIM document must load before security assertions');
  assert.match(parent.headers()['content-security-policy'], /frame-src 'self'/);
  assert.match(content.headers()['content-security-policy'], /script-src 'none'/);
  assert.equal(content.headers()['x-content-type-options'], 'nosniff');
  const canonical = await metadata.json();
  assert.equal(content.url(), new URL(canonical.iframe_url, host.origin).href);
  current.document = { url: content.url(), status: content.status(), csp: content.headers()['content-security-policy'] };
  await host.page.locator('#article-frame').waitFor();
  assert.equal(await host.page.locator('#article-frame').getAttribute('sandbox'), 'allow-same-origin');
  return host.page.frameLocator('#article-frame');
}

test('benign article applies CSS, inline styles, style attributes, images and data images', async () => {
  const frame = await article('benign', host.manifest.benign.entries.index);
  await host.page.getByText('Article ready', { exact: true }).waitFor();
  await frame.locator('#external-css').waitFor();
  const styles = await frame.locator('body').evaluate((body) => {
    const document = body.ownerDocument;
    return {
      external: getComputedStyle(document.querySelector('#external-css')).color,
      inline: getComputedStyle(document.querySelector('#inline-style')).color,
      attribute: getComputedStyle(document.querySelector('#style-attribute')).backgroundColor,
      image: document.querySelector('#image').naturalWidth,
      dataImage: document.querySelector('#data-image').naturalWidth,
    };
  });
  assert.equal(styles.external, 'rgb(17, 34, 51)');
  assert.equal(styles.inline, 'rgb(11, 22, 33)');
  assert.equal(styles.attribute, 'rgb(44, 55, 66)');
  assert.ok(styles.image > 0 && styles.dataImage > 0);
  current.styles = styles;
  assert.ok(current.responses.some((response) => response.owner === 'article' && response.url.endsWith('/assets/style.css') && response.status === 200));
  assert.ok(current.responses.some((response) => response.owner === 'article' && response.url.endsWith('/assets/pixel.png') && response.status === 200));
  await screenshot('benign');
});

test('internal links preserve the ZIM and accented paths while external links become unavailable', async () => {
  const frame = await article('benign', host.manifest.benign.entries.index);
  await host.page.getByText('Article ready', { exact: true }).waitFor();
  for (const id of ['external-link', 'protocol-relative-link']) {
    assert.equal(await frame.locator(`#${id}`).getAttribute('href'), null);
    await frame.locator(`#${id}`).click({ force: true });
  }
  assert.match(await frame.locator('body').innerText(), /unavailable offline/);
  await frame.locator('#internal-link').click();
  await host.page.getByText('Article ready', { exact: true }).waitFor();
  const frameUrl = await host.page.locator('#article-frame').getAttribute('src');
  assert.ok(decodeURI(frameUrl).includes(host.manifest.benign.entries.spanish));
  assert.ok(frameUrl.includes('/zim/benign/'));
  assert.match(await host.page.locator('#article-title').innerText(), /Niñez|educación/i);
  assert.match(await frame.locator('#article-heading').innerText(), /Niñez y educación/);
  assert.equal(await frame.locator('#image').evaluate((image) => image.naturalWidth), 2);
});

test('redirect metadata reaches the final article with working relative resources', async () => {
  const frame = await article('benign', host.manifest.benign.entries.redirect);
  await host.page.getByText('Article ready', { exact: true }).waitFor();
  await frame.locator('#image').waitFor();
  assert.ok(await frame.locator('#image').evaluate((image) => image.naturalWidth > 0));
  const src = await host.page.locator('#article-frame').getAttribute('src');
  assert.ok(decodeURI(src).endsWith(host.manifest.benign.entries.index));
});

for (const entry of ['passive', 'all', 'meta', 'top', 'popup', 'form', 'ping', ...(withNotes ? ['neutral'] : [])]) {
  test(`hostile ${entry} cannot execute scripts, escape its frame or access API`, async () => {
    const parentUrl = `${host.origin}/article.html?${new URLSearchParams({ pack: 'hostile', url: host.manifest.hostile.entries[entry] })}`;
    const frame = await article('hostile', host.manifest.hostile.entries[entry]);
    if (['top', 'popup'].includes(entry)) {
      await host.page.getByText('Article ready', { exact: true }).waitFor();
      await frame.locator(entry === 'top' ? '#top-link' : '#popup-link').click({ force: true });
    } else if (entry === 'form') {
      await frame.locator('#attack-form button, #attack-form input[type=submit]').first().click();
      await frame.locator('#ping-form button, #ping-form input[type=submit]').first().click();
    } else if (entry === 'ping') {
      await host.page.getByText('Article ready', { exact: true }).waitFor();
      await frame.locator('#ping-link').click();
    }
    await host.page.waitForLoadState('networkidle', { timeout: 3000 });
    assert.equal(host.page.url(), parentUrl);
    const actualFrame = host.page.frames().find((candidate) => candidate !== host.page.mainFrame() && candidate.parentFrame() === host.page.mainFrame());
    assert.equal(await actualFrame.evaluate(() => Boolean(window.articleScriptRan)), false);
    if (entry === 'passive') await screenshot('hostile');
  });
}
