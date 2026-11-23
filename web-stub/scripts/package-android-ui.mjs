import { createHash } from 'node:crypto';
import { copyFile, lstat, mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const sha = (bytes) => createHash('sha256').update(bytes).digest('hex');
const exists = async (file) => { try { await lstat(file); return true; } catch (error) { if (error.code === 'ENOENT') return false; throw error; } };

/** Build artifacts only; the Android shell separately verifies staged assets at runtime. */
export async function packageAndroidUi(input, output, {
  maxBytes = 32 * 1024 * 1024, maxFiles = 2000, maxManifestBytes = 512 * 1024,
} = {}) {
  input = path.resolve(input);
  output = path.resolve(output);
  const relativeOutput = path.relative(input, output);
  if (!relativeOutput || (!relativeOutput.startsWith(`..${path.sep}`) && relativeOutput !== '..' && !path.isAbsolute(relativeOutput))) {
    throw new Error('Output must be outside the UI input path');
  }
  const files = [];
  let total = 0;
  async function walk(directory, prefix = '') {
    const info = await lstat(directory);
    if (info.isSymbolicLink() || !info.isDirectory()) throw new Error('UI directories must not be links');
    for (const name of (await readdir(directory)).sort()) {
      if (name === '.' || name === '..' || /[\\:\x00-\x1f]/.test(name)) throw new Error('Invalid UI path');
      const source = path.join(directory, name);
      const relative = prefix + name;
      if (relative.length > 512) throw new Error('UI path limit exceeded');
      const stat = await lstat(source);
      if (stat.isSymbolicLink()) throw new Error('UI links are forbidden');
      if (stat.isDirectory()) await walk(source, `${relative}/`);
      else {
        if (!stat.isFile()) throw new Error('UI input must contain regular files');
        total += stat.size;
        if (total > maxBytes || files.length >= maxFiles) throw new Error('UI bundle limit exceeded');
        const bytes = await readFile(source);
        if (bytes.length !== stat.size) throw new Error('UI input changed during packaging');
        files.push({ path: relative, bytes: bytes.length, sha256: sha(bytes) });
      }
    }
  }
  await walk(input);
  if (!files.some((file) => file.path === 'index.html')) throw new Error('UI requires index.html');
  files.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  const manifest = { version: 1, id: sha(JSON.stringify(files)), files };
  const manifestText = `${JSON.stringify(manifest, null, 2)}\n`;
  if (Buffer.byteLength(manifestText, 'utf8') > maxManifestBytes) throw new Error('UI manifest limit exceeded');
  if (await exists(output)) {
    const info = await lstat(output);
    if (!info.isDirectory() || info.isSymbolicLink()
      || (await readdir(output)).some((name) => !['ui', 'ui-manifest.json'].includes(name))
      || !(await exists(path.join(output, 'ui-manifest.json')))) {
      throw new Error('Refusing to replace an unrecognized generated asset directory');
    }
  }
  const parent = path.dirname(output);
  await mkdir(parent, { recursive: true });
  const scratch = await mkdtemp(path.join(parent, '.android-ui-'));
  const staged = path.join(scratch, 'assets');
  const backup = path.join(scratch, 'previous');
  let preserveBackup = false;
  try {
    await mkdir(path.join(staged, 'ui'), { recursive: true });
    for (const file of files) {
      const target = path.join(staged, 'ui', file.path);
      await mkdir(path.dirname(target), { recursive: true });
      await copyFile(path.join(input, file.path), target);
      const bytes = await readFile(target);
      if (bytes.length !== file.bytes || sha(bytes) !== file.sha256) throw new Error('UI input changed during copying');
    }
    await writeFile(path.join(staged, 'ui-manifest.json'), manifestText, { flag: 'wx' });
    if (await exists(output)) await rename(output, backup);
    try { await rename(staged, output); }
    catch (error) {
      if (await exists(backup)) {
        try { await rename(backup, output); }
        catch (restoreError) {
          preserveBackup = true;
          throw new AggregateError([error, restoreError], `Previous generated assets retained at ${backup}`);
        }
      }
      throw error;
    }
  } finally {
    // Only this mkdtemp-owned child of the explicit output parent is removed.
    if (path.dirname(scratch) !== parent || !path.basename(scratch).startsWith('.android-ui-')) throw new Error('Invalid scratch cleanup path');
    if (!preserveBackup) await rm(scratch, { recursive: true, force: true });
  }
  return manifest;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const input = fileURLToPath(new URL('../dist/', import.meta.url));
  const output = fileURLToPath(new URL('../../android/app/build/generated/webAssets/', import.meta.url));
  const manifest = await packageAndroidUi(input, output);
  console.log(`Android UI ${manifest.id}: ${manifest.files.length} files / ${manifest.files.reduce((sum, file) => sum + file.bytes, 0)} bytes`);
}
