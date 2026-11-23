// Exercises the actual Compose service and Linux SQLCipher helper; no radio is simulated.
import assert from 'node:assert/strict';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdtemp,writeFile} from 'node:fs/promises';
import {randomBytes,X509Certificate} from 'node:crypto';
import https from 'node:https';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {setTimeout as delay} from 'node:timers/promises';
import mqtt from 'mqtt';
const execute=promisify(execFile);
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const directory=await mkdtemp(path.join(os.tmpdir(),'tonalli-docker-chat-'));
const project='tonalli-chat-check-'+randomBytes(5).toString('hex');
const image=process.argv[2]||'tonalli-chat:unified-r1';
const override=path.join(directory,'compose-override.json');
await writeFile(override,JSON.stringify({services:{chat:{image}}}));
async function docker(...args){return(await execute('docker',args,{windowsHide:true,timeout:60000,maxBuffer:4*1024*1024})).stdout;}
const compose=(...args)=>docker('compose','--ansi','never','-p',project,'-f',path.join(root,'broker/compose.yaml'),'-f',override,...args);
const base='https://127.0.0.1:8443';let ca,owner;
function request(route,{method='GET',body,session,origin=base,trust=ca}={}){return new Promise((resolve,reject)=>{const req=https.request(base+route,{method,ca:trust,rejectUnauthorized:true,family:4,agent:false,timeout:5000,headers:{Origin:origin,'Content-Type':'application/json',...(session?{Cookie:session.cookie,'X-Tonalli-Csrf':session.csrf}:{})}},res=>{let bytes=0;const chunks=[];res.on('data',chunk=>{bytes+=chunk.length;if(bytes>1048576){res.destroy();reject(Error('Response too large'));}else chunks.push(chunk);});res.on('end',()=>{const data=Buffer.concat(chunks);let value;try{value=JSON.parse(data.toString());}catch{}resolve({status:res.statusCode,value,headers:res.headers});});});req.on('error',reject);req.on('timeout',()=>req.destroy(Error('Request timed out')));req.end(body===undefined?undefined:JSON.stringify(body));});}
async function ready(){for(let attempt=0;attempt<100;attempt++){try{if((await request('/api/session')).status===200)return;}catch{}await delay(100);}throw Error('Compose service did not become ready');}
async function login(password){const r=await request('/api/chat/login',{method:'POST',body:{user:'owner',password}});assert.equal(r.status,200);const cookie=r.headers['set-cookie']?.[0]?.split(';')[0];assert.ok(cookie);const s=await request('/api/session',{session:{cookie,csrf:''}});return{cookie,csrf:s.value.csrf};}
const checks=[];const pass=name=>{checks.push(name);console.log('PASS '+name);};
try{
 const initialized=await compose('run','--rm','--no-deps','chat','node','src/init.mjs','/data','--container');
 await writeFile(path.join(directory,'initialization-output.txt'),initialized,{mode:0o600});
 const password=/\nPassword: ([^\r\n]+)/.exec(initialized)?.[1];assert.ok(password);
 ca=await compose('run','--rm','--no-deps','chat','node','-e',"process.stdout.write(require('fs').readFileSync('/data/tls-cert.pem'))");
 await compose('up','-d','--no-build');await ready();
 const container=(await compose('ps','-q','chat')).trim();assert.ok(container);
 const inspect=JSON.parse(await docker('inspect',container))[0];
 assert.equal(inspect.Config.User,'node');assert.equal(inspect.HostConfig.ReadonlyRootfs,true);assert.ok(inspect.HostConfig.CapDrop.includes('ALL'));assert.ok(inspect.HostConfig.SecurityOpt.some(v=>v.startsWith('no-new-privileges')));
 const bindings=Object.values(inspect.HostConfig.PortBindings).flat();assert.ok(bindings.length>0&&bindings.every(v=>v.HostIp==='127.0.0.1'));
 const volume=inspect.Mounts.find(v=>v.Destination==='/data');assert.equal(volume.Type,'volume');assert.ok(volume.Name.startsWith(project));
 pass('Compose uses an unprivileged process, read-only root, loopback port and private named volume');
 await assert.rejects(request('/api/session',{trust:Buffer.from('')}));assert.equal((await request('/api/chat/rooms')).status,401);owner=await login(password);
 assert.equal((await request('/api/chat/rooms',{session:owner,origin:'https://other.invalid'})).status,403);assert.equal((await request('/api/notes',{session:owner})).status,404);
 pass('real container HTTPS validates trust, authentication, Origin and private route boundaries');
 const text='Container chat check ñ <b>literal text</b>',nonce=randomBytes(16).toString('hex');
 const sent=await request('/api/chat/rooms/general',{method:'POST',session:owner,body:{text,nonce}});assert.equal(sent.status,200);assert.equal(sent.value.message.status,'broker_received');assert.equal(sent.value.message.text,text);
 const duplicate=await request('/api/chat/rooms/general',{method:'POST',session:owner,body:{text,nonce}});assert.equal(duplicate.value.message.id,sent.value.message.id);
 pass('Linux history helper persists a real message and deduplicates an immutable send');
 const client=await new Promise((resolve,reject)=>{const c=mqtt.connect(base.replace('https:','wss:')+'/mqtt',{username:'owner',password,ca,rejectUnauthorized:true,protocolVersion:4,clean:true,reconnectPeriod:0,queueQoSZero:false,connectTimeout:5000});c.once('connect',()=>resolve(c));c.once('error',error=>{c.end(true);reject(error);});});
 try{await client.subscribeAsync('tonalli/rooms/general/events',{qos:1});const tag=randomBytes(16).toString('hex');const incoming=new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('No real WSS message received')),5000);client.on('message',(_topic,payload)=>{const value=JSON.parse(payload);if(value.nonce===tag){clearTimeout(timer);resolve(value);}});});await client.publishAsync('tonalli/rooms/general/send',JSON.stringify({text:'Container WSS check',nonce:tag}),{qos:1});assert.equal((await incoming).actor.id,'owner');}finally{await client.endAsync(true);}
 pass('actual authenticated MQTT.js WSS send and fan-out work through the container port');
 assert.equal((await request('/api/chat/admin/stop',{method:'POST',body:{},session:owner})).status,200);await compose('stop','-t','15');
 assert.equal(JSON.parse(await docker('inspect',container))[0].State.ExitCode,0);
 await compose('up','-d','--no-build');await ready();owner=await login(password);
 const restored=await request('/api/chat/rooms/general',{session:owner});assert.equal(restored.value.messages.find(v=>v.nonce===nonce).text,text);assert.deepEqual(restored.value.radio_routes,[]);
 pass('normal Stop and cold container restart preserve history without enabling a radio route');
 assert.equal((await request('/api/chat/admin/stop',{method:'POST',body:{},session:owner})).status,200);await compose('stop','-t','15');
 const storageCheck=`const fs=require('fs'),c=require('crypto'),cp=require('child_process'),a=require('assert/strict');
 const file='/data/history.db',key=fs.readFileSync('/data/history.key','utf8').trim();
 const probe=candidate=>cp.spawnSync('/app/bin/tonalli-chat-store',[],{input:JSON.stringify({path:file,key:candidate})+String.fromCharCode(10),encoding:'utf8'});
 const valid=probe(key);a.equal(valid.status,0);a.equal(JSON.parse(valid.stdout.trim()).ready,true);
 const before=fs.readFileSync(file);a.equal(before.subarray(0,16).equals(Buffer.concat([Buffer.from('SQLite format 3'),Buffer.from([0])])),false);
 a.equal(before.includes(Buffer.from(${JSON.stringify(text)})),false);a.equal(fs.statSync('/data/history.key').mode&63,0);a.equal(fs.statSync('/data').mode&63,0);
 const wrong=(key[0]==='0'?'1':'0')+key.slice(1);const r=probe(wrong);a.equal(r.status,1);a.equal(r.stdout.trim(),'');
 a.equal(c.createHash('sha256').update(before).digest('hex'),c.createHash('sha256').update(fs.readFileSync(file)).digest('hex'));console.log('valid key opened; encrypted; wrong key refused; file unchanged; private modes');`;
 const storage=await compose('run','--rm','--no-deps','chat','node','-e',storageCheck);assert.match(storage,/file unchanged/);
 pass('Linux SQLCipher has no plaintext fallback; wrong key leaves bytes unchanged and keys stay private');
 const result={label:'docker',project,image,image_id:inspect.Image,volume:volume.Name,directory,certificate_sha256:new X509Certificate(ca).fingerprint256,checks,radio_tested:false,public_deployment:false};
 await writeFile(path.join(root,'target/unified-chat-r1/docker-acceptance.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}catch(error){console.error('Docker acceptance failed; private evidence: '+directory);console.error(error.code||error.message);process.exitCode=1;}
finally{try{await compose('stop','-t','15');}catch{} /* Keep the owned volume and evidence for review. */}
