import React,{useEffect,useState} from 'react';
import {useApp} from '../../app/context.js';
import {Link} from 'react-router-dom';
import {parseConnectionCode} from './connection-code.js';
const ChatBridge=React.lazy(()=>import('./ChatBridge.jsx'));
export async function connectionRequest(csrf,route,body) {
  const response=await fetch('/api/connections/chat'+route,{method:body===undefined?'GET':'POST',credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf,'Content-Type':'application/json'},...(body===undefined?{}:{body:JSON.stringify(body)})});
  const value=await response.json().catch(()=>({}));if(!response.ok)throw Error(value.error||'Chat connection unavailable');return value;
}
export default function ChatConnections({changed,onReady}) {
  const {csrf,language,privateDevice,canUseMesh}=useApp(),es=language==='es';
  const [status,setStatus]=useState(null),[url,setUrl]=useState(''),[user,setUser]=useState(''),[password,setPassword]=useState(''),[certificate,setCertificate]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false);
  const [remember,setRemember]=useState(false);
  const [setup,setSetup]=useState('');
  function importSetup(){try{const value=parseConnectionCode(setup.trim());setUrl(value.url);setUser(value.user);setPassword(value.password);setCertificate(value.certificate);setSetup('');setError('');}catch(e){setError(e.message);}}
  useEffect(()=>{let active=true;if(privateDevice)connectionRequest(csrf,'').then(v=>{if(active){setStatus(v);onReady?.({destination:'broker'});}}).catch(e=>{if(active){setError(e.message);onReady?.({destination:'broker',unavailable:true});}});return()=>{active=false;};},[csrf,privateDevice]);
  if(!privateDevice)return null;
  async function perform(route,body){setBusy(true);setError('');try{await connectionRequest(csrf,route,body);setPassword('');const value=await connectionRequest(csrf,'');setStatus(value);changed?.(value);}catch(e){setError(e.message);}finally{setBusy(false);}}
  async function importConnection(event){const file=event.target.files?.[0];if(!file)return;try{if(file.size>20000)throw Error('Invalid connection file');const value=parseConnectionCode(await file.text());setUrl(value.url);setUser(value.user);setPassword(value.password);setCertificate(value.certificate);}catch(e){setError(e.message);}event.target.value='';}
  return <section className="app-card space-y-3"><h3>{es?'Chat y conexiones':'Chat & Connections'}</h3><p>{es?'Conecta un servidor local o de internet. La sala local y la radio funcionan sin este servidor.':'Connect a local or internet broker. The local room and radio work independently of this connection.'}</p>{error&&<p role="alert">{error}</p>}
    {canUseMesh&&<Link className="secondary-button" to="/room?destination=radio">{es?'Radio y canales':'Radio & channels'}</Link>}
    {status?.storage_error&&<p role="alert">{status.storage_error}</p>}
    {status?.saved&&<div className="space-y-2"><p>{es?'Conexión guardada':'Saved connection'} · {status.saved.url} · {status.saved.user}</p>{!status.connected&&<button className="secondary-button" disabled={busy} onClick={()=>void perform('/connect',{saved:true})}>{es?'Conectar guardada':'Connect saved connection'}</button>}<button className="secondary-button" disabled={busy} onClick={()=>void perform('/forget',{})}>{es?'Olvidar datos de acceso':'Forget saved credentials'}</button></div>}
    {status?.connected?<><p role="status">{es?'Conectado':'Connected'} · {status.url} · {status.user?.name}</p><button className="secondary-button" disabled={busy} onClick={()=>void perform('/disconnect',{})}>{es?'Desconectar':'Disconnect'}</button></>:<form className="space-y-3" onSubmit={e=>{e.preventDefault();void perform('/connect',{url,user,password,certificate:certificate||null,remember});}}>
      <label className="block">{es?'Pegar configuración del propietario':'Paste setup from the owner'}<textarea className="block w-full border p-2" value={setup} maxLength={30000} onChange={e=>setSetup(e.target.value)}/></label><button type="button" className="secondary-button" disabled={!setup.trim()||busy} onClick={importSetup}>{es?'Importar conexión':'Import connection'}</button>
      {!/Android/i.test(navigator.userAgent)&&<label className="block">{es?'Importar archivo de conexión':'Import connection file'}<input className="block" type="file" accept="application/json,.json" onChange={e=>void importConnection(e)}/></label>}
      <label className="block">{es?'Dirección segura del servidor':'Secure broker address'}<input className="block w-full border rounded p-2" type="url" required placeholder="https://…" value={url} onChange={e=>setUrl(e.target.value)}/></label><label className="block">{es?'Usuario':'User'}<input required maxLength={64} className="block border rounded p-2" autoComplete="username" value={user} onChange={e=>setUser(e.target.value)}/></label><label className="block">{es?'Contraseña':'Password'}<input required maxLength={256} className="block border rounded p-2" type="password" autoComplete="current-password" value={password} onChange={e=>setPassword(e.target.value)}/></label>
      <details><summary>{es?'Certificado privado del propietario':'Owner’s private certificate'}</summary><p>{es?'Importa el certificado que te dio el propietario. Solo se confía en él para esta conexión.':'Import the certificate supplied by the owner. Trust applies only to this connection.'}</p><textarea className="block w-full border p-2" aria-label={es?'Certificado':'Certificate'} value={certificate} maxLength={16384} onChange={e=>setCertificate(e.target.value)}/></details>
      <label className="block"><input type="checkbox" checked={remember} disabled={Boolean(status?.storage_error)} onChange={e=>setRemember(e.target.checked)}/>{es?'Recordar esta conexión de forma cifrada en este dispositivo':'Remember this connection encrypted on this device'}</label><p>{es?'No se conecta ni envía mensajes automáticamente al reiniciar.':'Restart never connects or forwards messages automatically.'}</p><button className="chat-control" disabled={busy}>{es?'Conectar y comprobar':'Connect and check'}</button>{(busy||status?.connecting)&&<button type="button" className="secondary-button" onClick={()=>void perform('/disconnect',{})}>{es?'Detener':'Stop'}</button>}
    </form>}
    {canUseMesh&&status?.connected&&status.user?.role==='gateway'&&<React.Suspense fallback={<p role="status">…</p>}><ChatBridge/></React.Suspense>}
  </section>;
}
