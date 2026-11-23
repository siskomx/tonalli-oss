// Retain exact installed image package versions and verbatim distribution notices.
import assert from 'node:assert/strict';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdir,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const execute=promisify(execFile);
const image=process.argv[2]||'tonalli-chat:unified-r1';
const destination=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../licenses/linux-runtime');
const program=String.raw`
const fs=require('fs'),cp=require('child_process'),crypto=require('crypto');
const packages=cp.execFileSync('dpkg-query',['-W','-f','${'${binary:Package}'}\t${'${Version}'}\n'],{encoding:'utf8'}).trim().split('\n').map(line=>{const [name,version]=line.split('\t');if(!/^[a-z0-9][a-z0-9+.:\-]*$/.test(name)||!version)throw Error('Invalid package inventory');const file='/usr/share/doc/'+name.split(':')[0]+'/copyright';return {name,version,notice:fs.existsSync(file)?file:null};});
const files=[...new Set(packages.map(p=>p.notice).filter(Boolean)),...fs.readdirSync('/usr/share/common-licenses').map(n=>'/usr/share/common-licenses/'+n)].filter(p=>fs.statSync(p).isFile());
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
process.stdout.write(JSON.stringify({node:process.version,architecture:process.arch,platform:process.platform,node_sha256:hash(process.execPath),helper_sha256:hash('/app/bin/tonalli-chat-store'),packages,files:files.map(file=>({file,data:fs.readFileSync(file).toString('base64')}))}));`;
// The dpkg format above must reach execFile literally, without shell interpolation.
const {stdout}=await execute('docker',['run','--rm','--network','none','--entrypoint','node',image,'-e',program],{windowsHide:true,maxBuffer:16*1024*1024,timeout:60000});
const result=JSON.parse(stdout);assert.equal(result.platform,'linux');assert.equal(result.node,'v22.23.2');assert.ok(result.packages.length>50);assert.ok(result.packages.every(p=>p.notice),'Installed package missing copyright file');
const notices=[];
for(const file of result.files){assert.ok(file.file.startsWith('/usr/share/doc/')||file.file.startsWith('/usr/share/common-licenses/'));const relative=file.file.slice('/usr/share/'.length);assert.ok(!relative.split('/').includes('..'));const target=path.join(destination,relative);await mkdir(path.dirname(target),{recursive:true});const bytes=Buffer.from(file.data,'base64');await writeFile(target,bytes);notices.push({file:relative,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')});}
delete result.files;
await writeFile(path.join(destination,'inventory.json'),JSON.stringify({...result,notices},null,2)+'\n');
console.log(JSON.stringify({packages:result.packages.length,notices:notices.length,node:result.node,helper_sha256:result.helper_sha256}));
