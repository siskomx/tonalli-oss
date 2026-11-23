import React,{useEffect,useRef,useState} from 'react';
import Plyr from 'plyr';
import 'plyr/dist/plyr.css';
import './player.css';
import {useQueryClient} from '@tanstack/react-query';
import {useApp} from '../../app/context.js';
import {useMusic} from '../music/MusicPlayer.jsx';
import {playbackStart} from '../music/playback.js';
import {readingRequest} from '../library/reading-client.js';
import {videoRequest} from './client.js';

export default function Player({file,poster,onReady,sharedSource}){
  const {csrf,t}=useApp(),music=useMusic(),cache=useQueryClient();
  const host=useRef(null),player=useRef(null),callbacks=useRef({music,onReady});callbacks.current={music,onReady};
  const [failed,setFailed]=useState(false),[saveFailed,setSaveFailed]=useState(false),[loading,setLoading]=useState(true);
  useEffect(()=>{
    const element=document.createElement('video');element.controls=true;element.playsInline=true;element.preload='metadata';
    host.current.append(element);let closed=false,ready=false,initial=0,token=null,sequence=0,lastSaved=0;
    const controller=new AbortController();
    const instance=new Plyr(element,{title:file.name,autoplay:false,storage:{enabled:false},iconUrl:'/assets/player/plyr.svg',
      blankVideo:'/assets/player/blank.mp4',fullscreen:{enabled:true,fallback:'force'},settings:['speed'],
      controls:['play-large','play','progress','current-time','duration','mute','volume','captions','settings','fullscreen'],
      i18n:Object.fromEntries(['play','pause','restart','rewind','fastForward','seek','seekLabel','played','buffered','currentTime','duration','volume','mute','unmute','enableCaptions','disableCaptions','captions','settings','enterFullscreen','exitFullscreen','speed','normal','menuBack'].map(key=>[key,t(`player.${key}`)]))});
    player.current=instance;
    function complete(){if(closed)return;ready=true;setLoading(false);callbacks.current.onReady();}
    function failure(kind){if(closed)return;setLoading(false);setFailed(kind);callbacks.current.onReady();}
    function error(){failure('unsupported');}
    function save(){
      if(!ready||!token||!Number.isFinite(element.currentTime))return;
      void videoRequest(csrf,'progress',{token,sequence:++sequence,seconds:Math.max(0,element.currentTime)}).then(value=>{
        if(value.accepted){if(!closed)setSaveFailed(false);cache.invalidateQueries({queryKey:['reading',csrf]});}
      }).catch(()=>{if(!closed)setSaveFailed(true);});
    }
    function metadata(){const value=playbackStart(initial,element.duration,false);if(value>0)element.currentTime=value;else complete();}
    function seeked(){if(!ready)complete();save();}
    function progress(){if(ready&&!element.seeking&&performance.now()-lastSaved>=5000){lastSaved=performance.now();save();}}
    function play(){callbacks.current.music.setVideoActive(true);if(callbacks.current.music.state)void callbacks.current.music.play('pause');}
    const listeners=[['loadedmetadata',metadata],['seeked',seeked],['timeupdate',progress],['pause',save],['ended',save],['play',play],['error',error]];
    for(const [event,handler] of listeners)element.addEventListener(event,handler);
    function leaving(){save();}
    window.addEventListener('pagehide',leaving);
    async function open(){
      if(sharedSource){element.src=sharedSource;element.load();return;}
      const grant=await videoRequest(csrf,'stream',{id:file.id},controller.signal);
      if(closed)return;token=grant.url.split('/').at(-1);
      const reading=await readingRequest(csrf,'GET',undefined,'',controller.signal).catch(problem=>{
        if(problem.name==='AbortError')throw problem;
        if(!closed)setSaveFailed(true);return {entries:[]};
      });
      if(closed)return;
      const entry=reading.entries.find(entry=>entry.destination.kind==='media'&&entry.destination.id===file.id&&entry.position.kind==='time');
      initial=entry?.position.seconds||0;
      element.src=grant.url;element.load();
      if(poster)videoRequest(csrf,'poster',{id:poster},controller.signal).then(value=>{if(!closed)element.poster=value.url;}).catch(()=>{});
    }
    void open().catch(problem=>{if(!closed)failure(problem.message==='video_unsupported'?'unsupported':'unavailable');});
    return()=>{
      save();closed=true;controller.abort();window.removeEventListener('pagehide',leaving);
      callbacks.current.music.setVideoActive(false);
      for(const [event,handler] of listeners)element.removeEventListener(event,handler);
      element.pause();player.current=null;instance.destroy();host.current?.replaceChildren();
    };
  },[file.id,file.version,csrf,sharedSource]);
  useEffect(()=>{if(music.status.playing)player.current?.pause();},[music.status.playing]);
  useEffect(()=>{
    if(!music.videoActive||!('mediaSession' in navigator))return;
    const actions={play:()=>void player.current?.play().catch(()=>setFailed('unsupported')),pause:()=>player.current?.pause(),seekto:event=>{if(player.current)player.current.currentTime=event.seekTime;}};
    if(globalThis.MediaMetadata)navigator.mediaSession.metadata=new MediaMetadata({title:file.name});
    for(const [action,handler] of Object.entries(actions)){try{navigator.mediaSession.setActionHandler(action,handler);}catch{}}
    return()=>{for(const action of Object.keys(actions)){try{navigator.mediaSession.setActionHandler(action,null);}catch{}}};
  },[music.videoActive,file.name]);
  return <section className="app-card space-y-3" data-testid="video-player">
    <h3 className="break-words">{file.name}</h3><div ref={host}/>
    {loading&&<p role="status">{t('app.loading')}</p>}
    {failed&&<p role="alert">{t(failed==='unsupported'?'video.unsupported':'video.failed')}</p>}
    {saveFailed&&<p role="alert">{t('video.saveFailed')}</p>}
  </section>;
}
