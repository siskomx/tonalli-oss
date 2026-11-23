import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { packIdentity, root, sha256 } from './article-host.mjs';

export const wiki = {
  id: 'wiki', file: 'docs/spike/packs/wikipedia_en_all_nopic_2026-06.zim',
  version: '2026-06', bytes: 52690706555,
  sha256: '441a56d9e05b2d98f8ae9acb7986a513ed47904d73852c92dc6b7d50baa122e5',
  titleCount: 19191219,
};
const maps = [
  ['mx', 'mx-mexico-city-z8.pmtiles', 579622, '9a8d7ef3570d5cebd4e3ad24dd25ea88b22e578520dd19fbb9058c0248ab5238'],
  ['us', 'us-san-diego-z8.pmtiles', 573529, '056249419246ceadc38991b8f40e1fbf073b22c13b856724ff1301b086d18fb4'],
  ['ca', 'ca-montreal-z8.pmtiles', 780083, '6a152c16ad463083c8d1987d3ead4faa150816feb9d0b934ea0910970183f867'],
].map(([id, file, bytes, hash]) => ({ id, file: `docs/spike/packs/${file}`, bytes, sha256: hash }));

let verified;
export async function verifiedReactPacks() {
  if (verified) return verified;
  const path = join(root, wiki.file);
  const identity = await packIdentity(path);
  assert.equal(identity.bytes, String(wiki.bytes));
  const receiptPath = 'target/react-full-wiki-verified.json';
  let receipt;
  try { receipt = JSON.parse(await readFile(join(root, receiptPath), 'utf8')); } catch {}
  if (receipt?.actualSha256 !== wiki.sha256 || JSON.stringify(receipt.identity) !== JSON.stringify(identity)) {
    console.log('Verifying full Wikipedia SHA-256 once, outside UI tests/timings.');
    const started = performance.now();
    const actualSha256 = await sha256(path);
    assert.equal(actualSha256, wiki.sha256);
    assert.deepEqual(await packIdentity(path), identity, 'corpus changed during verification');
    receipt = { source: wiki, identity, actualSha256, verifiedAt: new Date().toISOString(),
      setupHashMs: performance.now() - started, verification: 'full streaming SHA-256; identity checked before/after' };
    await writeFile(join(root, receiptPath), `${JSON.stringify(receipt, null, 2)}\n`, 'utf8');
    console.log(`Verified full Wikipedia in ${receipt.setupHashMs.toFixed(1)} ms.`);
  } else console.log('Reusing full Wikipedia SHA receipt bound to unchanged path/size/mtime/file identity.');
  verified = { packs: [{ ...wiki, identity, verificationReceipt: receiptPath }, ...maps], receipt };
  return verified;
}
