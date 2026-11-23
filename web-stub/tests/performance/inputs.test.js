import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { access, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { promisify } from 'node:util';
import { test } from 'vitest';
import { preparePerfRun } from '../../scripts/perf-inputs.mjs';
import { root, treeHash } from '../e2e/article-host.mjs';

async function fixture(check) {
  const temporary = await mkdtemp(join(tmpdir(), 'tonalli-perf-input-test-'));
  try { await check(temporary); }
  finally {
    assert.equal(dirname(resolve(temporary)), resolve(tmpdir()));
    assert.ok(temporary.includes('tonalli-perf-input-test-'));
    await rm(temporary, { recursive: true, force: true, maxRetries: 3 });
  }
}

test('existing output remains byte-identical and prevents input preparation or a run', async () => fixture(async (temporary) => {
  const output = join(temporary, 'existing.json');
  const retained = join(temporary, 'retained');
  const bytes = Buffer.from('existing measurement: do not overwrite\n');
  await writeFile(output, bytes);
  await assert.rejects(preparePerfRun({ output, webRoot: join(temporary, 'absent'), artifactRoot: retained }), { code: 'EEXIST' });
  assert.deepEqual(await readFile(output), bytes);
  await assert.rejects(access(retained), { code: 'ENOENT' });
  await assert.rejects(promisify(execFile)(process.execPath, [join(root, 'web-stub/scripts/measure-web-perf.mjs'), '--output', output],
    { cwd: root, env: { ...process.env, TONALLI_SPIKE_CLI: join(temporary, 'must-not-run.exe') } }), (error) => {
    assert.match(error.stderr, /EEXIST/);
    assert.doesNotMatch(error.stdout, /trial \d/);
    return true;
  });
  assert.deepEqual(await readFile(output), bytes);
}));

test('retained input has a verified manifest and survives later source changes', async () => fixture(async (temporary) => {
  const source = join(temporary, 'web');
  await mkdir(join(source, 'assets'), { recursive: true });
  await writeFile(join(source, 'index.html'), '<h1>Frozen input</h1>\n');
  await writeFile(join(source, 'assets', 'data.bin'), Buffer.from([0, 1, 2, 255]));
  const expectedHash = await treeHash(source);
  const frozen = await preparePerfRun({ output: join(temporary, 'new.json'), webRoot: source,
    artifactRoot: join(temporary, 'retained'), expectedHash });
  assert.equal(frozen.sha256, expectedHash);
  assert.ok(frozen.path.includes(expectedHash));
  assert.equal(await treeHash(frozen.path), expectedHash);
  const manifest = JSON.parse(await readFile(frozen.manifestPath, 'utf8'));
  assert.equal(manifest.sha256, expectedHash);
  assert.equal(manifest.files.length, 2);
  assert.equal((await stat(join(frozen.path, 'index.html'))).mode & 0o222, 0);
  await writeFile(join(source, 'index.html'), '<h1>Changed after freeze</h1>\n');
  assert.notEqual(await treeHash(source), expectedHash);
  assert.equal(await treeHash(frozen.path), expectedHash);
  assert.equal(await readFile(join(frozen.path, 'index.html'), 'utf8'), '<h1>Frozen input</h1>\n');
}));
