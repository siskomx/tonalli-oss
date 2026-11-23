import React,{useEffect,useState} from 'react';
import {Link} from 'react-router-dom';
import {useQueries,useQuery} from '@tanstack/react-query';
import {useApp} from '../../app/context.js';
import Input from '../../ui/Input.jsx';
import Select from '../../ui/Select.jsx';
import {loadSuggestions} from './client.js';
import {mediaCategory,mediaHref,searchMedia} from './media-catalog.js';
import {loadInstalledMaps} from '../../../map-client.js';

export default function UnifiedLibrary({packs=[]}){
  const {csrf,t,beginNavigation,setLastArticle}=useApp();
  const [query,setQuery]=useState(''),[term,setTerm]=useState(''),[category,setCategory]=useState('all'),[page,setPage]=useState(0);
  useEffect(()=>{const timer=setTimeout(()=>{setTerm(query.trim());setPage(0);},200);return()=>clearTimeout(timer);},[query]);
  const files=useQuery({queryKey:['managed-media',csrf],staleTime:0,refetchOnMount:'always',queryFn:async({signal})=>{
    const response=await fetch('/api/files',{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});
    if(!response.ok)throw Error('files_unavailable');return (await response.json()).files;
  }});
  const suggestions=useQueries({queries:packs.map(pack=>({queryKey:['title-search',pack.id,term,csrf],enabled:!!term&&['all','books'].includes(category),
    queryFn:({signal})=>loadSuggestions(pack.id,term,csrf,signal)}))});
  const maps=useQuery({queryKey:['unified-maps',csrf],staleTime:0,refetchOnMount:'always',queryFn:async()=>{
    const installed=await loadInstalledMaps(csrf);
    const response=await fetch('/api/map-management',{credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});
    if(!response.ok)throw Error('maps_unavailable');const managed=await response.json();
    return installed.map(map=>{const country=/^country-(mx|us|ca)-/.exec(map.id)?.[1];return {...map,title:country?t(`map.country.${country}`):managed.imports.find(file=>file.id===map.id)?.title||t('map.mapFile')};});
  }});
  const mapRows=['all','maps'].includes(category)?(maps.data||[]).filter(map=>!term||map.title.toLowerCase().includes(term.toLowerCase())):[];
  const matched=searchMedia(files.data||[],term,category);
  const books=['all','books'].includes(category)?packs.filter(pack=>!term||pack.title.toLowerCase().includes(term.toLowerCase())):[];
  const categories=['all','books','maps','music','video','images','comics','recipes','workshop','files'];
  const link=(href)=>({to:href,onClick:event=>beginNavigation(href,event)});
  return <section className="app-card my-5 space-y-4" data-testid="unified-library">
    <h3>{t('unified.heading')}</h3><p>{t('unified.help')}</p>
    <Input name="global-search" label={t('unified.search')} type="search" value={query} maxLength={512} onChange={event=>setQuery(event.target.value)}/>
    <Select name="media-category" label={t('unified.category')} value={category} onChange={value=>{setCategory(value);setPage(0);}} options={categories.map(value=>({value,label:t(`unified.${value}`)}))}/>
    {files.isPending&&<p role="status">{t('app.loading')}</p>}
    {(files.error||maps.error||suggestions.some(result=>result.error))&&<p role="alert">{t('unified.failed')}</p>}
    <ul className="space-y-3">
      {books.map(pack=><li key={`book:${pack.id}`}><Link className="underline" {...link(`/library?${new URLSearchParams({pack:pack.id})}`)}>{pack.title}</Link> — {t('unified.books')}</li>)}
      {mapRows.map(map=><li key={`map:${map.id}`}><Link className="underline" {...link(`/map?${new URLSearchParams({pack:map.id})}`)}>{map.title}</Link> — {t('unified.maps')}</li>)}
      {matched.slice(page*100,page*100+100).map(file=><li key={file.id}><Link className="underline break-words" {...link(mediaHref(file))}>{file.name}</Link><p className="text-sm break-words">{t(`unified.${mediaCategory(file.name)}`)}{file.folder?` · ${file.folder}`:''}</p></li>)}
    </ul>
    {matched.length>100&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*100>=matched.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
    {term&&['all','books'].includes(category)&&<div aria-live="polite"><h4>{t('unified.articles')}</h4>
      {suggestions.some(result=>result.isFetching)?<p>{t('app.loading')}</p>:suggestions.map((result,index)=><ul key={packs[index].id}>{result.data?.hits.map(hit=>{
        const href=`/article?${new URLSearchParams({pack:packs[index].id,url:hit.url})}`;
        return <li key={hit.url}><Link className="underline" to={href} onClick={event=>{if(beginNavigation(href,event))setLastArticle({pack:packs[index].id,url:hit.url});}}>{hit.title}</Link> · {packs[index].title}</li>;
      })}</ul>)}</div>}
    {!files.isPending&&!maps.isPending&&!matched.length&&!books.length&&!mapRows.length&&!term&&<p>{t('unified.empty')}</p>}
  </section>;
}
