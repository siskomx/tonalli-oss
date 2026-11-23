import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';

const root = new URL('../public/assets/map/', import.meta.url);
try {
  const manifest = JSON.parse(await readFile(new URL('manifest.json', root), 'utf8'));
  for (const record of manifest.files) {
    const bytes = await readFile(new URL(record.path, root));
    if (bytes.length !== record.bytes || createHash('sha256').update(bytes).digest('hex') !== record.sha256) {
      throw new Error(`Asset checksum mismatch: ${record.path}`);
    }
  }
} catch (error) {
  console.error(`Map assets are missing or invalid: ${error.message}\nRun npm run prepare:maps explicitly, then npm run build. Preparation downloads pinned assets; build never downloads them.`);
  process.exitCode = 1;
}
