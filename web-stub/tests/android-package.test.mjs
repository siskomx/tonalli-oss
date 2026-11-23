import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { packageAndroidUi } from '../scripts/package-android-ui.mjs';

test('packages checked UI bytes and retains previous output on invalid input', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'tonalli-ui-package-test-'));
  try {
    const input = path.join(root, 'dist');
    const output = path.join(root, 'assets');
    await mkdir(path.join(input, 'assets'), { recursive: true });
    await writeFile(path.join(input, 'index.html'), '<main>Tonalli</main>');
    await writeFile(path.join(input, 'assets', 'app.js'), 'console.log("offline");');
    const first = await packageAndroidUi(input, output);
    assert.equal(first.files.length, 2);
    assert.deepEqual(await readFile(path.join(output, 'ui/index.html')), await readFile(path.join(input, 'index.html')));
    assert.deepEqual(JSON.parse(await readFile(path.join(output, 'ui-manifest.json'))), first);
    assert.deepEqual(await packageAndroidUi(input, output), first);
    await mkdir(path.join(root, 'outside'));
    await symlink(path.join(root, 'outside'), path.join(input, 'linked'), 'junction');
    await assert.rejects(packageAndroidUi(input, output), /link/);
    assert.deepEqual(JSON.parse(await readFile(path.join(output, 'ui-manifest.json'))), first);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('requires index.html and enforces byte limits before publishing', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'tonalli-ui-package-test-'));
  try {
    const input = path.join(root, 'dist');
    const output = path.join(root, 'assets');
    await mkdir(input);
    await assert.rejects(packageAndroidUi(input, output), /index.html/);
    await writeFile(path.join(input, 'index.html'), '12345');
    await assert.rejects(packageAndroidUi(input, output, { maxBytes: 4 }), /limit/);
    await assert.rejects(packageAndroidUi(input, output, { maxManifestBytes: 4 }), /manifest limit/);
    await assert.rejects(readFile(path.join(output, 'ui-manifest.json')), /ENOENT/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
