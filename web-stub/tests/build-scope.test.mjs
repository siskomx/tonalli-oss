import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, writeFile, mkdir, cp, rm, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';
import tailwindcss from '@tailwindcss/vite';

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

test('production CSS is stable when measurement scripts and test fixtures contain utilities', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'tonalli-css-scope-'));
  const owned = await realpath(temporary);
  try {
    await cp(join(webRoot, 'src'), join(temporary, 'src'), { recursive: true });
    // An index-only utility also proves that the HTML source remains included.
    const indexClass = ['m-', '[13px]'].join('');
    await writeFile(join(temporary, 'index.html'),
      `<link rel="stylesheet" href="/src/app/styles.css"><main class="${indexClass}"></main>`, 'utf8');
    const css = async () => {
      const result = await build({ configFile: false, root: temporary, publicDir: false, logLevel: 'silent',
        plugins: [tailwindcss()], resolve: { alias: { tailwindcss: join(webRoot, 'node_modules/tailwindcss/index.css') } },
        build: { write: false, minify: false, cssMinify: false, rollupOptions: { input: join(temporary, 'index.html') } } });
      return result.output.filter((item) => item.type === 'asset' && item.fileName.endsWith('.css'))
        .map((item) => String(item.source)).join('\n');
    };
    const before = await css();
    assert.match(before, /\.overflow-auto\b/, 'actual JSX utility is retained');
    assert.match(before, /margin: 13px/, 'index HTML utility is retained');
    await mkdir(join(temporary, 'scripts'));
    await mkdir(join(temporary, 'tests'));
    const noise = ['rotate-', '[17deg]', ' ', 'bg-', '[#123abc]'].join('');
    await writeFile(join(temporary, 'scripts/measurement.mjs'), `const fixture = '${noise}';`, 'utf8');
    await writeFile(join(temporary, 'tests/fixture.html'), `<div class="${noise}"></div>`, 'utf8');
    assert.equal(await css(), before, 'non-production text must not add shipped CSS');
  } finally {
    const current = await realpath(temporary);
    assert.equal(current, owned, 'cleanup target still belongs to this test');
    assert.ok(current.startsWith(resolve(tmpdir()) + sep), 'cleanup stays inside the temporary directory');
    await rm(temporary, { recursive: true });
  }
});
