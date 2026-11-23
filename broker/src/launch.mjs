import {access,readFile,mkdir} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import https from 'node:https';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const here=path.dirname(fileURLToPath(import.meta.url));
const directory=process.argv[2] || path.join(process.env.LOCALAPPDATA||path.join(os.homedir(),'.local','share'),'TonalliChat');
await mkdir(directory,{recursive:true,mode:0o700});
try{await access(path.join(directory,'config.json'));}catch{
  const initialized=spawn(process.execPath,[path.join(here,'init.mjs'),directory],{stdio:'inherit',windowsHide:true});
  const code=await new Promise((resolve,reject)=>{initialized.once('error',reject);initialized.once('exit',resolve);});
  if(code!==0)throw Error('Broker initialization failed; existing files were retained');
}
const config=JSON.parse(await readFile(path.join(directory,'config.json'),'utf8'));
const child=spawn(process.execPath,[path.join(here,'main.mjs'),directory],{stdio:'inherit',windowsHide:true});
let closed=false;child.once('error',()=>{closed=true;process.exitCode=1;});child.once('exit',code=>{closed=true;process.exitCode=code??1;});
// The inherited console delivers Ctrl+C to the service too; do not TerminateProcess it on Windows.
process.once('SIGINT',()=>{});
const address=config.bind==='127.0.0.1'?'localhost':config.bind;
const url=`https://${address.includes(':')?'['+address+']':address}:${config.port}`;
const ca=await readFile(path.join(directory,'tls-cert.pem'));
for(let attempt=0;attempt<30&&!closed;attempt++){
  const ready=await new Promise(resolve=>{const request=https.get(url+'/api/session',{ca,timeout:500},response=>{response.resume();resolve(response.statusCode===200);});request.on('timeout',()=>request.destroy());request.on('error',()=>resolve(false));});
  if(ready){
    console.log(`Chat is ready at ${url}. Use the Chat administrator's Stop broker button or Ctrl+C to stop.`);
    if(process.platform==='win32')spawn('rundll32.exe',['url.dll,FileProtocolHandler',url],{stdio:'ignore',windowsHide:true}).on('error',()=>{});
    break;
  }
  await new Promise(resolve=>setTimeout(resolve,500));
}
