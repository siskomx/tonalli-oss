import https from 'node:https';
import { readFile, stat, realpath } from 'node:fs/promises';
import { randomBytes, X509Certificate } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { Aedes } from 'aedes';
import { WebSocketServer, createWebSocketStream } from 'ws';
import { loadConfig, saveConfig, passwordHash, verifyPassword, identifier,localNetworks } from './config.mjs';
import { openHistory } from './history.mjs';

const here=path.dirname(fileURLToPath(import.meta.url));
const directory=path.resolve(process.argv[2] || 'data');
let config=await loadConfig(directory);
const helper=process.env.TONALLI_CHAT_STORE || path.resolve(here,'../bin',process.platform==='win32'?'tonalli-chat-store.exe':'tonalli-chat-store');
const history=await openHistory(directory,helper);
const staticRoot=path.resolve(process.env.TONALLI_CHAT_UI || path.join(here,'../public'));
const privateRoot=await realpath(directory),publicRoot=await realpath(staticRoot);
if(privateRoot===publicRoot || privateRoot.startsWith(publicRoot+path.sep))throw Error('Private broker data must be outside the public UI');
const certificate=await readFile(path.join(directory,'tls-cert.pem'));
const sessions=new Map(), rates=new Map(), radioRoutes=new Map(), invites=new Map();
const csrf=()=>randomBytes(24).toString('hex');
const broker=await Aedes.createBroker({queueLimit:16,maxInflightInbound:16,maxTopicLevels:6,drainTimeout:10000,connectTimeout:10000});
const userById=id=>config.users.find(u=>u.id===id);
const safeUser=u=>({id:u.id,name:u.name,role:u.role,read:u.read,send:u.send,bridge:u.bridge});
function limited(key,maximum,period=60000) {
  const now=Date.now();
  if(rates.size>1024) for(const [k,r] of rates) if(r.until<=now) rates.delete(k);
  let rate=rates.get(key); if(!rate || rate.until<=now){rate={count:0,until:now+period};rates.set(key,rate);}
  return ++rate.count>maximum;
}
const denied=()=>Object.assign(Error('Permission denied'),{status:403});
const invalid=()=>Object.assign(Error('Invalid request'),{status:400});
let sending=Promise.resolve();
let pendingSubmissions=0;const pendingByUser=new Map();
function routes(room){return [...radioRoutes.values()].filter(r=>r.room===room&&r.expires>Date.now()&&userById(r.gateway)?.bridge.includes(room)).map(({expires,room,...r})=>r);}
broker.preConnect=(client,packet,callback)=>callback(null,!packet.will&&packet.clean===true);
function submit(user, room, value) {
  user=userById(user.id);if(!user)return Promise.reject(denied());
  if(!identifier(room) || !user.send.includes(room)) return Promise.reject(denied());
  if(!value || typeof value.text!=='string' || !value.text.trim() || Buffer.byteLength(value.text)>1000 || /[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(value.text) || !/^[a-zA-Z0-9_-]{16,80}$/.test(value.nonce||'')) return Promise.reject(invalid());
  if(limited(`send:${user.id}`,30)) return Promise.reject(Object.assign(Error('Please wait before sending again'),{status:429}));
  // The authenticated gateway supplies real radio provenance; participants cannot claim it.
  const radio=value.origin==='radio';
  if(radio && (user.role!=='gateway' || !user.bridge.includes(room) || !/^![a-f0-9]{8}$/.test(value.node||'') || !Number.isInteger(value.channel) || value.channel<0 || value.channel>7)) return Promise.reject(denied());
  if(!radio&&routes(room).some(r=>r.to_radio)&&Buffer.byteLength(`[Chat: ${user.name}] ${value.text}`)>200)return Promise.reject(Object.assign(Error('Shorten the message for the connected radio'),{status:400}));
  if(pendingSubmissions>=64||(pendingByUser.get(user.id)||0)>=4)return Promise.reject(Object.assign(Error('Chat is busy; try again shortly'),{status:429}));
  pendingSubmissions++;pendingByUser.set(user.id,(pendingByUser.get(user.id)||0)+1);
  const message={text:value.text,actor:{id:user.id,name:user.name},origin:radio?'radio':'broker',...(radio?{node:value.node,channel:value.channel}:{}),status:'broker_received',time:Date.now(),nonce:value.nonce};
  const result=sending.then(()=>history.request({op:'append',room,actor:radio?`radio:${value.node}:${value.channel}`:user.id,nonce:value.nonce,message}));
  sending=result.catch(()=>{});
  return result.then(async stored=>{
    await new Promise((resolve,reject)=>broker.publish({topic:`tonalli/rooms/${room}/events`,payload:Buffer.from(JSON.stringify(stored)),qos:0,retain:false},e=>e?reject(e):resolve()));
    return stored;
  }).finally(()=>{pendingSubmissions--;const count=pendingByUser.get(user.id)-1;if(count)pendingByUser.set(user.id,count);else pendingByUser.delete(user.id);});
}
broker.authenticate=async(client,username,password,callback)=>{
  try {
    if(limited('mqtt-login',60) || broker.connectedClients>=128) return callback(null,false);
    const user=userById(String(username));
    const valid=user && await verifyPassword(password?.toString('utf8'),user.passwordHash);
    if(valid)client.tonalliUser=user.id;
    callback(null,Boolean(valid));
  } catch {callback(null,false);}
};
broker.authorizeSubscribe=(client,subscription,callback)=>{
  const match=/^tonalli\/rooms\/([a-zA-Z0-9_-]{1,64})\/events$/.exec(subscription.topic);
  const user=userById(client.tonalliUser);
  callback(null,match && user?.read.includes(match[1]) ? {...subscription,qos:0}:null);
};
broker.authorizeForward=(client,packet)=>{
  const match=/^tonalli\/rooms\/([a-zA-Z0-9_-]{1,64})\/events$/.exec(packet.topic);
  return match && userById(client.tonalliUser)?.read.includes(match[1]) ? packet:null;
};
broker.authorizePublish=(client,packet,callback)=>{
  if(!client)return callback(null);
  const match=/^tonalli\/rooms\/([a-zA-Z0-9_-]{1,64})\/send$/.exec(packet.topic);
  const user=userById(client.tonalliUser);
  if(!match || !user || packet.retain || packet.payload.length>4096)return callback(denied());
  try { submit(user,match[1],JSON.parse(packet.payload.toString('utf8'))).then(()=>callback(null),()=>callback(denied())); }
  catch {callback(invalid());}
};
function sessionFor(req) {
  const token=/\btonalli_chat=([a-f0-9]{48})\b/.exec(req.headers.cookie||'')?.[1];
  const session=sessions.get(token);
  if(!session || session.expires<Date.now()){if(token)sessions.delete(token);return null;}
  const user=userById(session.user); return user?{...session,user,token}:null;
}
async function body(req) {
  let size=0;const chunks=[];
  for await(const chunk of req){size+=chunk.length;if(size>32768)throw invalid();chunks.push(chunk);}
  try{return JSON.parse(Buffer.concat(chunks).toString('utf8'));}catch{throw invalid();}
}
function json(res,status,value) {res.writeHead(status,{'Content-Type':'application/json; charset=utf-8','Cache-Control':'no-store'});res.end(JSON.stringify(value));}
function signedIn(res,user){
  for(const [key,session]of sessions)if(session.expires<Date.now())sessions.delete(key);
  if(sessions.size>=128)throw Object.assign(Error('Broker is full'),{status:503});
  const token=csrf();sessions.set(token,{user:user.id,csrf:csrf(),expires:Date.now()+12*60*60*1000});
  res.setHeader('Set-Cookie',`tonalli_chat=${token}; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200`);
  return json(res,200,{ok:true});
}
const mime={'.html':'text/html; charset=utf-8','.js':'text/javascript; charset=utf-8','.css':'text/css; charset=utf-8','.json':'application/json','.svg':'image/svg+xml','.png':'image/png','.woff2':'font/woff2'};
let changing=Promise.resolve();
async function handler(req,res) {
  res.setHeader('X-Content-Type-Options','nosniff');res.setHeader('Referrer-Policy','no-referrer');
  res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
  try {
    const origin=`https://${req.headers.host}`;
    if(req.headers.origin && req.headers.origin!==origin)throw denied();
    const url=new URL(req.url,origin);
    const session=sessionFor(req);
    if(url.pathname==='/api/session' && req.method==='GET')return json(res,200,{csrf:session?.csrf||'signed-out',chat_broker:true,chat_user:session?safeUser(session.user):null});
    if(url.pathname==='/api/chat/login' && req.method==='POST') {
      if(req.headers.origin!==origin || limited('http-login',60))throw denied();
      const value=await body(req),user=userById(value.user);
      if(!user || !await verifyPassword(value.password,user.passwordHash))throw denied();
      return signedIn(res,user);
    }
    if(url.pathname==='/api/chat/join'&&req.method==='POST'){
      if(req.headers.origin!==origin||limited('http-login',60))throw denied();
      if(session)throw Object.assign(Error('Sign out before joining another invitation'),{status:409});
      const value=await body(req),invite=invites.get(value.token),user=invite&&userById(invite.user);
      if(!/^[a-f0-9]{48}$/.test(value.token||'')||!invite||invite.expires<Date.now()||user?.role!=='participant')throw denied();
      invites.delete(value.token);return signedIn(res,user);
    }
    if(url.pathname.startsWith('/api/')) {
      if(!session)throw Object.assign(Error('Sign in to Chat'),{status:401});
      if(req.method!=='GET' && (req.headers.origin!==origin || req.headers['x-tonalli-csrf']!==session.csrf))throw denied();
      if(url.pathname==='/api/chat/logout' && req.method==='POST'){sessions.delete(session.token);res.setHeader('Set-Cookie','tonalli_chat=; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=0');return json(res,200,{ok:true});}
      if(url.pathname==='/api/chat/admin/stop'&&req.method==='POST'&&session.user.role==='admin'){
        json(res,200,{stopped:true});setImmediate(()=>{stop().then(()=>process.exit(0));});return;
      }
      if(url.pathname==='/api/chat/admin/invite'&&req.method==='POST'&&session.user.role==='admin'){
        const value=await body(req),user=userById(value.user);if(user?.role!=='participant'||!user.read.length)throw invalid();
        for(const [key,invite]of invites)if(invite.expires<Date.now())invites.delete(key);
        if(invites.size>=64)throw Object.assign(Error('Too many pending invitations'),{status:409});
        const token=csrf(),expires=Date.now()+5*60000;invites.set(token,{user:user.id,expires});
        return json(res,200,{url:`${origin}/#join=${token}`,expires});
      }
      if(url.pathname==='/api/chat/rooms' && req.method==='GET')return json(res,200,{rooms:config.rooms.filter(r=>session.user.read.includes(r.id)).map(r=>({...r,canSend:session.user.send.includes(r.id),canBridge:session.user.bridge.includes(r.id)})),mode:config.mode});
      const match=/^\/api\/chat\/rooms\/([a-zA-Z0-9_-]{1,64})$/.exec(url.pathname);
      if(match){
        const room=match[1];if(!session.user.read.includes(room))throw denied();
        if(req.method==='GET')return json(res,200,{messages:await history.request({op:'list',room}),radio_routes:routes(room)});
        if(req.method==='POST')return json(res,200,{message:await submit(session.user,room,await body(req))});
      }
      const routeMatch=/^\/api\/chat\/rooms\/([a-zA-Z0-9_-]{1,64})\/bridge$/.exec(url.pathname);
      if(routeMatch&&req.method==='POST'){
        const room=routeMatch[1],user=session.user;
        if(user.role!=='gateway'||!user.bridge.includes(room)||!user.read.includes(room)||!user.send.includes(room))throw denied();
        const value=await body(req),key=`${user.id}:${room}`;
        if(value.active===false){radioRoutes.delete(key);return json(res,200,{ok:true});}
        if(!Number.isInteger(value.channel)||value.channel<0||value.channel>7||typeof value.name!=='string'||value.name.length>64||/[\u0000-\u001f]/.test(value.name)||typeof value.to_radio!=='boolean'||typeof value.to_broker!=='boolean')throw invalid();
        if(radioRoutes.size>=128&&!radioRoutes.has(key))throw Object.assign(Error('Too many bridges'),{status:409});
        radioRoutes.set(key,{room,gateway:user.id,gateway_name:user.name,channel:value.channel,name:value.name,to_radio:value.to_radio,to_broker:value.to_broker,expires:Date.now()+15000});
        return json(res,200,{ok:true});
      }
      if(url.pathname==='/api/chat/admin' && session.user.role==='admin') {
        if(req.method==='GET')return json(res,200,{rooms:config.rooms,users:config.users.map(safeUser),mode:config.mode,bind:config.bind,port:config.port,networks:config.mode==='local'?localNetworks():[],pending_restart:config.bind!==server.address()?.address,url:origin,certificate:certificate.toString('utf8'),fingerprint:new X509Certificate(certificate).fingerprint256});
        if(req.method==='POST'){
          const value=await body(req);
          const change=changing.then(async()=>{
            const next=structuredClone(config);
            if(value.action==='network'&&config.mode==='local'&&localNetworks().some(n=>n.address===value.bind)){
              if(!new X509Certificate(certificate).checkIP(value.bind))throw Object.assign(Error('The certificate does not cover this address'),{status:400});
              next.bind=value.bind;
            } else if(value.action==='room' && identifier(value.id)) {
              const index=next.rooms.findIndex(r=>r.id===value.id),room={id:value.id,name:value.name};
              if(index<0)next.rooms.push(room);else next.rooms[index]=room;
            } else if(value.action==='user' && identifier(value.id)){
              const index=next.users.findIndex(u=>u.id===value.id),previous=next.users[index];
              if(value.password!==undefined && (typeof value.password!=='string'||value.password.length<16||value.password.length>256))throw invalid();
              const password=value.password?await passwordHash(value.password):previous?.passwordHash;
              const user={id:value.id,name:value.name,role:value.role,passwordHash:password,read:value.read,send:value.send,bridge:value.bridge};
              if(index<0)next.users.push(user);else next.users[index]=user;
            } else throw invalid();
            await saveConfig(directory,next);config=next;
            // A changed credential or grant revokes existing sessions and MQTT connections.
            for(const [token,s] of sessions)if(s.user===value.id && value.action==='user')sessions.delete(token);
            for(const [token,invite]of invites)if(invite.user===value.id&&value.action==='user')invites.delete(token);
            for(const client of Object.values(broker.clients))if(client.tonalliUser===value.id && value.action==='user')client.close();
          });changing=change.catch(()=>{});await change;return json(res,200,{ok:true});
        }
      }
      return json(res,404,{error:'Unavailable'});
    }
    if(req.method!=='GET' && req.method!=='HEAD')throw invalid();
    let relative=decodeURIComponent(url.pathname).replace(/^\//,'');
    if(!relative || !path.extname(relative))relative='index.html';
    const file=path.resolve(staticRoot,relative);
    if(!file.startsWith(staticRoot+path.sep))throw Object.assign(Error('Not found'),{status:404});
    let actual;try{if(!(await stat(file)).isFile())throw Error('Not a file');actual=await realpath(file);}catch{throw Object.assign(Error('Not found'),{status:404});}
    if(!actual.startsWith(publicRoot+path.sep))throw Object.assign(Error('Not found'),{status:404});
    const data=await readFile(actual);res.writeHead(200,{'Content-Type':mime[path.extname(file)]||'application/octet-stream','Cache-Control':'no-cache'});res.end(req.method==='HEAD'?undefined:data);
  }catch(error){json(res,error.status||503,{error:error.status?error.message:'Chat unavailable'});}
}
const server=https.createServer({key:await readFile(path.join(directory,'tls-key.pem')),cert:certificate,minVersion:'TLSv1.2'},handler);
server.maxConnections=160;server.requestTimeout=15000;server.headersTimeout=10000;
const websocket=new WebSocketServer({noServer:true,maxPayload:8192,perMessageDeflate:false});
server.on('upgrade',(req,socket,head)=>{
  if(req.url!=='/mqtt' || (req.headers.origin && req.headers.origin!==`https://${req.headers.host}`) || broker.connectedClients>=128){socket.destroy();return;}
  websocket.handleUpgrade(req,socket,head,ws=>{
    const stream=createWebSocketStream(ws);broker.handle(stream,req);
    // Aedes 1.2.0 exposes no packet-size option. Bound its pinned library parser's buffer without parsing MQTT ourselves.
    const parser=stream.client._parser,parse=parser.parse.bind(parser);
    parser.parse=bytes=>{if(parser._list.length+bytes.length>16384){stream.destroy();return 0;}return parse(bytes);};
  });
});
let stopping=false;
async function stop(){if(stopping)return;stopping=true;server.close();for(const ws of websocket.clients)ws.terminate();await new Promise(resolve=>broker.close(resolve));await sending;await changing;await history.close();}
process.once('SIGINT',()=>{stop().then(()=>process.exit(0));});process.once('SIGTERM',()=>{stop().then(()=>process.exit(0));});
await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(config.port,config.bind,resolve);});
console.log(`Tonalli Chat listening on ${config.bind}:${config.port} (${config.mode}); certificate SHA-256 ${new X509Certificate(certificate).fingerprint256}`);
