import { writeFile } from 'node:fs/promises';
import { randomBytes, X509Certificate } from 'node:crypto';
import path from 'node:path';
import selfsigned from 'selfsigned';
import { privateDirectory, passwordHash, validate,localNetworks } from './config.mjs';
const directory = path.resolve(process.argv[2] || 'data');
await privateDirectory(directory);
const password = randomBytes(24).toString('base64url');
const bindArgument=process.argv.indexOf('--bind');
const bind=bindArgument>=0?process.argv[bindArgument+1]:process.argv.includes('--container')?'0.0.0.0':'127.0.0.1';
const config = validate({version:1,mode:'local',bind,port:8443,rooms:[{id:'general',name:'General'}],users:[{
  id:'owner',name:'Owner',role:'admin',passwordHash:await passwordHash(password),read:['general'],send:['general'],bridge:[],
}]});
const pems = await selfsigned.generate([{name:'commonName',value:'localhost'}],{
  keyType:'ec',curve:'P-256',algorithm:'sha256',days:365,
  extensions:[{name:'subjectAltName',altNames:[{type:2,value:'localhost'},...localNetworks().map(n=>({type:7,ip:n.address}))]}],
});
// Never overwrite an existing deployment's identity or history key.
for (const [name,value] of Object.entries({'tls-key.pem':pems.private,'tls-cert.pem':pems.cert,'history.key':randomBytes(32).toString('hex')})) {
  await writeFile(path.join(directory,name),value,{flag:'wx',mode:0o600});
}
await writeFile(path.join(directory,'config.json'),JSON.stringify(config,null,2),{flag:'wx',mode:0o600});
console.log(`Created private broker at ${directory}\nOpen https://localhost:8443\nUser: owner\nPassword: ${password}\nCertificate SHA-256: ${new X509Certificate(pems.cert).fingerprint256}\nSave the password securely. Keep this directory private and back up history.key with history.db.`);
