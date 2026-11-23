import assert from 'node:assert/strict';
import { chmod, cp, mkdir, mkdtemp, open, readFile, rename, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { sha256, treeHash } from '../tests/e2e/article-host.mjs';

export async function preparePerfRun({ output, webRoot, artifactRoot, expectedHash }) {
  output = resolve(output);
  await mkdir(dirname(output), { recursive: true });
  // Exclusive creation precedes any input work or browser/server startup.
  const reserved = await open(output, 'wx');
  await reserved.writeFile(`${JSON.stringify({ status: 'reserved', startedAt: new Date().toISOString() })}\n`, 'utf8');
  await reserved.close();
  try {
    const started = performance.now();
    const sourceHash = await treeHash(resolve(webRoot));
    if (expectedHash) assert.equal(sourceHash, expectedHash, 'UI source differs from explicitly expected artifact');
    const root = resolve(artifactRoot);
    const bundle = join(root, sourceHash);
    const path = join(bundle, 'dist');
    const manifestPath = join(bundle, 'manifest.json');
    await mkdir(root, { recursive: true });
    let exists = false;
    try { exists = (await stat(bundle)).isDirectory(); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (!exists) {
      const staged = await mkdtemp(join(root, '.stage-'));
      try {
        const stagedDist = join(staged, 'dist');
        await cp(resolve(webRoot), stagedDist, { recursive: true, errorOnExist: true, force: false });
        const files = [];
        assert.equal(await treeHash(stagedDist, files), sourceHash, 'UI changed while snapshotting');
        assert.equal(await treeHash(resolve(webRoot)), sourceHash, 'UI source changed while snapshotting');
        await writeFile(join(staged, 'manifest.json'), `${JSON.stringify({ sha256: sourceHash, files }, null, 2)}\n`, 'utf8');
        await rename(staged, bundle);
      } finally {
        assert.equal(dirname(resolve(staged)), root, 'cleanup stays inside owned artifact directory');
        await rm(staged, { recursive: true, force: true, maxRetries: 3 });
      }
    }
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    const verifiedFiles = [];
    assert.equal(await treeHash(path, verifiedFiles), sourceHash, 'retained UI bundle was modified');
    assert.equal(manifest.sha256, sourceHash);
    assert.deepEqual(verifiedFiles, manifest.files, 'retained UI file manifest differs');
    // Windows can deny renaming a freshly scanned/read-only tree; seal after activation.
    for (const file of verifiedFiles) await chmod(join(path, file.file), 0o444);
    await chmod(manifestPath, 0o444);
    return { path, sha256: sourceHash, manifestPath, manifestSha256: await sha256(manifestPath),
      fileCount: verifiedFiles.length, preparationMs: performance.now() - started,
      retention: 'hash-keyed read-only files; reused only after verification; never removed by context cleanup' };
  } catch (error) {
    await writeFile(output, `${JSON.stringify({ status: 'input_preparation_failed', error: error.message })}\n`, 'utf8');
    throw error;
  }
}
