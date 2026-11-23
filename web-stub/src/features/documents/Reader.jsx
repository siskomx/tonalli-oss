import React,{useEffect,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {useQuery} from '@tanstack/react-query';
import {useApp,useRouteReadiness} from '../../app/context.js';
import {mediaCategory,mediaHref,searchMedia} from '../library/media-catalog.js';
import Input from '../../ui/Input.jsx';
import Pdf from './Pdf.jsx';
import Book from './Book.jsx';

export default function Reader(){
  const {csrf,t,beginNavigation}=useApp(),route=useRouteReadiness('reader'),[params]=useSearchParams(),[query,setQuery]=useState(''),[page,setPage]=useState(0);
  const files=useQuery({queryKey:['managed-media',csrf],staleTime:0,refetchOnMount:'always',queryFn:async({signal})=>{
    const response=await fetch('/api/files',{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});if(!response.ok)throw Error('files_unavailable');return(await response.json()).files;
  }});
  const all=(files.data||[]).filter(file=>['books','comics'].includes(mediaCategory(file.name))),selected=all.find(file=>file.id===params.get('file')),rows=searchMedia(all,query);
  const pdf=selected?.name.toLowerCase().endsWith('.pdf');
  useEffect(()=>{if(files.error)route.markError(files.error);else if(files.data&&!selected)void route.markReady({feature:'reader'});},[files.data,files.error,selected,route]);
  return <section className="space-y-5" data-testid="document-page"><h2>{t('nav.reader')}</h2><p>{t('document.help')}</p>
    <Link className="underline" to="/files" onClick={event=>beginNavigation('/files',event)}>{t('document.import')}</Link>
    {files.isPending&&<p role="status">{t('app.loading')}</p>}{files.error&&<p role="alert">{t('files.error.files_unavailable')}</p>}
    {params.get('file')&&!selected&&files.data&&<p role="alert">{t('gallery.missing')}</p>}
    {selected&&<section className="space-y-4" key={`${selected.id}:${selected.version}`}><h3 className="break-words">{selected.name}</h3>
      <Link className="underline" to={`/file?file=${selected.id}`} onClick={event=>beginNavigation(`/file?file=${selected.id}`,event)}>{t('gallery.file')}</Link>
      {pdf?<Pdf file={selected} route={route}/>:<Book file={selected} route={route}/>}
    </section>}
    <Input name="document-search" label={t('unified.search')} type="search" maxLength={512} value={query} onChange={event=>{setQuery(event.target.value);setPage(0);}}/>
    {!files.isPending&&!rows.length&&<p>{t('document.empty')}</p>}
    <ul className="space-y-3">{rows.slice(page*100,page*100+100).map(file=><li key={file.id}><Link className="underline break-words" to={mediaHref(file)} onClick={event=>beginNavigation(mediaHref(file),event)}>{file.name}</Link><p className="text-sm break-words">{file.folder}</p></li>)}</ul>
    {rows.length>100&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*100>=rows.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
  </section>;
}
