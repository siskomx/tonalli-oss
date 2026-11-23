import React,{useEffect,useMemo,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {useQuery} from '@tanstack/react-query';
import {useApp,useRouteReadiness} from '../../app/context.js';
import Input from '../../ui/Input.jsx';
import {videoCatalog} from './catalog.js';
import Player from './Player.jsx';
import Poster from './Poster.jsx';

export default function Videos(){
  const {csrf,t,beginNavigation}=useApp(),route=useRouteReadiness('videos'),[params]=useSearchParams();
  const [query,setQuery]=useState(''),[page,setPage]=useState(0),[showKey,setShowKey]=useState('');
  const [showPage,setShowPage]=useState(0);
  const files=useQuery({queryKey:['managed-media',csrf],staleTime:0,refetchOnMount:'always',queryFn:async({signal})=>{
    const response=await fetch('/api/files',{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});
    if(!response.ok)throw Error('files_unavailable');return(await response.json()).files;
  }});
  const catalog=useMemo(()=>videoCatalog(files.data||[],query),[files.data,query]);
  const complete=useMemo(()=>videoCatalog(files.data||[]),[files.data]);
  const episodes=complete.shows.flatMap(show=>show.seasons.flatMap(season=>season.episodes));
  const selected=[...complete.movies,...episodes].find(file=>file.id===params.get('file'));
  const show=catalog.shows.find(show=>show.key===showKey);
  useEffect(()=>{if(files.error)route.markError(files.error);else if(files.data&&!selected)void route.markReady({feature:'videos',missing:!!params.get('file')});},[files.data,files.error,selected,params,route]);
  const link=id=>{const href=`/videos?${new URLSearchParams({file:id})}`;return{to:href,onClick:event=>beginNavigation(href,event)};};
  return <section className="space-y-5" data-testid="videos-page"><h2>{t('nav.videos')}</h2><p>{t('video.help')}</p>
    {files.isPending&&<p role="status">{t('app.loading')}</p>}{files.error&&<p role="alert">{t('video.failed')}</p>}
    {params.get('file')&&!selected&&!files.isPending&&<p role="alert">{t('video.missing')}</p>}
    {selected&&<Player key={`${selected.id}:${selected.version}`} file={selected} poster={selected.poster} onReady={()=>void route.markReady({feature:'videos',selected:selected.id})}/>}
    <Link className="underline" to="/files" onClick={event=>beginNavigation('/files',event)}>{t('video.import')}</Link>
    <Input name="video-search" label={t('unified.search')} type="search" value={query} maxLength={512} onChange={event=>{setQuery(event.target.value);setPage(0);setShowPage(0);}}/>
    <p>{t('video.coversHelp')}</p>
    {!files.isPending&&!catalog.movies.length&&!catalog.shows.length&&<p>{t('video.empty')}</p>}
    {!!catalog.movies.length&&<section className="space-y-3"><h3>{t('video.movies')}</h3><ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {catalog.movies.slice(page*24,page*24+24).map(file=><li key={file.id} className="app-card"><Link {...link(file.id)}><Poster id={file.poster}/><h4 className="underline break-words mt-2">{file.title}</h4><p className="break-words text-sm">{file.folder}</p></Link></li>)}
    </ul>{catalog.movies.length>24&&<div className="flex gap-2"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*24>=catalog.movies.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}</section>}
    {!!catalog.shows.length&&<section className="space-y-3"><h3>{t('video.shows')}</h3><ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {catalog.shows.slice(showPage*24,showPage*24+24).map(show=><li key={show.key}><button className="app-card w-full text-left" aria-pressed={showKey===show.key} onClick={()=>setShowKey(show.key)}><Poster id={show.poster}/><h4 className="underline break-words mt-2">{show.title||t('video.unnamedShow')}</h4></button></li>)}
    </ul>{catalog.shows.length>24&&<div className="flex gap-2"><button className="secondary-button" disabled={!showPage} onClick={()=>setShowPage(showPage-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(showPage+1)*24>=catalog.shows.length} onClick={()=>setShowPage(showPage+1)}>{t('library.next')}</button></div>}</section>}
    {show&&<section className="app-card space-y-4" data-testid="show-seasons"><h3>{show.title||t('video.unnamedShow')}</h3>{show.seasons.map(season=><section key={season.number}><h4>{season.number?t('video.season',{number:season.number}):t('video.specials')}</h4><ul className="space-y-2">{season.episodes.map(episode=><li key={episode.id}><Link className="underline break-words" {...link(episode.id)}>{episode.title}</Link></li>)}</ul></section>)}</section>}
  </section>;
}
