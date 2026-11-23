import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const root = fileURLToPath(new URL('../../', import.meta.url));
const browserPath = process.env.TONALLI_MAP_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const cliPath = process.env.TONALLI_SPIKE_CLI || join(root, 'target/debug/spike-cli.exe');
const files = { mx: 'mx-mexico-city-z8.pmtiles', us: 'us-san-diego-z8.pmtiles', ca: 'ca-montreal-z8.pmtiles' };
const cases = [{ id: 'mx', packs: ['mx'] }, { id: 'us', packs: ['us'] }, { id: 'ca', packs: ['ca'] }, { id: 'mx-ca', packs: ['mx', 'ca'] }, { id: 'mx-malicious-credit', packs: ['mx'], maliciousCredit: true }];
const results = [];
const scrub = (url) => url.replace(/token=[0-9a-f]+/gi, 'token=[redacted]');
const temporary = await mkdtemp(join(tmpdir(), 'tonalli-map-browser-'));
const frozenCli = join(temporary, 'spike-cli.exe');
await copyFile(cliPath, frozenCli);
const binaryHash = createHash('sha256').update(await readFile(frozenCli)).digest('hex');

async function start(caseInfo, directory) {
  const args = ['serve', '--web', join(root, 'web-stub/dist'), '--data-dir', join(directory, 'data')];
  for (const id of caseInfo.packs) args.push('--pack', `${id}=${join(root, 'docs/spike/packs', files[id])}`);
  const child = spawn(frozenCli, args, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let stderr = '';
  child.stderr.on('data', (data) => { stderr += data; });
  const launch = await new Promise((accept, reject) => {
    const timeout = setTimeout(() => reject(new Error(`CLI startup timed out: ${stderr}`)), 20000);
    const reader = createInterface({ input: child.stdout });
    reader.once('line', (line) => { clearTimeout(timeout); reader.close(); accept(line.trim()); });
    child.once('error', (error) => { clearTimeout(timeout); reject(error); });
    child.once('exit', (code) => { clearTimeout(timeout); reject(new Error(`CLI exited ${code}: ${stderr}`)); });
  }).catch((error) => { child.kill(); throw error; });
  assert.ok(launch.startsWith('http://127.0.0.1:'));
  return { child, launch, origin: new URL(launch).origin };
}

async function capture(page, context, caseInfo, selected, log) {
  const status = `Map ready — ${caseInfo.packs.length} installed pack${caseInfo.packs.length === 1 ? '' : 's'} — ${selected}`;
  await page.getByText(status, { exact: true }).waitFor({ state: 'visible', timeout: 30000 });
  const render = await page.locator('#map').evaluate((element) => JSON.parse(element.dataset.renderEvidence));
  assert.deepEqual(render.sources.sort(), caseInfo.packs.map((id) => `pack:${id}`).sort());
  assert.ok(render.labels.length > 0, 'actual rendered symbol features must include labels');
  assert.ok(render.iconLayers.length > 0, 'actual rendered symbol features must include icon layers');
  assert.match(await page.locator('.maplibregl-ctrl-attrib').innerText(), /OpenStreetMap/);
  assert.equal(await page.locator('.maplibregl-ctrl-attrib img, .maplibregl-ctrl-attrib script').count(), 0);
  const png = join(root, `docs/spike/map-${caseInfo.id}-${selected}-offline.png`);
  await page.screenshot({ path: png, type: 'png', fullPage: true, timeout: 20000 });
  const renderer = await page.locator('canvas').evaluate((canvas) => {
    const gl = canvas.getContext('webgl2') || canvas.getContext('webgl');
    const extension = gl?.getExtension('WEBGL_debug_renderer_info');
    return { version: gl?.getParameter(gl.VERSION), renderer: extension ? gl.getParameter(extension.UNMASKED_RENDERER_WEBGL) : gl?.getParameter(gl.RENDERER) };
  });
  assert.ok(renderer.version, 'WebGL context must be real and initialized');
  log.views.push({ selected, status, render, renderer, screenshot: png.slice(root.length).replaceAll('\\', '/') });
}

try {
  for (const caseInfo of cases) {
    const directory = join(temporary, caseInfo.id);
    await mkdir(directory);
    const server = await start(caseInfo, directory);
    let context;
    const log = { case: caseInfo.id, installed: caseInfo.packs, requests: [], responses: [], blocked: [], console: [], errors: [], views: [] };
    results.push(log);
    try {
      context = await chromium.launchPersistentContext(join(directory, 'profile'), {
        executablePath: browserPath, headless: true, serviceWorkers: 'block',
        viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1,
      });
      log.browserVersion = context.browser().version();
      await context.route('**/*', async (route) => {
        try {
          const url = route.request().url();
          if (new URL(url).origin === server.origin) {
            if (caseInfo.maliciousCredit && new URL(url).pathname === '/api/maps') {
              // Only this security case supplies hostile metadata; tiles still use the real pack.
              await route.fulfill({ json: { packs: [{ id: 'mx', bounds: [-99.2, 19.38, -99.1, 19.48],
                min_zoom: 0, max_zoom: 8, center: [-99.15, 19.43],
                attribution: '<img src="https://invalid.example/credit" onerror="window.compromised=true"><script>window.compromised=true</script>Publisher credit' }] } });
            } else await route.continue();
          }
          else { log.blocked.push(scrub(url)); await route.abort('blockedbyclient'); }
        } catch (error) {
          log.errors.push(scrub(String(error)));
          await route.abort();
        }
      });
      await context.routeWebSocket('**/*', (socket) => { log.blocked.push(scrub(socket.url())); socket.close(); });
      context.on('request', (request) => log.requests.push({ url: scrub(request.url()), type: request.resourceType(),
        site: request.headers()['sec-fetch-site'] ?? null, destination: request.headers()['sec-fetch-dest'] ?? null }));
      context.on('response', (response) => log.responses.push({ url: scrub(response.url()), status: response.status() }));
      const page = context.pages()[0] || await context.newPage();
      page.on('console', (message) => log.console.push({ type: message.type(), text: scrub(message.text()) }));
      page.on('pageerror', (error) => log.errors.push(scrub(error.message)));
      await page.goto(server.launch, { waitUntil: 'load', timeout: 20000 });
      assert.equal(page.url(), `${server.origin}/`, 'native launch must exchange credentials without a bypass');
      await page.goto(`${server.origin}/map.html`, { waitUntil: 'load', timeout: 20000 });
      // Inventory ordering is independent of command-line registration ordering.
      await page.locator('#pack-select').selectOption(caseInfo.packs[0]);
      await capture(page, context, caseInfo, caseInfo.packs[0], log);
      if (caseInfo.packs.length > 1) {
        await page.locator('#pack-select').selectOption('ca');
        await capture(page, context, caseInfo, 'ca', log);
      }
      if (caseInfo.maliciousCredit) {
        assert.equal(await page.evaluate(() => Boolean(window.compromised)), false);
        assert.match(await page.locator('.maplibregl-ctrl-attrib').innerText(), /Publisher credit/);
      }
      assert.equal(log.blocked.length, 0, 'the map must attempt no external requests');
      assert.equal(log.errors.length, 0);
      assert.equal(log.console.filter((entry) => entry.type === 'error').length, 0);
      assert.ok(log.responses.some((entry) => entry.status === 206 && entry.url.includes('/packs/')));
      assert.ok(log.responses.some((entry) => entry.status === 200 && entry.url.includes('/fonts/')));
      assert.ok(log.responses.some((entry) => entry.status === 200 && /sprites\/sprite(?:@2x)?\.png$/.test(entry.url)));
      console.log(`${caseInfo.id}: ${log.views.length} view(s), ${log.requests.length} requests, zero external attempts`);
    } finally {
      if (context) await context.close();
      if (server.child.exitCode === null) {
        const stopped = new Promise((accept) => server.child.once('exit', accept));
        server.child.kill();
        await stopped;
      }
      await writeFile(join(root, 'docs/spike/map-browser-trace.json'), `${JSON.stringify({ browserPath, cliSha256: binaryHash,
        isolation: 'One isolated temporary Edge profile and owned CLI process at a time; context HTTP interception allows exact loopback origin only; WebSockets blocked; service workers blocked; page CSP also denies external resources.',
        cases: results }, null, 2)}\n`, 'utf8');
    }
  }
} catch (error) {
  console.error(scrub(error.stack || String(error)));
  process.exitCode = 1;
} finally {
  const resolved = resolve(temporary);
  assert.ok(resolved.startsWith(resolve(tmpdir()) + sep + 'tonalli-map-browser-'), 'cleanup must stay inside the owned temp directory');
  await rm(resolved, { recursive: true, force: true, maxRetries: 3 });
}
