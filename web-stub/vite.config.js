import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';
import tailwindcss from '@tailwindcss/vite';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';

function buildMetadata() {
  return { name: 'ui-build-metadata', async writeBundle(options, bundle) {
    const chunks = Object.values(bundle).filter((item) => item.type === 'chunk');
    const entry = chunks.find((item) => item.isEntry && item.name === 'index');
    if (!entry) this.error('Missing app entry for bundle accounting');
    const initial = new Set();
    function visit(name) {
      if (initial.has(name) || bundle[name]?.type !== 'chunk') return;
      initial.add(name);
      for (const dependency of bundle[name].imports) visit(dependency);
    }
    visit(entry.fileName);
    const html = await readFile(join(options.dir, 'index.html'), 'utf8');
    const initialCss = new Set([...html.matchAll(/<link\b[^>]*>/g)].filter(([tag]) => /rel="stylesheet"/.test(tag))
      .map(([tag]) => tag.match(/href="([^"]+)"/)[1].replace(/^\//, '')));
    const assets = [...chunks.map((chunk) => ({ file: chunk.fileName, kind: 'js', initial: initial.has(chunk.fileName), dynamic: chunk.isDynamicEntry })),
      ...Object.values(bundle).filter((item) => item.type === 'asset' && item.fileName.endsWith('.css'))
        .map((item) => ({ file: item.fileName, kind: 'css', initial: initialCss.has(item.fileName), dynamic: !initialCss.has(item.fileName) }))];
    const files = await Promise.all(assets.map(async (asset) => {
      const bytes = await readFile(join(options.dir, asset.file));
      return { ...asset, bytes: bytes.length, gzipBytes: gzipSync(bytes, { level: 9 }).length };
    }));
    const sum = (kind, initialOnly, field) => files.filter((file) => file.kind === kind && (!initialOnly || file.initial))
      .reduce((total, file) => total + file[field], 0);
    await writeFile(join(options.dir, 'assets/ui-build.json'), JSON.stringify({
      gzipLevel: 9,
      initialJsBytes: sum('js', true, 'bytes'), initialCssBytes: sum('css', true, 'bytes'),
      totalJsBytes: sum('js', false, 'bytes'), totalCssBytes: sum('css', false, 'bytes'),
      initialJsGzipBytes: sum('js', true, 'gzipBytes'), initialCssGzipBytes: sum('css', true, 'gzipBytes'),
      totalJsGzipBytes: sum('js', false, 'gzipBytes'), totalCssGzipBytes: sum('css', false, 'gzipBytes'), files,
    }, null, 2) + '\n', 'utf8');
  } };
}

export default defineConfig({
  plugins: [tailwindcss(), buildMetadata()],
  build: {
    rollupOptions: {
      input: {
        index: fileURLToPath(new URL('./index.html', import.meta.url)),
        map: fileURLToPath(new URL('./map.html', import.meta.url)),
        article: fileURLToPath(new URL('./article.html', import.meta.url)),
      },
    },
  },
});
