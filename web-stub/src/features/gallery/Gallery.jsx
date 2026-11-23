import React,{useEffect,useMemo,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {useQuery,useQueryClient} from '@tanstack/react-query';
import {useApp,useRouteReadiness} from '../../app/context.js';
import Input from '../../ui/Input.jsx';
import {mediaCategory,mediaHref,searchMedia} from '../library/media-catalog.js';
import {readingRequest} from '../library/reading-client.js';
import {videoRequest} from '../video/client.js';
import {IconPhoto,IconMovie} from '@tabler/icons-react';

function ImageView({file,large=false,onReady}){
  const {csrf,t}=useApp(),[url,setUrl]=useState(''),[failed,setFailed]=useState(false);
  useEffect(()=>{
    let closed=false;const controller=new AbortController();setUrl('');setFailed(false);
    videoRequest(csrf,'poster',{id:file.id},controller.signal).then(value=>{if(!closed)setUrl(value.url);}).catch(()=>{if(!closed){setFailed(true);onReady?.();}});
    return()=>{closed=true;controller.abort();};
  },[csrf,file.id,file.version]);
  return <div className={large?'overflow-auto max-h-[75vh]':'aspect-video flex items-center justify-center overflow-hidden bg-black/5 rounded'}>
    {url&&!failed?<img data-testid={large?'gallery-image':'gallery-thumbnail'} src={url} alt={large?file.name:''} loading={large?'eager':'lazy'}
      className={large?'max-w-full h-auto mx-auto':'w-full h-full object-contain'} onLoad={()=>onReady?.()} onError={()=>{setFailed(true);onReady?.();}}/>:<IconPhoto size={48} aria-hidden="true"/>}
    {failed&&large&&<p role="alert">{t('gallery.unsupported')}</p>}
  </div>;
}
export default function Gallery(){
  const {csrf,t,beginNavigation}=useApp(),route=useRouteReadiness('gallery'),cache=useQueryClient(),[params]=useSearchParams();
  const [query,setQuery]=useState(''),[folder,setFolder]=useState(''),[page,setPage]=useState(0),[historyError,setHistoryError]=useState(false);
  const files=useQuery({queryKey:['managed-media',csrf],staleTime:0,refetchOnMount:'always',queryFn:async({signal})=>{
    const response=await fetch('/api/files',{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});
    if(!response.ok)throw Error('files_unavailable');return(await response.json()).files;
  }});
  const all=useMemo(()=>(files.data||[]).filter(file=>['images','video'].includes(mediaCategory(file.name))),[files.data]);
  const rows=searchMedia(all,query).filter(file=>!folder||file.folder===folder),selected=all.find(file=>file.id===params.get('file'));
  const images=all.filter(file=>mediaCategory(file.name)==='images'),index=images.findIndex(file=>file.id===selected?.id);
  const link=file=>{const href=mediaCategory(file.name)==='images'?`/gallery?${new URLSearchParams({file:file.id})}`:mediaHref(file);return{to:href,onClick:event=>beginNavigation(href,event)};};
  useEffect(()=>{if(files.error)route.markError(files.error);else if(files.data&&(!selected||mediaCategory(selected.name)==='video'))void route.markReady({feature:'gallery'});},[files.data,files.error,selected,route]);
  useEffect(()=>{
    setHistoryError(false);if(!selected||mediaCategory(selected.name)!=='images')return;let closed=false;
    readingRequest(csrf,'POST',{destination:{kind:'media',id:selected.id},position:{kind:'start'},title:selected.name,bookmarked:false})
      .then(()=>cache.invalidateQueries({queryKey:['reading',csrf]})).catch(()=>{if(!closed)setHistoryError(true);});
    return()=>{closed=true;};
  },[selected?.id,csrf]);
  return <section className="space-y-5" data-testid="gallery-page"><h2>{t('nav.gallery')}</h2><p>{t('gallery.help')}</p>
    <Link className="underline" to="/files" onClick={event=>beginNavigation('/files',event)}>{t('gallery.import')}</Link>
    {files.isPending&&<p role="status">{t('app.loading')}</p>}{files.error&&<p role="alert">{t('gallery.failed')}</p>}
    {params.get('file')&&!selected&&files.data&&<p role="alert">{t('gallery.missing')}</p>}
    {selected&&mediaCategory(selected.name)==='images'&&<section className="app-card space-y-3" data-testid="gallery-viewer">
      <h3 className="break-words">{selected.name}</h3><ImageView key={`${selected.id}:${selected.version}`} file={selected} large onReady={()=>void route.markReady({feature:'gallery',selected:selected.id})}/>
      <div className="flex gap-3 flex-wrap">{index>0&&<Link className="secondary-button" {...link(images[index-1])}>{t('gallery.previous')}</Link>}{index+1<images.length&&<Link className="secondary-button" {...link(images[index+1])}>{t('gallery.next')}</Link>}
        <Link className="underline" to={`/file?${new URLSearchParams({file:selected.id})}`} onClick={event=>beginNavigation(`/file?${new URLSearchParams({file:selected.id})}`,event)}>{t('gallery.file')}</Link></div>
      {historyError&&<p role="alert">{t('reading.failed')}</p>}
    </section>}
    {selected&&mediaCategory(selected.name)==='video'&&<Link className="underline" {...link(selected)}>{selected.name}</Link>}
    <Input name="gallery-search" label={t('unified.search')} type="search" value={query} maxLength={512} onChange={event=>{setQuery(event.target.value);setPage(0);}}/>
    <label className="block">{t('files.browseFolder')}<select className="block rounded border p-2" value={folder} onChange={event=>{setFolder(event.target.value);setPage(0);}}><option value="">{t('files.allFolders')}</option>{[...new Set(all.map(file=>file.folder).filter(Boolean))].sort().map(name=><option key={name}>{name}</option>)}</select></label>
    {!files.isPending&&!rows.length&&<p>{t('gallery.empty')}</p>}
    <ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">{rows.slice(page*24,page*24+24).map(file=><li className="app-card" key={file.id}><Link {...link(file)}>
      {mediaCategory(file.name)==='images'?<ImageView file={file}/>:<div className="aspect-video flex items-center justify-center bg-black/5"><IconMovie size={48} aria-hidden="true"/></div>}<h3 className="underline break-words mt-2">{file.name}</h3><p className="text-sm break-words">{file.folder}</p>
    </Link></li>)}</ul>
    {rows.length>24&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*24>=rows.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
  </section>;
}
