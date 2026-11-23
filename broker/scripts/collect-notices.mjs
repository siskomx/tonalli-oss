import {readFile,writeFile,readdir,mkdir} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const lock=JSON.parse(await readFile(path.join(root,'package-lock.json'),'utf8'));
let notice='Tonalli optional Chat broker dependency notices\n\n', table='| Package | Exact version | Declared license |\n| --- | --- | --- |\n';
for(const [location,pkg] of Object.entries(lock.packages)){
  if(!location)continue;
  if(!['MIT','ISC','BSD-3-Clause','Apache-2.0','0BSD'].includes(pkg.license))throw Error(`License review required: ${location}`);
  const name=location.slice(location.lastIndexOf('node_modules/')+13);
  table+=`| ${name} | ${pkg.version} | ${pkg.license} |\n`;
  const directory=path.join(root,location);
  const files=(await readdir(directory)).filter(f=>/^(licen[cs]e|copying|notice)(\.|$|-)/i.test(f));
  if(!files.length)throw Error(`License text missing: ${location}`);
  for(const file of files){notice+=`\n=== ${name} ${pkg.version}: ${file} ===\n`+await readFile(path.join(directory,file),'utf8')+'\n';}
}
await mkdir(path.join(root,'licenses'),{recursive:true});
await writeFile(path.join(root,'licenses/THIRD-PARTY-NOTICES.txt'),notice);
await writeFile(path.join(root,'licenses/dependencies.md'),table);
console.log(`Recorded ${Object.keys(lock.packages).length-1} exact dependency versions and license texts.`);
