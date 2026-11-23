import {readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
const version='22.23.2';
if(process.version!==`v${version}`||process.platform!=='win32'||process.arch!=='x64')throw Error('Use the exact Windows x64 Node22.23.2 runtime');
async function download(url){const response=await fetch(url);if(!response.ok)throw Error(`Download failed: ${response.status}`);return Buffer.from(await response.arrayBuffer());}
const checksumUrl=`https://nodejs.org/dist/v${version}/SHASUMS256.txt`;
const licenseUrl=`https://raw.githubusercontent.com/nodejs/node/v${version}/LICENSE`;
const sums=(await download(checksumUrl)).toString('utf8');
const expected=sums.split(/\r?\n/).map(line=>line.trim().split(/\s+/)).find(([,file])=>file==='win-x64/node.exe')?.[0];
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const actual=hash(await readFile(process.execPath));if(!expected||actual!==expected)throw Error('Installed Node does not match the official Windows binary');
const license=await download(licenseUrl);
await writeFile(new URL(`../licenses/Node-v${version}-LICENSE.txt`,import.meta.url),license);
await writeFile(new URL('../licenses/node-runtime.json',import.meta.url),JSON.stringify({version,arch:'win-x64',node_sha256:actual,license_sha256:hash(license),checksum_url:checksumUrl,license_url:licenseUrl},null,2)+'\n');
console.log(JSON.stringify({version,node_sha256:actual,license_bytes:license.length,license_sha256:hash(license)}));
