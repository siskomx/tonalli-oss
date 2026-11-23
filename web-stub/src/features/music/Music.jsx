import React,{useEffect,useRef,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {useApp} from '../../app/context.js';
import Input from '../../ui/Input.jsx';
import {mediaCategory,searchMedia} from '../library/media-catalog.js';
import {useMusic} from './MusicPlayer.jsx';

export default function Music(){
  const {t,bindRoute,beginNavigation}=useApp(),music=useMusic(),[params]=useSearchParams();
  const [query,setQuery]=useState(''),[page,setPage]=useState(0),[selected,setSelected]=useState(''),[label,setLabel]=useState(''),[addition,setAddition]=useState('');
  const [removing,setRemoving]=useState(null),confirmation=useRef(null);
  useEffect(()=>{if(removing&&!confirmation.current?.open)confirmation.current?.showModal();else if(!removing)confirmation.current?.close();},[removing]);
  const {state,files,busy,error,command}=music,route=React.useMemo(()=>bindRoute('music'),[bindRoute]);
  const requested=params.get('track');
  useEffect(()=>{if(state)void route.markReady({feature:'music'});else if(error)route.markError(Error('music_unavailable'));},[state,error,route]);
  useEffect(()=>{if(!requested||!state||state.queue.tracks[state.queue.current]===requested)return;
    if(files.some(file=>file.id===requested&&mediaCategory(file.name)==='music'))void command('queue',{tracks:[requested],current:0});
  },[requested,!!state,files]);
  const playlist=state?.playlists.find(row=>row.id===selected),tracks=searchMedia(files,query,'music');
  useEffect(()=>{setLabel(playlist?.name||'');},[playlist?.id,playlist?.name]);
  function name(id){return files.find(file=>file.id===id)?.name||t('music.missing');}
  async function edit(items){await command('replace_playlist',{id:playlist.id,name:label||playlist.name,items});}
  function move(items,index,delta){const next=[...items];[next[index],next[index+delta]]=[next[index+delta],next[index]];return next;}
  function changeQueue(next,current){void command('queue',{tracks:next,current:next.length?current:null});}
  function reorder(index,delta){const queue=state.queue,next=move(queue.tracks,index,delta);let current=queue.current;
    if(current===index)current=index+delta;else if(current===index+delta)current=index;changeQueue(next,current);}
  function remove(index){const queue=state.queue,next=queue.tracks.filter((_,i)=>i!==index);let current=queue.current;
    if(current>index)current--;else if(current===index)current=Math.min(index,next.length-1);changeQueue(next,Math.max(0,current??0));}
  return <section className="space-y-5" data-testid="music-page"><h2>{t('nav.music')}</h2><p>{t('music.help')}</p>
    {error&&<p role="alert">{t('music.failed')}</p>}{!state?<p role="status">{t('app.loading')}</p>:<>
      <section className="app-card space-y-3"><h3>{t('music.tracks')}</h3>
        <Link className="underline" to="/files" onClick={event=>beginNavigation('/files',event)}>{t('music.import')}</Link>
        <Input name="music-search" label={t('unified.search')} type="search" value={query} onChange={event=>{setQuery(event.target.value);setPage(0);}}/>
        {!tracks.length&&<p>{t('music.empty')}</p>}
        <ul className="space-y-3">{tracks.slice(page*100,page*100+100).map(file=><li key={file.id} className="flex flex-wrap gap-2 items-center"><span className="break-words">{file.name}{file.folder?` · ${file.folder}`:''}</span>
          <button className="primary-button" disabled={busy} onClick={()=>void command('queue',{tracks:[file.id],current:0}).then(result=>{if(result)void music.play();})}>{t('music.play')}</button>
          <button className="secondary-button" disabled={busy} onClick={()=>changeQueue([...state.queue.tracks,file.id],state.queue.current??0)}>{t('music.addQueue')}</button></li>)}</ul>
        {tracks.length>100&&<div className="flex gap-2"><button disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button disabled={(page+1)*100>=tracks.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
      </section>
      <section className="app-card space-y-3"><h3>{t('music.queue')}</h3>{!state.queue.tracks.length&&<p>{t('music.queueEmpty')}</p>}
        <ol className="space-y-2">{state.queue.tracks.map((id,index)=><li key={`${index}:${id}`} className="flex flex-wrap gap-2 items-center" aria-current={index===state.queue.current?'true':undefined}>
          <button className="secondary-button" disabled={busy} onClick={()=>changeQueue(state.queue.tracks,index)}>{index===state.queue.current?'▶ ':''}{name(id)}</button>
          <button className="secondary-button" aria-label={`${t('music.up')} ${name(id)}`} disabled={busy||index===0} onClick={()=>reorder(index,-1)}>↑</button>
          <button className="secondary-button" aria-label={`${t('music.down')} ${name(id)}`} disabled={busy||index===state.queue.tracks.length-1} onClick={()=>reorder(index,1)}>↓</button>
          <button className="secondary-button" disabled={busy} onClick={()=>remove(index)}>{t('music.remove')}</button></li>)}</ol>
      </section>
      <section className="app-card space-y-3"><h3>{t('music.playlists')}</h3>
        <form onSubmit={event=>{event.preventDefault();void command('create_playlist',{name:label});}} className="flex flex-wrap gap-2 items-end">
          <Input name="playlist-name" label={t('music.name')} value={label} maxLength={240} onChange={event=>setLabel(event.target.value)}/><button className="primary-button" disabled={busy||!label.trim()}>{t('music.create')}</button></form>
        <label className="block">{t('music.choose')}<select className="block max-w-full" value={selected} onChange={event=>setSelected(event.target.value)}><option value="">{t('music.choose')}</option>{state.playlists.map(row=><option key={row.id} value={row.id}>{row.name}</option>)}</select></label>
        {playlist&&<>
          <div className="flex flex-wrap gap-2"><button className="primary-button" disabled={busy} onClick={()=>void command('play_playlist',{id:playlist.id}).then(result=>{if(result&&!music.native)void music.play();})}>{t('music.play')}</button>
            <button className="secondary-button" disabled={busy||!label.trim()} onClick={()=>void edit(playlist.items)}>{t('music.rename')}</button>
            <button className="secondary-button" disabled={busy} onClick={()=>setRemoving({id:playlist.id,name:playlist.name})}>{t('music.delete')}</button></div>
          <label className="block">{t('music.addItem')}<select className="block max-w-full" value={addition} onChange={event=>setAddition(event.target.value)}><option value="">{t('music.choose')}</option>
            {files.filter(file=>mediaCategory(file.name)==='music').map(file=><option key={file.id} value={`track:${file.id}`}>{file.name}</option>)}
            {state.playlists.filter(row=>row.id!==playlist.id).map(row=><option key={row.id} value={`playlist:${row.id}`}>{t('music.playlist')}: {row.name}</option>)}</select></label>
          <button className="secondary-button" disabled={busy||!addition} onClick={()=>{const [kind,id]=addition.split(':');void edit([...playlist.items,{kind,id}]);}}>{t('music.addItem')}</button>
          <ol className="space-y-2">{playlist.items.map((item,index)=><li key={index} className="flex flex-wrap gap-2 items-center"><span>{item.kind==='track'?name(item.id):`${t('music.playlist')}: ${state.playlists.find(row=>row.id===item.id)?.name||t('music.missing')}`}</span>
            <button className="secondary-button" aria-label={`${t('music.up')} ${index+1}`} disabled={busy||!index} onClick={()=>void edit(move(playlist.items,index,-1))}>↑</button>
            <button className="secondary-button" aria-label={`${t('music.down')} ${index+1}`} disabled={busy||index===playlist.items.length-1} onClick={()=>void edit(move(playlist.items,index,1))}>↓</button>
            <button className="secondary-button" disabled={busy} onClick={()=>void edit(playlist.items.filter((_,i)=>i!==index))}>{t('music.remove')}</button></li>)}</ol>
        </>}
      </section>
    </>}
    <dialog ref={confirmation} className="app-card max-w-lg" onCancel={()=>setRemoving(null)} aria-labelledby="playlist-delete-heading">
      <h3 id="playlist-delete-heading">{t('music.delete')}</h3><p className="break-words my-4">{removing?.name}</p><p>{t('music.deleteConfirm')}</p>
      {error&&<p role="alert">{t('music.failed')}</p>}
      <div className="flex flex-wrap gap-3 my-4"><button className="secondary-button" disabled={busy} onClick={()=>setRemoving(null)}>{t('files.cancel')}</button>
        <button className="primary-button" disabled={busy||!removing} onClick={()=>void command('delete_playlist',{id:removing.id,confirmed:true}).then(result=>{if(result)setRemoving(null);})}>{t('music.delete')}</button></div>
    </dialog>
  </section>;
}
