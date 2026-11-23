import React,{useEffect,useRef,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {useQuery,useQueryClient} from '@tanstack/react-query';
import {useApp,useRouteReadiness} from '../../app/context.js';
import {mediaCategory,mediaHref,searchMedia} from '../library/media-catalog.js';
import Input from '../../ui/Input.jsx';
import ModelCanvas from './ModelCanvas.jsx';
import {readingRequest} from '../library/reading-client.js';

export default function Workshop(){
  const{csrf,t,beginNavigation}=useApp(),route=useRouteReadiness('workshop'),[params]=useSearchParams(),[query,setQuery]=useState(''),[page,setPage]=useState(0);
  const files=useQuery({queryKey:['managed-media',csrf],staleTime:0,refetchOnMount:'always',queryFn:async({signal})=>{const response=await fetch('/api/files',{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});if(!response.ok)throw Error('files_unavailable');return(await response.json()).files;}});
  const all=(files.data||[]).filter(file=>mediaCategory(file.name)==='workshop'),selected=all.find(file=>file.id===params.get('file')),rows=searchMedia(all,query);
  useEffect(()=>{if(files.error)route.markError(files.error);else if(files.data&&!selected)void route.markReady({feature:'workshop'});},[files.data,files.error,selected,route]);
  return <section className="space-y-5" data-testid="workshop-page"><h2>{t('nav.workshop')}</h2><p>{t('workshop.help')}</p>
    <Link className="secondary-button" to="/files" onClick={event=>beginNavigation('/files',event)}>{t('workshop.import')}</Link>
    {files.isPending&&<p role="status">{t('app.loading')}</p>}{files.error&&<p role="alert">{t('files.error.files_unavailable')}</p>}
    {params.get('file')&&!selected&&files.data&&<p role="alert">{t('gallery.missing')}</p>}
    {selected&&<section key={`${selected.id}:${selected.version}`} className="space-y-4"><h3 className="break-words">{selected.name}</h3><Link className="underline" to={`/file?file=${selected.id}`} onClick={event=>beginNavigation(`/file?file=${selected.id}`,event)}>{t('gallery.file')}</Link><Model file={selected} route={route}/></section>}
    <Input name="workshop-search" label={t('unified.search')} type="search" maxLength={512} value={query} onChange={event=>{setQuery(event.target.value);setPage(0);}}/>
    {!files.isPending&&!rows.length&&<p>{t('workshop.empty')}</p>}
    <ul className="space-y-3">{rows.slice(page*100,page*100+100).map(file=><li key={file.id}><Link className="underline break-words" to={mediaHref(file)} onClick={event=>beginNavigation(mediaHref(file),event)}>{file.name}</Link><p className="text-sm break-words">{file.folder}</p></li>)}</ul>
    {rows.length>100&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*100>=rows.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
  </section>;
}
function Model({file,route}){
  const{csrf,t}=useApp(),cache=useQueryClient(),callbacks=useRef(route);callbacks.current=route;
  const[model,setModel]=useState(null),[error,setError]=useState(''),[historyError,setHistoryError]=useState(false),[attempt,setAttempt]=useState(0);
  useEffect(()=>{
    let closed=false;const controller=new AbortController();setModel(null);setError('');callbacks.current.markPending();
    async function open(){
      const response=await fetch('/api/workshop/model',{method:'POST',signal:controller.signal,credentials:'same-origin',cache:'no-store',headers:{'Content-Type':'application/json','X-Tonalli-Csrf':csrf},body:JSON.stringify({id:file.id})});
      if(!response.ok)throw Error(response.status===429?'workshop.busy':response.status===415?'workshop.unsupported':'workshop.failed');
      const value=await response.json();if(!closed){setModel(value.model);void readingRequest(csrf,'POST',{destination:{kind:'media',id:file.id},position:{kind:'start'},title:file.name,bookmarked:false}).then(()=>cache.invalidateQueries({queryKey:['reading',csrf]})).catch(()=>{if(!closed)setHistoryError(true);});}
    }
    void open().catch(failure=>{if(!closed&&failure.name!=='AbortError'){setError(failure.message);void callbacks.current.markReady({feature:'workshop',unsupported:true});}});
    return()=>{closed=true;controller.abort();};
  },[csrf,file.id,file.version,attempt]);
  return <div className="space-y-3">{historyError&&<p role="alert">{t('reading.failed')}</p>}{error?<><p role="alert">{t(error)}</p><button className="secondary-button" onClick={()=>setAttempt(value=>value+1)}>{t('files.refresh')}</button></>:!model?<p role="status">{t('app.loading')}</p>:<ModelCanvas model={model} route={route}/>}</div>;
}
