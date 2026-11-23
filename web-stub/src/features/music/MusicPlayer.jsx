import React,{createContext,useContext,useEffect,useRef,useState} from 'react';
import {Link} from 'react-router-dom';
import {useQuery,useQueryClient} from '@tanstack/react-query';
import {useApp} from '../../app/context.js';
import {playbackStart} from './playback.js';

const MusicContext=createContext(null);
export const useMusic=()=>useContext(MusicContext);
export const clock=value=>`${Math.floor((value||0)/60)}:${String(Math.floor((value||0)%60)).padStart(2,'0')}`;
export default function MusicPlayer({children}){
  const {csrf,canReadProgress,t,beginNavigation}=useApp(),cache=useQueryClient();
  const [error,setError]=useState(false),[busy,setBusy]=useState(false),[status,setStatus]=useState({});
  const [videoActive,setVideoActive]=useState(false);
  const audio=useRef(null),latest=useRef(null),wantPlay=useRef(false),tail=useRef(Promise.resolve()),seekGeneration=useRef(null);
  const key=['music',csrf];
  async function request(path='',body,signal){
    const response=await fetch(`/api/music${path}`,{method:body?'POST':'GET',signal,credentials:'same-origin',cache:'no-store',
      headers:{'X-Tonalli-Csrf':csrf,...(body?{'Content-Type':'application/json'}:{})},...(body?{body:JSON.stringify(body)}:{})});
    if(!response.ok)throw Error('music_unavailable');return response.json();
  }
  const music=useQuery({queryKey:key,enabled:!!csrf&&canReadProgress,staleTime:0,refetchOnMount:'always',queryFn:({signal})=>request('',undefined,signal)});
  latest.current=music.data;
  const files=useQuery({queryKey:['managed-media',csrf],enabled:!!csrf&&canReadProgress,staleTime:0,refetchOnMount:'always',queryFn:async({signal})=>{
    const response=await fetch('/api/files',{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});
    if(!response.ok)throw Error('files_unavailable');return (await response.json()).files;
  }});
  function update(state){const value={...latest.current,state};latest.current=value;cache.setQueryData(key,value);}
  function run(operation){
    const next=tail.current.catch(()=>{}).then(async()=>{setBusy(true);setError(false);try{return await operation();}catch{setError(true);await music.refetch();}finally{setBusy(false);}});
    tail.current=next;return next;
  }
  async function mutate(command){const result=await request('',command);if(command.action==='seek')seekGeneration.current=result.state.queue.generation;update(result.state);return result;}
  function command(action,fields={}){return run(()=>mutate({action,revision:latest.current.state.revision,...fields}));}
  function step(delta){return run(()=>mutate({action:'step',generation:latest.current.state.queue.generation,delta}));}
  function seek(seconds){return run(()=>mutate({action:'seek',generation:latest.current.state.queue.generation,seconds}));}
  function play(action='play'){return run(async()=>{
    if(action==='play')setVideoActive(false);
    if(latest.current.native){await request('/playback',{action});return;}
    const element=audio.current;
    if(action==='play'){
      wantPlay.current=true;
      if(latest.current.state.queue.current===null){await mutate({action:'step',generation:latest.current.state.queue.generation,delta:1});return;}
      if(element.dataset.generation===String(latest.current.state.queue.generation)&&element.readyState>0)await element.play();
    }else{wantPlay.current=false;element.pause();await savePosition(latest.current.state.queue.generation,element.currentTime);}
  });}
  async function savePosition(generation,seconds){
    if(!Number.isFinite(seconds))return;
    try{await request('',{action:'position',generation,seconds});cache.invalidateQueries({queryKey:['reading',csrf]});}catch{setError(true);}
  }
  const queue=music.data?.state.queue,id=queue?.tracks[queue.current],generation=queue?.generation;
  const position=music.data?.native&&(status.id!==id||status.generation!==generation)?(queue?.seconds||0):(status.seconds??queue?.seconds??0);
  const duration=music.data?.native&&(status.id!==id||status.generation!==generation)?0:status.duration;
  useEffect(()=>{
    if(!music.data?.native)return;
    let closed=false,controller=null,timer;
    async function poll(){controller=new AbortController();try{
      const value=await request('/playback',undefined,controller.signal);
      if(closed)return;setStatus(value);
      if(value.generation>latest.current.state.queue.generation)await music.refetch();
    }catch{if(!closed)setError(true);}finally{if(!closed)timer=setTimeout(poll,1000);}}
    void poll();return()=>{closed=true;controller?.abort();clearTimeout(timer);};
  },[music.data?.native,csrf]);
  useEffect(()=>{
    const element=audio.current;
    if(!element||music.data?.native||!id)return;
    let closed=false,ready=false,lastSaved=0;
    const initial=latest.current.state.queue.seconds;
    const seeking=seekGeneration.current===generation;
    const controller=new AbortController();
    function report(){if(closed)return;setStatus({playing:!element.paused,seconds:element.currentTime,duration:Number.isFinite(element.duration)?element.duration:0,loading:!ready});}
    function loaded(){if(closed)return;ready=true;element.currentTime=playbackStart(initial,element.duration,seeking);if(wantPlay.current)element.play().catch(()=>setError(true));report();}
    function progress(){report();if(ready&&performance.now()-lastSaved>5000){lastSaved=performance.now();void savePosition(generation,element.currentTime);}}
    function paused(){report();if(ready)void savePosition(generation,element.currentTime);}
    function ended(){if(closed)return;wantPlay.current=true;void run(async()=>{await savePosition(generation,element.currentTime);await mutate({action:'step',generation,delta:1});});}
    function failed(){if(!closed){wantPlay.current=false;setError(true);report();}}
    for(const [event,handler] of [['loadedmetadata',loaded],['timeupdate',progress],['play',report],['pause',paused],['ended',ended],['error',failed]])element.addEventListener(event,handler);
    request('/stream',{id},controller.signal).then(value=>{if(!closed){element.dataset.generation=String(generation);element.src=value.url;element.load();}}).catch(()=>{if(!closed)setError(true);});
    return()=>{closed=true;controller.abort();for(const [event,handler] of [['loadedmetadata',loaded],['timeupdate',progress],['play',report],['pause',paused],['ended',ended],['error',failed]])element.removeEventListener(event,handler);if(ready)void savePosition(generation,element.currentTime);element.pause();element.removeAttribute('src');element.load();};
  },[id,generation,music.data?.native,csrf]);
  useEffect(()=>{if(id)return;wantPlay.current=false;audio.current?.pause();setStatus({playing:false,seconds:0,duration:0});},[id]);
  useEffect(()=>{if(status.playing)setVideoActive(false);},[status.playing]);
  useEffect(()=>{
    if(music.data?.native||videoActive||!('mediaSession' in navigator)||!id)return;
    const controls={play:()=>void play(),pause:()=>void play('pause'),previoustrack:()=>void step(-1),nexttrack:()=>void step(1),seekto:event=>void seek(event.seekTime)};
    const file=files.data?.find(file=>file.id===id);
    if(globalThis.MediaMetadata)navigator.mediaSession.metadata=new MediaMetadata({title:file?.name||t('nav.music')});
    for(const [action,handler] of Object.entries(controls)){try{navigator.mediaSession.setActionHandler(action,handler);}catch{}}
    return()=>{for(const action of Object.keys(controls)){try{navigator.mediaSession.setActionHandler(action,null);}catch{}}};
  },[id,music.data?.native,files.data,videoActive]);
  const value={state:music.data?.state,native:music.data?.native,files:files.data||[],status,busy,videoActive,setVideoActive,error:error||!!music.error||!!files.error,command,step,seek,play};
  return <MusicContext.Provider value={value}>
    <audio ref={audio} preload="metadata" data-testid="music-audio"/>
    {canReadProgress&&id&&<section className="app-card mb-4 space-y-2" aria-label={t('music.player')} data-testid="music-player">
      <Link className="underline break-words" to="/music" onClick={event=>beginNavigation('/music',event)}>{files.data?.find(file=>file.id===id)?.name||t('nav.music')}</Link>
      <div className="flex flex-wrap gap-2 items-center"><button className="secondary-button" disabled={busy} onClick={()=>void step(-1)}>{t('music.previous')}</button>
        <button className="primary-button" disabled={busy||status.loading} onClick={()=>void play(status.playing?'pause':'play')}>{t(status.playing?'music.pause':'music.play')}</button>
        <button className="secondary-button" disabled={busy} onClick={()=>void step(1)}>{t('music.next')}</button>
        {music.data?.native&&<button className="secondary-button" disabled={busy} onClick={()=>void play('stop')}>{t('music.stop')}</button>}
        <span>{clock(position)} / {clock(duration)}</span></div>
      <label className="block">{t('music.seek')}<input className="w-full" aria-label={t('music.seek')} type="range" min="0" max={duration||0} step="1" value={Math.min(position,duration||0)} disabled={busy||!duration} onChange={event=>void seek(Number(event.target.value))}/></label>
      {(error||status.error)&&<p role="alert">{t('music.failed')}</p>}
    </section>}
    {children}
  </MusicContext.Provider>;
}
