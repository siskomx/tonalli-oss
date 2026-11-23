import React,{useEffect,useRef,useState} from 'react';
import {getDocument,GlobalWorkerOptions,TextLayer} from 'pdfjs-dist';
import 'pdfjs-dist/web/pdf_viewer.css';
import {useQueryClient} from '@tanstack/react-query';
import {useApp} from '../../app/context.js';
import {readingRequest} from '../library/reading-client.js';
import {articleFrameUrl} from '../article/client.js';

GlobalWorkerOptions.workerSrc='/assets/pdf/pdf.worker.mjs';

export default function Pdf({file,article,route}){
  const {csrf,t,canReadProgress}=useApp(),cache=useQueryClient(),callbacks=useRef(route);callbacks.current=route;
  const id=file?.id,version=file?.version,title=file?.name||article?.title,pack=article?.pack,urlPath=article?.url;
  const destination=pack?{kind:'article',pack,url:urlPath}:{kind:'media',id};
  const [document,setDocument]=useState(null),[number,setNumber]=useState(1),[zoom,setZoom]=useState(1),[failed,setFailed]=useState(false),[saveFailed,setSaveFailed]=useState(false),[loading,setLoading]=useState(true);
  const area=useRef(null),canvas=useRef(null),text=useRef(null),[width,setWidth]=useState(600);
  useEffect(()=>{
    const observer=new ResizeObserver(entries=>{if(entries[0].contentRect.width>0)setWidth(entries[0].contentRect.width);});
    observer.observe(area.current);return()=>observer.disconnect();
  },[]);
  useEffect(()=>{
    let closed=false,task;const controller=new AbortController();setDocument(null);setFailed(false);setLoading(true);setSaveFailed(false);
    async function open(){
      let source;
      if(article)source=articleFrameUrl(article,location.origin);
      else{
        const response=await fetch('/api/documents/stream',{method:'POST',credentials:'same-origin',cache:'no-store',signal:controller.signal,headers:{'Content-Type':'application/json','X-Tonalli-Csrf':csrf},body:JSON.stringify({id})});
        if(!response.ok)throw Error('document_unavailable');const grant=await response.json();
        const url=new URL(grant.url,location.origin);if(url.origin!==location.origin||!/^\/api\/documents\/content\/[a-f0-9]{64}$/.test(url.pathname)||url.search||url.hash)throw Error('document_invalid');source=url.href;
      }
      if(closed)return;
      task=getDocument({url:source,httpHeaders:{'X-Tonalli-Csrf':csrf},withCredentials:true,rangeChunkSize:65536,disableAutoFetch:true,disableStream:true,disableRange:!!article,
        isEvalSupported:false,enableXfa:false,useWasm:false,stopAtErrors:true,maxImageSize:16*1024*1024,canvasMaxAreaInBytes:64*1024*1024,
        cMapUrl:'/assets/pdf/cmaps/',cMapPacked:true,standardFontDataUrl:'/assets/pdf/standard_fonts/',iccUrl:'/assets/pdf/iccs/',wasmUrl:'/assets/pdf/decoders/'});
      // No document JavaScript, actions, attachments or scripting manager is executed.
      task.onPassword=()=>{void task.destroy();if(!closed){setFailed(true);setLoading(false);callbacks.current.markReady({feature:'reader',unsupported:true});}};
      const [pdf,history]=await Promise.all([task.promise,canReadProgress?readingRequest(csrf).catch(()=>{if(!closed)setSaveFailed(true);return {entries:[]};}):Promise.resolve({entries:[]})]);
      if(closed)return;
      const saved=history.entries.find(entry=>entry.destination.kind===destination.kind&&(pack?entry.destination.pack===pack&&entry.destination.url===urlPath:entry.destination.id===id)&&entry.position.kind==='page');
      setNumber(Math.min(pdf.numPages,Math.max(1,saved?.position.number||1)));setDocument(pdf);
    }
    void open().catch(problem=>{if(!closed&&problem.name!=='AbortError'){setFailed(true);setLoading(false);callbacks.current.markReady({feature:'reader',unsupported:true});}});
    return()=>{closed=true;controller.abort();void task?.destroy();};
  },[id,version,pack,urlPath,csrf,canReadProgress]);
  useEffect(()=>{
    if(!document)return;let closed=false,render,layer,page;setLoading(true);setFailed(false);
    async function draw(){
      page=await document.getPage(number);if(closed)return;
      const normal=page.getViewport({scale:1});
      const scale=Math.min(6,Math.max(.1,(width-4)/normal.width*zoom));const viewport=page.getViewport({scale});
      // Bound a single canvas while retaining page controls and text selection.
      const density=Math.min(devicePixelRatio||1,2,Math.sqrt(16*1024*1024/(viewport.width*viewport.height)));
      const element=canvas.current;const container=text.current;
      element.width=Math.max(1,Math.floor(viewport.width*density));element.height=Math.max(1,Math.floor(viewport.height*density));element.style.width=`${viewport.width}px`;element.style.height=`${viewport.height}px`;
      container.replaceChildren();container.style.setProperty('--scale-factor',scale);container.style.setProperty('--total-scale-factor',scale);container.style.width=`${viewport.width}px`;container.style.height=`${viewport.height}px`;
      render=page.render({canvasContext:element.getContext('2d'),viewport,transform:density===1?null:[density,0,0,density,0,0]});await render.promise;if(closed)return;
      layer=new TextLayer({textContentSource:page.streamTextContent(),container,viewport});await layer.render();if(closed)return;
      setLoading(false);callbacks.current.markReady({feature:'reader',page:number,pages:document.numPages});
      if(canReadProgress)void readingRequest(csrf,'POST',{destination,position:{kind:'page',number},title,bookmarked:false})
        .then(()=>{if(!closed)setSaveFailed(false);cache.invalidateQueries({queryKey:['reading',csrf]});}).catch(()=>{if(!closed)setSaveFailed(true);});
    }
    void draw().catch(problem=>{if(!closed&&problem.name!=='RenderingCancelledException'){setFailed(true);setLoading(false);callbacks.current.markReady({feature:'reader',unsupported:true});}});
    return()=>{closed=true;render?.cancel();layer?.cancel();};
  },[document,number,width,zoom,csrf,id,title,pack,urlPath,canReadProgress,cache]);
  return <section className="space-y-3" data-testid="pdf-reader">
    <div className="flex flex-wrap gap-3 items-center"><button className="secondary-button" disabled={!document||number<=1} onClick={()=>setNumber(number-1)}>{t('document.previous')}</button>
      <label>{t('document.page')} <input aria-label={t('document.page')} type="number" min="1" max={document?.numPages||1} value={number} className="border rounded p-2 w-24" onChange={event=>{const next=Number(event.target.value);if(document&&Number.isInteger(next)&&next>=1&&next<=document.numPages)setNumber(next);}}/></label>
      <span>{t('document.of',{count:document?.numPages||'…'})}</span><button className="secondary-button" disabled={!document||number>=document.numPages} onClick={()=>setNumber(number+1)}>{t('document.next')}</button>
      <label>{t('document.zoom')} <select aria-label={t('document.zoom')} className="border rounded p-2" value={zoom} onChange={event=>setZoom(Number(event.target.value))}><option value="1">{t('document.fit')}</option>{[1.25,1.5,2].map(value=><option key={value} value={value}>{Math.round(value*100)}%</option>)}</select></label>
    </div>
    {loading&&<p role="status">{t('app.loading')}</p>}{failed&&<p role="alert">{t('document.failed')}</p>}{saveFailed&&<p role="alert">{t('reading.failed')}</p>}
    <div ref={area} className="overflow-auto max-w-full max-h-[80vh]" data-testid="pdf-area"><div className="relative w-max bg-white" style={{display:failed?'none':undefined}}><canvas ref={canvas} aria-label={t('document.canvas',{number})}/><div ref={text} className="textLayer" style={{position:'absolute',inset:0}}/></div></div>
  </section>;
}
