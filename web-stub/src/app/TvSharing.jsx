// Reuse the existing Sharing layout, NOMAD Input and managed-media catalog.
import React,{useEffect,useState} from 'react';
import {Link} from 'react-router-dom';
import {useQuery,useMutation,useQueryClient} from '@tanstack/react-query';
import {useApp} from './context.js';
import Input from '../ui/Input.jsx';
import {useMusic} from '../features/music/MusicPlayer.jsx';
import {mediaCategory,searchMedia} from '../features/library/media-catalog.js';

export default function TvSharing(){
  const {csrf,t}=useApp(),catalog=useMusic(),cache=useQueryClient();
  const [network,setNetwork]=useState(''),[selected,setSelected]=useState([]),[query,setQuery]=useState(''),[page,setPage]=useState(0);
  const key=['tv',csrf];
  async function request(path='',body,signal){
    const reply=await fetch(`/api/tv${path}`,{method:body?'POST':'GET',signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf,...(body?{'Content-Type':'application/json'}:{})},...(body?{body:JSON.stringify(body)}:{})});
    if(!reply.ok)throw Error('tv_unavailable');return reply.json();
  }
  const change=useMutation({mutationFn:async({path,body})=>{await cache.cancelQueries({queryKey:key});await request(path,body);await cache.invalidateQueries({queryKey:key});},onError:()=>cache.invalidateQueries({queryKey:key})});
  const data=useQuery({queryKey:key,queryFn:({signal})=>request('',undefined,signal),enabled:!change.isPending,refetchInterval:2000,refetchIntervalInBackground:false,gcTime:0,retry:false});
  const status=data.data?.status,busy=change.isPending||status?.busy;
  useEffect(()=>{if(status)setNetwork(current=>status.networks.some(n=>n.id===current)?current:status.networks[0]?.id||'');},[status]);
  const available=(catalog?.files||[]).filter(file=>['music','video','images'].includes(mediaCategory(file.name)));
  const valid=selected.filter(id=>available.some(file=>file.id===id)),matched=searchMedia(available,query);
  const act=(path,body={})=>change.mutate({path,body});
  return <section className="app-card space-y-4" data-testid="tv-sharing">
    <h3>{t('tv.title')}</h3><p>{t('tv.help')}</p><p className="font-semibold" data-testid="tv-warning">{t('tv.warning')}</p>
    {(data.isError||change.isError||status?.error)&&<p role="alert">{t('tv.error')}</p>}
    {!status&&!data.isError&&<p role="status">{t('app.loading')}</p>}
    {status&&<><p role="status">{t(busy?'tv.working':status.running?'tv.on':'tv.off')}</p>
      {status.running||status.busy?<>
        <p>{t('tv.connect')}</p><h4>{t('sharing.chosenFiles')}</h4>
        <ul>{data.data.selected_media.map(file=><li key={file.id} className="break-words">{file.name}</li>)}</ul><p>{t('tv.snapshotHelp')}</p>
        {status.url&&<details><summary>{t('tv.manual')}</summary><code className="break-all">{status.url}</code></details>}
        <button className="chat-control" disabled={change.isPending} onClick={()=>act('/stop')}>{t('tv.stop')}</button>
      </>:<>
        {!available.length?<p>{t('tv.empty')} <Link className="underline" to="/files">{t('nav.files')}</Link></p>:<>
          <Input name="tv-media-search" label={t('unified.search')} type="search" value={query} maxLength={512} onChange={event=>{setQuery(event.target.value);setPage(0);}}/>
          <fieldset disabled={busy} className="space-y-3"><legend>{t('tv.choose')}</legend>{matched.slice(page*50,page*50+50).map(file=><label key={file.id} className="flex gap-3 items-center break-words"><input type="checkbox" data-tv-media-id={file.id} checked={selected.includes(file.id)} disabled={!selected.includes(file.id)&&valid.length>=128} onChange={event=>setSelected(current=>event.target.checked?[...current,file.id]:current.filter(id=>id!==file.id))}/><span>{file.name}<small className="block">{file.folder}</small></span></label>)}</fieldset>
          {matched.length>50&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*50>=matched.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
        </>}
        {!!valid.length&&<p>{t('sharing.selectedCount',{count:valid.length})}</p>}
        {status.networks.length>1&&<label>{t('sharing.network')}<select className="block rounded border p-3" value={network} disabled={busy} onChange={event=>setNetwork(event.target.value)}>{status.networks.map(n=><option key={n.id} value={n.id}>{n.label}</option>)}</select></label>}
        {!network&&<p>{t('tv.noNetwork')}</p>}
        <button className="chat-control" disabled={busy||!network||!valid.length} onClick={()=>act('/start',{network,media:valid})}>{t('tv.start')}</button>
      </>}
    </>}
    {data.isError&&<button className="secondary-button" disabled={change.isPending} onClick={()=>act('/stop')}>{t('tv.stop')}</button>}
    <p>{t('tv.lifetime')}</p>
  </section>;
}
