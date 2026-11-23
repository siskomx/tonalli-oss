import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom';
import { useApp, useRouteReadiness } from '../../app/context.js';
import { attachArticleFrame } from './frame.js';
import {readingRequest} from '../library/reading-client.js';
import {useQueryClient} from '@tanstack/react-query';
import Embedded from '../documents/Embedded.jsx';

export default function Article() {
  const { csrf, t, setLastArticle,canReadProgress } = useApp();
  const [progressError,setProgressError]=useState(false);
  const [textSize,setTextSize]=useState(100),reader=useRef(null);
  const [embedded,setEmbedded]=useState(null);
  const navigate=useNavigate(),documentBack=useLocation().state?.documentBack;
  const queryClient=useQueryClient();
  const readiness = useRouteReadiness('article');
  const callbacks = useRef(readiness);
  const loaded = useRef(null);
  useLayoutEffect(() => { callbacks.current = readiness; }, [readiness]);
  const [search] = useSearchParams();
  const pack = search.get('pack') ?? undefined;
  const url = search.get('url');
  const frame = useRef(null);
  const title = useRef(null);
  const status = useRef(null);
  useEffect(() => {
    let active=true,timer,pending;const recent=new Map();setProgressError(false);setEmbedded(null);
    const saved=canReadProgress?readingRequest(csrf).catch(()=>{if(active)setProgressError(true);return null;}):Promise.resolve(null);
    function flush(){clearTimeout(timer);if(!pending)return;const value=pending;pending=null;void readingRequest(csrf,'POST',value).then(()=>queryClient.invalidateQueries({queryKey:['reading',csrf]})).catch(()=>{if(active)setProgressError(true);});}
    function remember(value){if(!canReadProgress)return;pending={destination:{kind:'article',pack:value.pack,url:value.url},position:{kind:'scroll',y:value.y},title:value.title,bookmarked:false};recent.set(`${value.pack}\n${value.url}`,pending);clearTimeout(timer);timer=setTimeout(flush,600);}
    loaded.current = null;
    if (!url) {
      loaded.current = { article: { empty: true, resourcesLoaded: false } };
      void callbacks.current.markReady(loaded.current);
      return()=>{active=false;};
    }
    // Navigation generations change the guarded callbacks, not this data's lifetime.
    const controller = attachArticleFrame({ frame: frame.current, title: title.current, status: status.current, csrf, t, textSize,
      onDocument:(article,back)=>{
        if(!active)return;
        const previous=back||documentBack;
        const target=previous&&typeof previous.pack==='string'&&typeof previous.url==='string'?{pack:previous.pack,url:previous.url}:null;
        setLastArticle({pack:article.pack,url:article.url});
        if(article.pack!==pack||article.url!==url){
          navigate(`/article?${new URLSearchParams({pack:article.pack,url:article.url})}`,{replace:true,state:target?{documentBack:target}:null});
        }else setEmbedded({article,back:target});
      },
      onPosition:remember,
      onPending: () => { loaded.current = null; callbacks.current.markPending(); },
      onError: (error) => { loaded.current = null; callbacks.current.markError(error); }, onReady: (details) => {
        setLastArticle({ pack: details.pack, url: details.url });
        void saved.then(history=>{
          if(!active)return;const current=controller.position();if(!current||current.pack!==details.pack||current.url!==details.url)return;
          const entry=recent.get(`${details.pack}\n${details.url}`)||history?.entries.find(entry=>entry.destination.kind==='article'&&entry.destination.pack===details.pack&&entry.destination.url===details.url);
          if(entry?.position.kind==='scroll'&&!new URL(details.frameUrl).hash)controller.restore(entry.position.y);
          const value=controller.position();if(value)remember(value);
        });
        loaded.current = { article: details };
        void callbacks.current.markReady(loaded.current);
      } });
    reader.current=controller;void controller.open(pack, url);
    return () => {reader.current=null;controller.destroy();flush();active=false;};
  }, [pack, url, csrf, t, setLastArticle,canReadProgress,queryClient,navigate,documentBack]);
  useEffect(() => {
    if (loaded.current) void readiness.markReady(loaded.current);
  }, [readiness]);
  return <section>
    <p id="trusted-sentinel" className="sr-only">{t('article.parent')}</p>
    <h2 ref={title} id="article-title" className="text-lg font-semibold" />
    {url&&!embedded&&<label className="inline-flex gap-3 items-center my-3">{t('article.textSize')}<select aria-label={t('article.textSize')} className="border rounded p-2" value={textSize} onChange={event=>{const value=Number(event.target.value);setTextSize(value);reader.current?.setTextSize(value);}}>{[100,115,130,150].map(value=><option key={value} value={value}>{value}%</option>)}</select></label>}
    {progressError&&<p role="alert">{t('reading.failed')}</p>}
    <p ref={status} id="article-status" role="status" className="my-3">{url ? t('article.loading') : t('article.choose')}</p>
    {url && <iframe ref={frame} id="article-frame" data-testid="article-frame" sandbox="allow-same-origin"
      title={t('article.frameTitle')} className="w-full border border-slate-300 bg-white" style={{ height:'75dvh',minHeight:320,display:embedded?'none':undefined }} />}
    {embedded&&<Embedded key={`${embedded.article.pack}:${embedded.article.url}`} article={embedded.article} route={readiness} onBack={embedded.back?()=>{setEmbedded(null);navigate(`/article?${new URLSearchParams(embedded.back)}`,{replace:true});}:undefined}/>}
  </section>;
}
