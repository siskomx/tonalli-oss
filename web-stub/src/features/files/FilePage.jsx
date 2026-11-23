import React,{useEffect,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {useQuery} from '@tanstack/react-query';
import {useApp,useRouteReadiness} from '../../app/context.js';
import {nativeShell} from '../../app/AppLayout.jsx';
import {mediaCategory,mediaHref,searchMedia} from '../library/media-catalog.js';
import {formatBytes} from '../content/format.js';
import Player from '../video/Player.jsx';
import Input from '../../ui/Input.jsx';

export default function FilePage({shared=false}){
  const {csrf,t,language,beginNavigation}=useApp(),route=useRouteReadiness(shared?'shared':'file'),[params]=useSearchParams();
  const [query,setQuery]=useState(''),[page,setPage]=useState(0);
  const files=useQuery({queryKey:[shared?'shared-media':'managed-media',csrf],staleTime:0,refetchOnMount:'always',queryFn:async({signal})=>{
    const response=await fetch(shared?'/api/shared-media':'/api/files',{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});
    if(!response.ok)throw Error('files_unavailable');return(await response.json()).files;
  }});
  // The ordinary managed-media cache contains MediaFile rows, not share wrappers.
  const rows=(files.data||[]).map(row=>row.file?row:{file:row,inline_mime:null});
  const selected=rows.find(row=>row.file.id===params.get('file'));
  const text=useQuery({queryKey:['file-text',shared,selected?.file.id,selected?.file.version,csrf],enabled:!!selected&&selected.file.bytes<=65536&&/\.(txt|md|csv|json|toml|ya?ml|log|cook)$/i.test(selected.file.name),queryFn:async({signal})=>{
    const response=await fetch(`${shared?'/api/shared-media':'/api/files'}/${selected.file.id}/text`,{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});
    if(!response.ok)throw Error('file_text_unavailable');return response.json();
  }});
  useEffect(()=>{if(files.error)route.markError(files.error);else if(files.data)void route.markReady({feature:shared?'shared':'file'});},[files.data,files.error,route,shared]);
  const link=file=>{const href=`/${shared?'shared':'file'}?${new URLSearchParams({file:file.id})}`;return{to:href,onClick:event=>beginNavigation(href,event)};};
  const matched=searchMedia(rows.map(row=>row.file),query);
  return <section className="space-y-5" data-testid={shared?'shared-files-page':'file-page'}><h2>{t(shared?'nav.shared':'file.heading')}</h2><p>{t(shared?'shared.help':'file.help')}</p>
    {(files.error||text.error)&&<p role="alert">{t('files.error.files_unavailable')}</p>}{files.isPending&&<p role="status">{t('app.loading')}</p>}
    {params.get('file')&&!selected&&files.data&&<p role="alert">{t('gallery.missing')}</p>}
    {selected&&<section className="app-card space-y-4" key={`${selected.file.id}:${selected.file.version}`} data-testid="file-viewer"><h3 className="break-words">{selected.file.name}</h3><p className="break-words">{selected.file.folder} · {formatBytes(selected.file.bytes,language)}</p>
      {shared&&selected.inline_mime?.startsWith('image/')&&<SharedImage file={selected.file} t={t}/>}
      {shared&&selected.inline_mime?.startsWith('video/')&&<Player file={selected.file} sharedSource={`/shared-media/${selected.file.id}/content`} onReady={()=>void route.markReady({feature:'shared'})}/>}
      {shared&&selected.inline_mime?.startsWith('audio/')&&<audio controls preload="metadata" src={`/shared-media/${selected.file.id}/content`}/>}
      {text.isFetching&&<p role="status">{t('app.loading')}</p>}{text.data&&<pre className="whitespace-pre-wrap break-words max-h-[70vh] overflow-auto" data-testid="file-preview-text">{text.data.text}</pre>}
      {!shared&&['images','video','music','books','comics','recipes','workshop'].includes(mediaCategory(selected.file.name))&&<Link className="secondary-button" to={mediaHref(selected.file)} onClick={event=>beginNavigation(mediaHref(selected.file),event)}>{t('file.open')}</Link>}
      {shared?<a className="secondary-button" href={`/shared-media/${selected.file.id}/download`} download={selected.file.name}>{t('files.download')}</a>:nativeShell()&&<a className="secondary-button" href={`/native/export-media/${selected.file.id}`}>{t('files.download')}</a>}
      {!selected.inline_mime&&!text.data&&shared&&<p>{t('shared.downloadHelp')}</p>}
    </section>}
    <Input name={shared?'shared-search':'file-search'} label={t('unified.search')} type="search" value={query} maxLength={512} onChange={event=>{setQuery(event.target.value);setPage(0);}}/>
    {!files.isPending&&!matched.length&&<p>{t(shared?'shared.empty':'files.empty')}</p>}
    <ul className="space-y-3">{matched.slice(page*100,page*100+100).map(file=><li key={file.id}><Link className="underline break-words" {...link(file)}>{file.name}</Link><p className="text-sm break-words">{file.folder}</p></li>)}</ul>
    {matched.length>100&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*100>=matched.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
  </section>;
}
function SharedImage({file,t}){const[failed,setFailed]=useState(false);return failed?<p role="alert">{t('gallery.unsupported')}</p>:<img className="max-w-full max-h-[75vh] object-contain mx-auto" alt={file.name} src={`/shared-media/${file.id}/content`} onError={()=>setFailed(true)}/>;}
