import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { copyFile, cp, lstat, mkdir, mkdtemp, readFile, readdir, realpath, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, sep } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

export const root = fileURLToPath(new URL('../../../', import.meta.url));
export const redact = (value) => String(value).replace(/token=[0-9a-f]+/gi, 'token=[redacted]');

export async function trustedNotesRequest(page, path = '/api/notes', body) {
  return page.evaluate(async ({ path, body }) => {
    const session = await fetch('/api/session', { credentials: 'same-origin', cache: 'no-store' });
    const { csrf } = await session.json();
    const response = await fetch(path, { method: body ? 'POST' : 'GET', credentials: 'same-origin', cache: 'no-store',
      headers: { 'X-Tonalli-Csrf': csrf, ...(body ? { 'Content-Type': 'application/json' } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}) });
    return { status: response.status, cache: response.headers.get('cache-control'),
      data: response.headers.get('content-type')?.includes('application/json') ? await response.json() : null };
  }, { path, body });
}

export async function sha256(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

export async function packIdentity(path) {
  const value = await stat(path, { bigint: true });
  return { path: await realpath(path), bytes: String(value.size), mtimeNs: String(value.mtimeNs),
    birthtimeNs: String(value.birthtimeNs), dev: String(value.dev), ino: String(value.ino) };
}

export async function treeHash(directory, files = null) {
  const rows = [];
  async function visit(path, relative = '') {
    for (const entry of (await readdir(path, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const name = relative ? `${relative}/${entry.name}` : entry.name;
      if (entry.isDirectory()) await visit(join(path, entry.name), name);
      else if (entry.isFile()) {
        const file = join(path, entry.name);
        const hash = await sha256(file);
        rows.push(`${name}\0${hash}\n`);
        if (files) files.push({ file: name, bytes: (await stat(file)).size, sha256: hash });
      }
    }
  }
  await visit(directory);
  return createHash('sha256').update(rows.join('')).digest('hex');
}

export async function startArticleHost(options = {}) {
  const setupStarted = performance.now();
  const setup = {};
  const fixtures = join(root, 'crates/core/tests/fixtures/articles');
  const published = JSON.parse(await readFile(join(fixtures, 'manifest.json'), 'utf8'));
  const benign = published.packs.find((pack) => pack.kind === 'benign');
  const hostile = published.packs.find((pack) => pack.kind === 'hostile');
  for (const pack of [benign, hostile]) {
    assert.equal(createHash('sha256').update(await readFile(join(root, pack.path))).digest('hex'), pack.sha256);
  }
  const manifest = {
    benign: { file: benign.path, entries: { index: benign.urls['en/index.html'], spanish: benign.urls['es/Niñez_y_educación.html'], redirect: benign.urls['redirect/chain.html'] } },
    hostile: { file: hostile.path, entries: Object.fromEntries(['passive', 'all', 'meta', 'top', 'popup', 'form', 'ping', 'neutral']
      .map((name) => [name, hostile.urls[`hostile/${name}.html`]])) },
    provenance: published,
  };
  const extraPacks = options.extraPacks || [];
  for (const pack of extraPacks) {
    const path = join(root, pack.file);
    assert.ok(!path.endsWith('.part'), 'only activated packs are test inputs');
    assert.equal((await stat(path)).size, pack.bytes, `pack size: ${pack.id}`);
    if (pack.verificationReceipt) {
      // The full 52 GB corpus was already streamed/verified at activation. Do not
      // rehash it for each UI context; retain the receipt and expected fingerprint.
      assert.ok(pack.sha256 && (await stat(join(root, pack.verificationReceipt))).isFile());
      assert.deepEqual(await packIdentity(path), pack.identity, `verified pack identity: ${pack.id}`);
    } else assert.equal(await sha256(path), pack.sha256, `pack hash: ${pack.id}`);
  }
  const temporary = await mkdtemp(join(tmpdir(), 'tonalli-article-browser-'));
  let child;
  let fixtureChild;
  let context;
  let notes;
  async function close() {
    try { if (context) await context.close(); }
    finally {
      if (fixtureChild?.pid && fixtureChild.exitCode === null && fixtureChild.signalCode === null) {
        const stopped = new Promise((accept) => fixtureChild.once('exit', accept));
        fixtureChild.kill();
        await stopped;
      }
      if (child && child.exitCode === null) {
        const stopped = new Promise((accept) => child.once('exit', accept));
        child.kill();
        await stopped;
      }
      for (const pack of extraPacks.filter((pack) => pack.identity)) {
        assert.deepEqual(await packIdentity(join(root, pack.file)), pack.identity, `pack remained immutable: ${pack.id}`);
      }
      assert.ok(resolve(temporary).startsWith(resolve(tmpdir()) + sep + 'tonalli-article-browser-'));
      await rm(temporary, { recursive: true, force: true, maxRetries: 3 });
    }
  }
  try {
    const executable = join(temporary, 'spike-cli.exe');
    const sourceExecutable = process.env.TONALLI_SPIKE_CLI || join(root, 'target/debug/spike-cli.exe');
    const sourceHash = await sha256(sourceExecutable);
    await copyFile(sourceExecutable, executable);
    const cliSha256 = await sha256(executable);
    assert.equal(cliSha256, sourceHash, 'CLI copy changed during snapshot');
    assert.equal(await sha256(sourceExecutable), sourceHash, 'CLI source changed during snapshot');
    const web = join(temporary, 'web');
    await cp(options.webRoot || join(root, 'web-stub/dist'), web, { recursive: true });
    const distSha256 = await treeHash(web);
    setup.snapshotMs = performance.now() - setupStarted;
    const dataDirectory = join(temporary, 'data');
    if (options.notesFixture) {
      const fixtureStarted = performance.now();
      await mkdir(dataDirectory);
      const fixtureExecutable = join(temporary, 'notes-browser-fixture.exe');
      const sourceFixture = process.env.TONALLI_NOTES_FIXTURE || join(root, 'target/debug/examples/notes-browser-fixture.exe');
      const fixtureExecutableSha256 = await sha256(sourceFixture);
      await copyFile(sourceFixture, fixtureExecutable);
      assert.equal(await sha256(fixtureExecutable), fixtureExecutableSha256, 'fixture executable copy changed');
      assert.equal(await sha256(sourceFixture), fixtureExecutableSha256, 'fixture executable source changed');
      fixtureChild = spawn(fixtureExecutable, [dataDirectory], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
      await new Promise((accept, reject) => {
        let outputBytes = 0;
        const timeout = setTimeout(() => reject(new Error('Encrypted notes fixture setup timed out')), 20000);
        fixtureChild.stdout.on('data', (chunk) => { outputBytes += chunk.length; });
        fixtureChild.stderr.on('data', (chunk) => { outputBytes += chunk.length; });
        fixtureChild.once('error', () => { clearTimeout(timeout); reject(new Error('Encrypted notes fixture could not start')); });
        fixtureChild.once('close', (code) => {
          clearTimeout(timeout);
          if (code === 0 && outputBytes === 0) accept();
          else reject(new Error(`Encrypted notes fixture failed (exit ${code}; output bytes ${outputBytes})`));
        });
      });
      notes = { fixtureExecutableSha256, async fingerprint() {
        return Object.fromEntries(await Promise.all(['notes.db', 'notes.keys'].map(async (name) => {
          const file = join(dataDirectory, name);
          const info = await lstat(file);
          assert.ok(info.isFile() && !info.isSymbolicLink(), 'fixture must contain ordinary encrypted vault files');
          return [name, { bytes: info.size, sha256: await sha256(file) }];
        })));
      } };
      await notes.fingerprint();
      setup.notesFixtureMs = performance.now() - fixtureStarted;
    }
    const packs = [
      { id: 'benign', file: manifest.benign.file }, { id: 'hostile', file: manifest.hostile.file }, ...extraPacks,
    ];
    const titleIndexes = options.titleIndexes || [];
    const serverStarted = performance.now();
    child = spawn(executable, ['serve', '--web', web, '--data-dir', dataDirectory,
      ...packs.flatMap((pack) => ['--pack', `${pack.id}=${join(root, pack.file)}`]),
      ...titleIndexes.flatMap((index) => ['--title-index', `${index.id}=${resolve(root, index.file)}`])], {
      windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stderr = '';
    child.stderr.on('data', (data) => { stderr += data; });
    const launch = await new Promise((accept, reject) => {
      const timeout = setTimeout(() => reject(new Error(`CLI startup timed out: ${redact(stderr)}`)), options.startupTimeoutMs || 20000);
      const reader = createInterface({ input: child.stdout });
      reader.once('line', (line) => { clearTimeout(timeout); reader.close(); accept(line.trim()); });
      child.once('error', (error) => { clearTimeout(timeout); reject(error); });
      child.once('exit', (code) => { clearTimeout(timeout); reject(new Error(`CLI exited ${code}: ${redact(stderr)}`)); });
    });
    const origin = new URL(launch).origin;
    setup.serverStartupMs = performance.now() - serverStarted;
    assert.equal(new URL(launch).hostname, '127.0.0.1');
    const browserPath = process.env.TONALLI_ARTICLE_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
    const browserStarted = performance.now();
    context = await chromium.launchPersistentContext(join(temporary, 'profile'), { executablePath: browserPath,
      headless: options.headless ?? true, locale: options.locale || 'en-US',
      serviceWorkers: 'block', viewport: options.viewport || { width: 1280, height: 900 }, deviceScaleFactor: 1,
      args: options.browserArgs || [] });
    setup.browserLaunchMs = performance.now() - browserStarted;
    const blocked = [];
    await context.route('**/*', async (route) => {
      if (new URL(route.request().url()).origin === origin) await route.continue();
      else { blocked.push(redact(route.request().url())); await route.abort('blockedbyclient'); }
    });
    await context.routeWebSocket('**/*', (socket) => { blocked.push(redact(socket.url())); socket.close(); });
    const page = context.pages()[0] || await context.newPage();
    page.setDefaultTimeout(5000);
    page.setDefaultNavigationTimeout(15000);
    const authStarted = performance.now();
    let baseline;
    if (options.bootstrap === 'request') {
      const exchanged = await context.request.get(launch, { maxRedirects: 0 });
      assert.equal(exchanged.status(), 302, 'launch exchange must not follow into app HTML');
      assert.equal(new URL(exchanged.headers().location, origin).origin, origin);
      const session = await context.request.get(`${origin}/api/session`, { headers: { 'Sec-Fetch-Dest': 'empty', Origin: origin } });
      assert.equal(session.status(), 200);
      baseline = { launch: exchanged.status(), session: session.status(), bootstrap: 'request-only; no browser app navigation' };
      assert.equal(page.url(), 'about:blank');
    } else {
      await page.goto(launch, { waitUntil: 'load' });
      baseline = await page.evaluate(async () => {
      const session = await fetch('/api/session', { credentials: 'same-origin', cache: 'no-store' });
      const { csrf } = await session.json();
      const ping = await fetch('/api/ping', { credentials: 'same-origin', cache: 'no-store', headers: { 'X-Tonalli-Csrf': csrf } });
      return { session: session.status, ping: ping.status, body: await ping.text() };
    });
    assert.equal(baseline.session, 200);
    assert.equal(baseline.ping, 200, 'browser gate requires a CLI built with tonalli-core/test-routes');
      assert.equal(baseline.body, 'ok');
    }
    setup.authMs = performance.now() - authStarted;
    setup.totalMs = performance.now() - setupStarted;
    return { page, context, manifest, origin, blocked, baseline, browserPath, cliSha256, distSha256, notes,
      extraPacks, locale: options.locale || 'en-US', setup, close };
  } catch (error) {
    await close();
    throw new Error(redact(error.message));
  }
}
