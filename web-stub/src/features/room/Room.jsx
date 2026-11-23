import React,{useEffect,useRef,useState} from 'react';
import {Link} from 'react-router-dom';
import {useApp,useRouteReadiness} from '../../app/context.js';
import ChatMessageBubble from '../chat/ChatMessageBubble.jsx';
import {mergeRoom} from './client.js';

const newView=()=>crypto.randomUUID().replaceAll('-','');
export default function Room(){const[view,setView]=useState(newView);return <RoomSession key={view} view={view} restart={()=>setView(newView())}/>;}
function RoomSession({view,restart}){
  const{csrf,t,privateDevice,beginNavigation}=useApp(),route=useRouteReadiness('room'),callbacks=useRef(route);callbacks.current=route;
  const[data,setData]=useState(null),[draft,setDraft]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const current=useRef(null),alive=useRef(false),generation=useRef(0),lastEpoch=useRef(''),queue=useRef(Promise.resolve()),controller=useRef(null),lease=useRef(false),scroll=useRef(null),bottom=useRef(true);
  function apply(value){const next=mergeRoom(current.current,value);if(next.epoch){if(lastEpoch.current&&lastEpoch.current!==next.epoch&&alive.current)setDraft('');lastEpoch.current=next.epoch;}current.current=next;if(alive.current){setData(next);void callbacks.current.markReady({feature:'room',active:next.active,joined:next.joined});}return next;}
  function request(action='get',text=''){
    const ticket=generation.current;
    const work=queue.current.then(async()=>{
      if(!alive.current||generation.current!==ticket)throw new DOMException('Closed','AbortError');
      const state=current.current,body={action,view,epoch:state?.epoch||'',after:state?.latest||0,text};if(action==='join')lease.current=true;
      const response=await fetch('/api/room'+(action==='get'?`?${new URLSearchParams({view,epoch:body.epoch,after:String(body.after)})}`:''),{
        method:action==='get'?'GET':'POST',signal:controller.current.signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf,...(action==='get'?{}:{'Content-Type':'application/json'})},...(action==='get'?{}:{body:JSON.stringify(body)})});
      if(!alive.current||generation.current!==ticket)throw new DOMException('Closed','AbortError');
      if(!response.ok){const value=await response.json().catch(()=>({}));if(!alive.current||generation.current!==ticket)throw new DOMException('Closed','AbortError');const code=value.error|| (response.status===429?'room_rate':'room_unavailable');
        if(!['room_rate','room_full','room_invalid'].includes(code))apply({active:false,joined:false,epoch:'',self_id:'',latest:0,messages:[],members:[]});throw Error(code);
      }
      const value=await response.json();if(!alive.current||generation.current!==ticket)throw new DOMException('Closed','AbortError');const next=apply(value);setError('');return next;
    });queue.current=work.catch(()=>{});return work;
  }
  function failed(failure){if(alive.current&&failure.name!=='AbortError'){setError(['room_rate','room_full','room_changed','room_invalid','room_join_first'].includes(failure.message)?failure.message:'room_unavailable');void callbacks.current.markReady({feature:'room',unavailable:true});}}
  useEffect(()=>{
    const ticket=++generation.current;alive.current=true;controller.current=new AbortController();current.current=null;setData(null);setDraft('');let timer,polling=false;
    async function poll(){if(polling||document.hidden||!alive.current)return;polling=true;clearTimeout(timer);try{await request();}catch(failure){if(failure.name!=='AbortError'&&alive.current&&generation.current===ticket){apply({active:false,joined:false,epoch:'',self_id:'',latest:0,messages:[],members:[]});failed(failure);}}finally{polling=false;if(alive.current&&generation.current===ticket&&!document.hidden)timer=setTimeout(poll,2000);}}
    function visibility(){clearTimeout(timer);if(!document.hidden)void poll();}
    function leave(){if(!lease.current)return;lease.current=false;const epoch=current.current?.epoch||'';if(!epoch)return;void fetch('/api/room',{method:'POST',keepalive:true,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf,'Content-Type':'application/json'},body:JSON.stringify({action:'leave',view,epoch,text:'',after:0})}).catch(()=>{});}
    void poll();document.addEventListener('visibilitychange',visibility);window.addEventListener('pagehide',leave);
    return()=>{alive.current=false;generation.current++;clearTimeout(timer);controller.current.abort();leave();document.removeEventListener('visibilitychange',visibility);window.removeEventListener('pagehide',leave);};
  },[csrf,view]);
  useEffect(()=>{if(scroll.current&&bottom.current)scroll.current.scrollTop=scroll.current.scrollHeight;},[data?.latest]);
  async function perform(action){const ticket=generation.current;setBusy(true);setError('');const sent=draft;try{await request(action,action==='send'?sent:'');if(alive.current&&generation.current===ticket&&action==='send')setDraft(value=>value===sent?'':value);}catch(failure){if(generation.current===ticket)failed(failure);}finally{if(alive.current&&generation.current===ticket){setBusy(false);if(action==='leave')restart();}}}
  const count=Array.from(draft).length;
  return <section className="space-y-5" data-testid="room-page"><h2>{t('nav.room')}</h2><p>{t('room.help')}</p>
    {error&&<p role="alert">{t(`room.error.${error}`)}</p>}
    {!data&&!error&&<p role="status">{t('app.loading')}</p>}
    {data&&!data.active&&<div className="app-card space-y-3"><p>{t(privateDevice?'room.off':'room.disconnected')}</p>{privateDevice?<Link className="secondary-button" to="/settings?tab=sharing" onClick={event=>beginNavigation('/settings?tab=sharing',event)}>{t('room.settings')}</Link>:<a className="secondary-button" href="/pair">{t('room.pair')}</a>}</div>}
    {data?.active&&!data.joined&&<button className="chat-control" disabled={busy} onClick={()=>void perform('join')}>{t('room.join')}</button>}
    {data?.joined&&<>
      <div className="flex flex-wrap gap-3 items-center"><p role="status">{t('room.joined')}</p><button className="secondary-button" disabled={busy} onClick={()=>void perform('leave')}>{t('room.leave')}</button></div>
      <section className="app-card"><h3>{t('room.members')}</h3><ul className="flex flex-wrap gap-3">{data.members.map(member=><li key={member.id}><bdi>{member.label}</bdi>{member.is_host&&<strong> · {t('room.host')}</strong>}</li>)}</ul></section>
      <div ref={scroll} className="app-card space-y-3 h-[45dvh] min-h-64 overflow-y-auto" role="log" aria-label={t('room.messages')} aria-live="polite" onScroll={event=>{const element=event.currentTarget;bottom.current=element.scrollHeight-element.scrollTop-element.clientHeight<80;}} data-testid="room-messages">
        {!data.messages.length&&<p>{t('room.empty')}</p>}{data.messages.map(message=><article key={message.id} className={`flex flex-col ${message.sender.id===data.self_id?'items-end':'items-start'}`} data-testid="room-message"><p className="text-sm"><bdi>{message.sender.label}</bdi>{message.sender.is_host&&<strong> · {t('room.host')}</strong>}</p><ChatMessageBubble t={t} message={{role:message.sender.id===data.self_id?'user':'peer',content:message.text,timestamp:new Date(message.created_ms)}}/></article>)}
      </div>
      <form className="space-y-3" onSubmit={event=>{event.preventDefault();if(!busy&&draft.trim()&&count<=1000)void perform('send');}}>
        <label className="block" htmlFor="room-draft">{t('room.message')}</label><textarea id="room-draft" className="w-full min-h-24 rounded border p-3" maxLength={2000} value={draft} disabled={busy} aria-describedby="room-length" onChange={event=>setDraft(event.target.value)} onKeyDown={event=>{if(event.key==='Enter'&&!event.shiftKey&&!event.nativeEvent.isComposing){event.preventDefault();event.currentTarget.form.requestSubmit();}}}/>
        <p id="room-length">{t('room.characters',{count})}</p><button className="chat-control" disabled={busy||!draft.trim()||count>1000}>{t('room.send')}</button>
      </form>
    </>}
  </section>;
}
