import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { chromium } from 'playwright-core';
import { baseline, scroll } from '../../scripts/web-perf-steps.mjs';

test('desktop simulation only: CDP disconnect keeps the owned browser alive; shared scroll protocol accepts registry identity', async () => {
  const temporary = await mkdtemp(path.join(tmpdir(), 'tonalli-cdp-simulation-'));
  let context, attached;
  try {
    context = await chromium.launchPersistentContext(temporary, {
      executablePath: process.env.TONALLI_ARTICLE_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
      headless: true, args: ['--remote-debugging-port=0', '--remote-debugging-address=127.0.0.1'],
    });
    const port = Number((await readFile(path.join(temporary, 'DevToolsActivePort'), 'utf8')).split('\n')[0]);
    assert.ok(Number.isInteger(port) && port > 0);
    const original = context.pages()[0];
    attached = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { noDefaults: true });
    const page = attached.contexts()[0].pages()[0];
    await page.setContent('<div data-testid="library-list" style="height:400px;overflow:auto"><div style="height:4000000px"></div></div>');
    const idle = await baseline(page);
    assert.equal(idle.warmupFrames.length, 30);
    assert.equal(idle.frames.length, 90);
    // Synthetic committed rows are fixture data, never phone or Wikipedia evidence.
    await page.evaluate(() => {
      window.__perf = {};
      window.__libraryCommit = { pack: 'probe', visibleLimit: 100000, rowHeight: 40,
        rows: Array.from({ length: 200 }, (_, i) => ({ index: 50140 + i, loaded: 50140 + i !== 50200 })) };
    });
    const sample = await scroll({ page }, 400, 'probe');
    assert.ok(sample.samples.length > 2);
    assert.ok(sample.samples.every((frame) => frame.snapshotMatches));
    assert.ok(sample.visibleGapMs > 0 && sample.maxMissingRows > 0);
    await attached.close();
    attached = null;
    assert.equal(await original.evaluate(() => 42), 42, 'remote CDP close disconnected without closing the existing page/process');
  } finally {
    await attached?.close();
    await context?.close();
    assert.equal(path.dirname(path.resolve(temporary)), path.resolve(tmpdir()));
    assert.ok(path.basename(temporary).startsWith('tonalli-cdp-simulation-'));
    await rm(temporary, { recursive: true, force: true });
  }
});
