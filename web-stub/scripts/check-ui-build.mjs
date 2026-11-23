import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import { gzipSync } from 'node:zlib';

const root = new URL('../dist/', import.meta.url);
const metadata = JSON.parse(await readFile(new URL('assets/ui-build.json', root), 'utf8'));
for (const file of metadata.files) {
  const path = new URL(file.file, root);
  assert.equal(file.bytes, (await stat(path)).size, file.file);
  assert.equal(file.gzipBytes, gzipSync(await readFile(path), { level: metadata.gzipLevel }).length, file.file);
}
for (const [prefix, initial] of [['initial', true], ['total', false]]) for (const [name, kind] of [['Js', 'js'], ['Css', 'css']]) {
  const files = metadata.files.filter((file) => file.kind === kind && (!initial || file.initial));
  assert.equal(metadata[`${prefix}${name}Bytes`], files.reduce((sum, file) => sum + file.bytes, 0));
  assert.equal(metadata[`${prefix}${name}GzipBytes`], files.reduce((sum, file) => sum + file.gzipBytes, 0));
}
assert.ok(metadata.initialJsBytes > 0);
assert.ok(metadata.files.filter((file) => file.kind === 'js' && file.dynamic).length >= 3);
assert.ok(metadata.initialCssBytes > 0);
const sourceNotices = await readFile(new URL('assets/project-nomad-NOTICE.txt', root), 'utf8');
assert.ok(sourceNotices.includes('Copyright 2024-2026 Crosstalk Solutions LLC'));
assert.ok(sourceNotices.includes('c903da48c97bc02f5cdfeff2ebcde00a82dfec8e'));
assert.ok(sourceNotices.includes('Apache License'));
const dependencyNotices = await readFile(new URL('assets/react-ui-NOTICES.txt', root), 'utf8');
for (const name of ['react-markdown', 'remark-gfm', '@tabler/icons-react']) assert.ok(dependencyNotices.includes(`=== ${name} `));
console.log(`Verified initial JS+CSS ${metadata.initialJsBytes + metadata.initialCssBytes} bytes; total JS+CSS ${metadata.totalJsBytes + metadata.totalCssBytes} bytes`);
