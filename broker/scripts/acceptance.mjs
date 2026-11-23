// Real broker/helper/MQTT clients only. RF acceptance belongs to the connected physical radio.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {readFile,writeFile,mkdtemp,mkdir,cp,symlink} from 'node:fs/promises';
import {createHash,randomBytes,X509Certificate} from 'node:crypto';
import https from 'node:https';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import mqtt from 'mqtt';
import {X509Certificate as ParsedCertificate} from '@peculiar/x509';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const packageAt=process.argv.indexOf('--package');
const packaged=packageAt<0?null:path.resolve(process.argv[packageAt+1]);
const serviceRoot=packaged||path.join(root,'broker');
const helper=path.join(packaged?path.join(packaged,'bin'):path.join(root,'target/release'),process.platform==='win32'?'tonalli-chat-store.exe':'tonalli-chat-store');
const bindAt=process.argv.indexOf('--bind'),bind=bindAt<0?'127.0.0.1':process.argv[bindAt+1];
const keep=process.argv.includes('--keep');
const directory=await mkdtemp(path.join(os.tmpdir(),'tonalli-chat-acceptance-'));
const data=path.join(directory,'private'),publicRoot=path.join(directory,'public');
await mkdir(data,{mode:0o700});await cp(packaged?path.join(packaged,'public'):path.join(root,'web-stub/dist'),publicRoot,{recursive:true});
const results=[];let service,owner,ca,base;
const nonce=()=>randomBytes(16).toString('hex');
const digest=bytes=>createHash('sha256').update(bytes).digest('hex');
function runNode(file,args=[],input){return new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(serviceRoot,'src',file),...args],{stdio:['pipe','pipe','pipe'],windowsHide:true});let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);child.on('error',reject);child.on('exit',code=>resolve({code,stdout,stderr}));child.stdin.end(input);});}
function request(route,{method='GET',body,session,origin=base,csrf,trust=ca}={}){
 return new Promise((resolve,reject)=>{const headers={Origin:origin,...(session?{Cookie:session.cookie,'X-Tonalli-Csrf':csrf??session.csrf}:{}),...(body===undefined?{}:{'Content-Type':'application/json'})};const req=https.request(base+route,{method,headers,ca:trust,rejectUnauthorized:true,family:4,timeout:5000,agent:false},res=>{const chunks=[];let size=0;res.on('data',b=>{size+=b.length;if(size>1048576){res.destroy();reject(Error('Response exceeded bound'));}else chunks.push(b);});res.on('end',()=>{const bytes=Buffer.concat(chunks);let value;try{value=JSON.parse(bytes.toString('utf8'));}catch{}resolve({status:res.statusCode,value,headers:res.headers,bytes});});});req.on('error',reject);req.on('timeout',()=>req.destroy(Error('Request timed out')));req.end(body===undefined?undefined:JSON.stringify(body));});
}
async function login(user,password){const response=await request('/api/chat/login',{method:'POST',body:{user,password}});assert.equal(response.status,200);const cookie=response.headers['set-cookie']?.[0]?.split(';')[0];assert.ok(cookie);const session=await request('/api/session',{session:{cookie,csrf:''}});assert.equal(session.status,200);return {cookie,csrf:session.value.csrf,user:session.value.chat_user};}
async function start(){
 service=spawn(process.execPath,[path.join(serviceRoot,'src/main.mjs'),data],{stdio:['ignore','pipe','pipe'],windowsHide:true,env:{...process.env,TONALLI_CHAT_STORE:helper,TONALLI_CHAT_UI:publicRoot}});
 service.stderr.on('data',()=>{});service.stdout.on('data',()=>{});
 await new Promise((resolve,reject)=>{service.once('error',reject);let attempt=0;async function poll(){if(service.exitCode!==null)return reject(Error('Broker exited during startup'));try{if((await request('/api/session')).status===200)return resolve();}catch{}if(++attempt>50)return reject(Error('Broker startup timed out'));setTimeout(poll,100);}void poll();});
}
async function stop(){if(!service||service.exitCode!==null)return;const child=service;await request('/api/chat/admin/stop',{method:'POST',body:{},session:owner});await new Promise((resolve,reject)=>{if(child.exitCode!==null)return resolve();const timer=setTimeout(()=>reject(Error('Broker did not stop gracefully')),15000);child.once('exit',code=>{clearTimeout(timer);code===0?resolve():reject(Error('Broker stopped with failure'));});});}
async function check(name,work){await work();results.push({name,passed:true});console.log(`PASS ${name}`);}
function connect(user,password,options={}){return new Promise((resolve,reject)=>{const client=mqtt.connect(base.replace(/^https:/,'wss:')+'/mqtt',{username:user,password,ca,rejectUnauthorized:true,clean:true,protocolVersion:4,reconnectPeriod:0,queueQoSZero:false,connectTimeout:5000,clientId:'checked-'+nonce(),...options});client.once('connect',()=>resolve(client));client.once('error',error=>{client.end(true);reject(error);});});}
const ownerPasswordResult=await runNode('init.mjs',[data,'--bind',bind]);
assert.equal(ownerPasswordResult.code,0,'Broker initialization failed');
const ownerPassword=/\nPassword: ([^\r\n]+)/.exec(ownerPasswordResult.stdout)?.[1];assert.ok(ownerPassword);
const config=JSON.parse(await readFile(path.join(data,'config.json'),'utf8'));
const port=await new Promise((resolve,reject)=>{const listener=net.createServer();listener.once('error',reject);listener.listen(0,bind,()=>{const port=listener.address().port;listener.close(()=>resolve(port));});});
config.port=port;await writeFile(path.join(data,'config.json'),JSON.stringify(config,null,2));
base=`https://${bind}:${port}`;ca=await readFile(path.join(data,'tls-cert.pem'));
const userPassword=randomBytes(24).toString('base64url'),gatewayPassword=randomBytes(24).toString('base64url');
try{
 await start();owner=await login('owner',ownerPassword);
 await check('TLS requires SHA-256 signature and explicit valid trust',async()=>{assert.equal(new ParsedCertificate(ca).signatureAlgorithm.hash?.name,'SHA-256');await assert.rejects(request('/api/session',{trust:Buffer.from('')}));assert.equal((await request('/api/session')).value.chat_broker,true);});
 await check('authentication, CSRF and Origin boundaries',async()=>{
  assert.equal((await request('/api/chat/rooms')).status,401);
  assert.equal((await request('/api/chat/login',{method:'POST',body:{user:'owner',password:'wrong'}})).status,403);
  assert.equal((await request('/api/chat/rooms/general',{method:'POST',session:owner,csrf:'wrong',body:{text:'denied',nonce:nonce()}})).status,403);
  assert.equal((await request('/api/chat/rooms',{session:owner,origin:'https://other.invalid'})).status,403);
 });
 await check('administrator GUI protocol and independent room grants',async()=>{
  assert.equal((await request('/api/chat/admin',{method:'POST',session:owner,body:{action:'room',id:'hidden',name:'Hidden room'}})).status,200);
  for(const user of [{id:'checked-reader',name:'Checked reader',role:'participant',read:['general'],send:['general'],bridge:[],password:userPassword},{id:'checked-readonly',name:'Checked read only',role:'participant',read:['general'],send:[],bridge:[],password:userPassword},{id:'checked-gateway',name:'Checked gateway',role:'gateway',read:['general'],send:['general'],bridge:[],password:gatewayPassword}])assert.equal((await request('/api/chat/admin',{method:'POST',session:owner,body:{action:'user',...user}})).status,200);
 });
 let reader=await login('checked-reader',userPassword),readonly=await login('checked-readonly',userPassword),gateway=await login('checked-gateway',gatewayPassword);
 await check('participant cannot administer, cross rooms, forge RF or access private app data',async()=>{
  assert.equal((await request('/api/chat/admin',{session:reader})).status,404);
  assert.equal((await request('/api/chat/rooms/hidden',{session:reader})).status,403);
  assert.equal((await request('/api/chat/rooms/general',{method:'POST',session:readonly,body:{text:'denied',nonce:nonce()}})).status,403);
  assert.equal((await request('/api/chat/rooms/general',{method:'POST',session:reader,body:{text:'denied',nonce:nonce(),origin:'radio',node:'!00000001',channel:0}})).status,403);
  assert.equal((await request('/api/chat/rooms/general/bridge',{method:'POST',session:gateway,body:{active:true,channel:0,name:'Denied',to_radio:true,to_broker:true}})).status,403);
  for(const route of ['/api/notes','/api/preferences','/api/files','/api/connections/radio'])assert.equal((await request(route,{session:reader})).status,404);
 });
 const text='Checked literal <img src="https://other.invalid/x"> & Español';const identity=nonce();let first;
 await check('real text fan-out, replay, immutable nonce and size bounds',async()=>{
  first=await request('/api/chat/rooms/general',{method:'POST',session:reader,body:{text,nonce:identity}});assert.equal(first.status,200);assert.equal(first.value.message.actor.id,'checked-reader');assert.equal(first.value.message.origin,'broker');assert.equal(first.value.message.status,'broker_received');
  const duplicate=await request('/api/chat/rooms/general',{method:'POST',session:reader,body:{text,nonce:identity}});assert.equal(duplicate.value.message.id,first.value.message.id);
  assert.notEqual((await request('/api/chat/rooms/general',{method:'POST',session:reader,body:{text:'different',nonce:identity}})).status,200);
  assert.equal((await request('/api/chat/rooms/general',{method:'POST',session:reader,body:{text:'x'.repeat(1001),nonce:nonce()}})).status,400);
  const replay=await request('/api/chat/rooms/general',{session:readonly});assert.equal(replay.value.messages.filter(m=>m.nonce===identity).length,1);assert.equal(replay.value.messages.find(m=>m.nonce===identity).text,text);assert.deepEqual(replay.value.radio_routes,[]);
 });
 await check('real authenticated MQTT.js WSS publish and subscription',async()=>{
  const client=await connect('checked-reader',userPassword);
  try{await client.subscribeAsync('tonalli/rooms/general/events');const unique=nonce();const incoming=new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('MQTT event timed out')),5000);client.on('message',(topic,payload)=>{const message=JSON.parse(payload);if(message.nonce===unique){clearTimeout(timer);resolve(message);}});});await client.publishAsync('tonalli/rooms/general/send',JSON.stringify({text:'Real MQTT client message',nonce:unique}),{qos:1});const message=await incoming;assert.equal(message.actor.id,'checked-reader');assert.equal(message.origin,'broker');let denied=false;try{const grant=await client.subscribeAsync('tonalli/rooms/+/events');denied=grant.every(g=>g.qos===128);}catch{denied=true;}assert.equal(denied,true);}finally{await client.endAsync(true);}
 });
 await check('wrong MQTT credentials and automatic will publish denied',async()=>{
  await assert.rejects(connect('checked-reader','wrong'));
  await assert.rejects(connect('checked-reader',userPassword,{will:{topic:'tonalli/rooms/general/send',payload:JSON.stringify({text:'must not publish',nonce:nonce()}),qos:0,retain:false}}));
  const history=await request('/api/chat/rooms/general',{session:reader});assert.equal(history.value.messages.some(m=>m.text==='must not publish'),false);
 });
 await check('one-use invitation, signed-in protection and grant revocation',async()=>{
  const invite=await request('/api/chat/admin/invite',{method:'POST',session:owner,body:{user:'checked-reader'}});assert.equal(invite.status,200);const token=new URL(invite.value.url).hash.slice(6);
  assert.equal((await request('/api/chat/join',{method:'POST',session:owner,body:{token}})).status,409);
  assert.equal((await request('/api/chat/join',{method:'POST',body:{token}})).status,200);assert.equal((await request('/api/chat/join',{method:'POST',body:{token}})).status,403);
  assert.equal((await request('/api/chat/admin',{method:'POST',session:owner,body:{action:'user',id:'checked-readonly',name:'Checked read only',role:'participant',read:[],send:[],bridge:[]}})).status,200);assert.equal((await request('/api/chat/rooms',{session:readonly})).status,401);
 });
 await check('private configuration and key cannot escape public root',async()=>{
  await symlink(data,path.join(publicRoot,'escape'),process.platform==='win32'?'junction':'dir');
  for(const route of ['/escape/history.key','/escape/config.json','/history.key'])assert.equal((await request(route)).status,404);
 });
 await check('encrypted restart, no plaintext and wrong-key unchanged',async()=>{
  await stop();const database=path.join(data,'history.db'),bytes=await readFile(database),before=digest(bytes);assert.equal(bytes.subarray(0,16).equals(Buffer.from('SQLite format 3\0')),false);assert.equal(bytes.includes(Buffer.from(text)),false);assert.equal(bytes.includes(Buffer.from(userPassword)),false);
  const key=(await readFile(path.join(data,'history.key'),'utf8')).trim();const wrong=(key[0]==='0'?'1':'0')+key.slice(1);
  const failed=await new Promise((resolve,reject)=>{const child=spawn(helper,[],{stdio:['pipe','pipe','pipe'],windowsHide:true});child.stdout.resume();child.stderr.resume();child.on('error',reject);child.on('exit',code=>resolve(code));child.stdin.end(JSON.stringify({path:database,key:wrong})+'\n');});assert.notEqual(failed,0);assert.equal(digest(await readFile(database)),before);
  await start();owner=await login('owner',ownerPassword);reader=await login('checked-reader',userPassword);const history=await request('/api/chat/rooms/general',{session:reader});assert.equal(history.value.messages.find(m=>m.nonce===identity).text,text);assert.deepEqual(history.value.radio_routes,[]);
 });
 // The real native gateway will claim this grant only after its actual radio handshake and explicit UI consent.
 assert.equal((await request('/api/chat/admin',{method:'POST',session:owner,body:{action:'user',id:'checked-gateway',name:'Checked gateway',role:'gateway',read:['general'],send:['general'],bridge:['general']}})).status,200);
 const metadata={protocol_checks:results,radio_tested:false,native_apps_tested:false,docker_tested:false,helper_sha256:digest(await readFile(helper)),certificate_sha256:new X509Certificate(ca).fingerprint256,url:base};
 await writeFile(path.join(directory,'RESULTS.json'),JSON.stringify(metadata,null,2));
 await writeFile(path.join(directory,'native-profiles.json'),JSON.stringify({desktop:{url:base,user:'checked-reader',password:userPassword,certificate:ca.toString()},phone:{url:base,user:'checked-gateway',password:gatewayPassword,certificate:ca.toString()},owner:{url:base,user:'owner',password:ownerPassword,certificate:ca.toString()}},null,2),{mode:0o600});
 console.log(JSON.stringify({...metadata,directory,service_pid:service.pid,kept:keep}));
 if(keep)await new Promise(resolve=>service.once('exit',resolve));else await stop();
}catch(error){console.error(`FAILED: ${error.message}`);process.exitCode=1;try{await stop();}catch{if(service&&service.exitCode===null)service.kill();}}
