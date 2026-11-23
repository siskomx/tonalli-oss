import React,{useCallback,useEffect,useRef,useState} from 'react';
import {useQueryClient} from '@tanstack/react-query';
import {useApp} from '../../app/context.js';
import {readingRequest} from '../library/reading-client.js';
import {bookResourceUrl} from './book-client.js';
import Epub from './Epub.jsx';
import Comic from './Comic.jsx';

export default function Book({file,article,route}){
  const {csrf,t}=useApp(),cache=useQueryClient(),callbacks=useRef(route);callbacks.current=route;
  const id=file?.id,version=file?.version,title=file?.name||article?.title,pack=article?.pack,url=article?.url;
  const[data,setData]=useState(null),[failed,setFailed]=useState(false),[saveFailed,setSaveFailed]=useState(false),last=useRef('');
  useEffect(()=>{
    let closed=false;const controller=new AbortController();setData(null);setFailed(false);setSaveFailed(false);last.current='';
    async function open(){
      const [response,history]=await Promise.all([fetch('/api/documents/archive',{method:'POST',credentials:'same-origin',cache:'no-store',signal:controller.signal,headers:{'Content-Type':'application/json','X-Tonalli-Csrf':csrf},body:JSON.stringify(pack?{pack,url}:{id})}),readingRequest(csrf).catch(()=>{if(!closed)setSaveFailed(true);return{entries:[]};})]);
      if(!response.ok)throw Error('book_unavailable');const value=await response.json();
      if(!['epub','comic'].includes(value.book?.kind)||!Array.isArray(value.book.chapters)||!value.book.chapters.length||value.book.chapters.length>16384)throw Error('invalid_book');
      for(const chapter of value.book.chapters)bookResourceUrl(value.base_url,chapter.path);
      const saved=history.entries.find(entry=>pack?entry.destination.kind==='article'&&entry.destination.pack===pack&&entry.destination.url===url:entry.destination.kind==='media'&&entry.destination.id===id)?.position;
      if(!closed)setData({...value,saved});
    }
    void open().catch(problem=>{if(!closed&&problem.name!=='AbortError'){setFailed(true);callbacks.current.markReady({feature:'reader',unsupported:true});}});
    return()=>{closed=true;controller.abort();};
  },[csrf,id,version,pack,url]);
  const remember=useCallback(position=>{
    const key=JSON.stringify(position);if(key===last.current)return;last.current=key;
    void readingRequest(csrf,'POST',{destination:pack?{kind:'article',pack,url}:{kind:'media',id},position,title,bookmarked:false})
      .then(()=>{setSaveFailed(false);cache.invalidateQueries({queryKey:['reading',csrf]});}).catch(()=>{last.current='';setSaveFailed(true);});
  },[csrf,id,title,pack,url,cache]);
  return <div className="space-y-3" data-testid="book-reader">{failed?<p role="alert">{t('book.failed')}</p>:!data?<p role="status">{t('app.loading')}</p>:data.book.kind==='epub'?<Epub file={file||{name:title}} data={data} route={route} remember={remember}/>:<Comic data={data} route={route} remember={remember}/>}{saveFailed&&<p role="alert">{t('reading.failed')}</p>}</div>;
}
