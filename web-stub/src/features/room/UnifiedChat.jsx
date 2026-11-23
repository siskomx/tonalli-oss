import React,{useEffect,useState} from 'react';
import {useApp,useRouteReadiness} from '../../app/context.js';
import LocalRoom from './Room.jsx';
import BrokerChat from './BrokerChat.jsx';
import ChatConnections,{connectionRequest} from './ChatConnections.jsx';
import RadioChat from './RadioChat.jsx';
export default function UnifiedChat(){
  const {csrf,language,privateDevice,canUseMesh}=useApp(),es=language==='es';const {markReady,markPending}=useRouteReadiness('room');
  const [destination,setDestination]=useState(()=>canUseMesh&&new URLSearchParams(window.location.search).get('destination')==='radio'?'radio':'local'),[connection,setConnection]=useState(null);
  useEffect(()=>{let active=true;if(privateDevice)connectionRequest(csrf,'').then(v=>{if(active)setConnection(v);}).catch(()=>{});return()=>{active=false;};},[csrf,privateDevice]);
  const ready=details=>void markReady({feature:'room',...details});
  async function disconnect(){await connectionRequest(csrf,'/disconnect',{});setConnection({connected:false});setDestination('local');}
  return <section className="space-y-4">{privateDevice&&<label className="block">{es?'Destino':'Destination'}<select className="block border rounded p-2" value={destination} onChange={e=>{markPending();setDestination(e.target.value);}}><option value="local">{es?'Sala local':'Local room'}</option>{canUseMesh&&<option value="radio">{es?'Radio':'Radio'}</option>}<option value="broker">{es?'Servidor de Chat':'Chat broker'}</option></select></label>}
    {destination==='local'?<LocalRoom/>:destination==='radio'?<RadioChat onReady={ready}/>:connection?.connected?<BrokerChat key={`${connection.url}:${connection.user?.id}:${connection.epoch}`} language={language} initialSession={{csrf,chat_user:connection.user}} endpoint="/api/connections/chat/proxy" onDisconnect={disconnect} onReady={ready}/>:<ChatConnections changed={setConnection} onReady={ready}/>}
  </section>;
}
