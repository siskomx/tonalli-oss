import React,{useEffect,useRef,useState} from 'react';
import {useApp} from '../../app/context.js';
import {bookResourceUrl,resolveBookLink,framePosition,restoreFramePosition} from './book-client.js';

export default function Epub({file,data,route,remember}){
  const {t}=useApp(),chapters=data.book.chapters;let saved;
  try{if(data.saved?.kind==='epub')saved=JSON.parse(data.saved.location);}catch{}
  const initial=Number.isInteger(saved?.chapter)&&saved.chapter>=0&&saved.chapter<chapters.length?saved.chapter:0;
  const[selection,setSelection]=useState({path:chapters[initial].path,index:initial,serial:0}),[failed,setFailed]=useState(false),[offline,setOffline]=useState(false),[loading,setLoading]=useState(true);
  const frame=useRef(null),cleanup=useRef(()=>{}),pending=useRef(null),timer=useRef(null),hash=useRef(''),positions=useRef(new Map([[chapters[initial].path,saved||{y:0}]])),generation=useRef(0),callbacks=useRef({route,remember});callbacks.current={route,remember};
  const {path,index}=selection;
  function flush(){clearTimeout(timer.current);const work=pending.current;pending.current=null;work?.();}
  function move(next,fragment='',chosen=chapters.findIndex(chapter=>chapter.path===next)){flush();cleanup.current();generation.current++;hash.current=fragment;setFailed(false);setOffline(false);setLoading(true);callbacks.current.route.markPending();setSelection(previous=>({path:next,index:chosen,serial:previous.serial+1}));}
  useEffect(()=>{window.addEventListener('pagehide',flush);return()=>{flush();generation.current++;cleanup.current();window.removeEventListener('pagehide',flush);};},[]);
  async function loaded(){
    const request=++generation.current;cleanup.current();
    try{
      const view=frame.current.contentWindow,doc=frame.current.contentDocument,current=new URL(view.location.href),expected=new URL(bookResourceUrl(data.base_url,path));
      if(current.origin!==expected.origin||current.pathname!==expected.pathname||!['text/html','application/xhtml+xml','image/svg+xml'].includes(doc.contentType))throw Error('book_frame_unavailable');
      const status=view.performance.getEntriesByType('navigation')[0]?.responseStatus;if(!Number.isInteger(status)||status<200||status>=300)throw Error('book_frame_unavailable');
      const style=doc.contentType==='image/svg+xml'?doc.createElementNS('http://www.w3.org/2000/svg','style'):doc.createElement('style');style.textContent='img,video,svg{max-width:100%;height:auto}body{overflow-wrap:anywhere}';(doc.head||doc.documentElement).append(style);
      const savedIndex=index;
      function record(){
        if(savedIndex<0||frame.current?.contentDocument!==doc)return;const value=framePosition(frame.current);positions.current.set(path,value);
        callbacks.current.remember({kind:'epub',location:JSON.stringify({chapter:savedIndex,...value})});
      }
      function scroll(){pending.current=record;clearTimeout(timer.current);timer.current=setTimeout(flush,600);}
      function click(event){
        const anchor=event.target.closest?.('a,area');if(!anchor)return;event.preventDefault();
        const href=anchor.getAttribute('href')||anchor.getAttributeNS('http://www.w3.org/1999/xlink','href');
        const target=resolveBookLink(href,view.location.href,data.base_url);
        if(target.kind==='fragment')view.location.hash=target.hash;else if(target.kind==='internal')move(target.path,target.hash);else setOffline(true);
      }
      doc.addEventListener('click',click,true);view.addEventListener('scroll',scroll,{passive:true});
      cleanup.current=()=>{doc.removeEventListener('click',click,true);view.removeEventListener('scroll',scroll);style.remove();};
      await doc.fonts?.ready;if(request!==generation.current||frame.current?.contentDocument!==doc)return;
      if(hash.current){view.location.hash=hash.current;hash.current='';}else restoreFramePosition(frame.current,positions.current.get(path));
      setLoading(false);setFailed(false);record();void callbacks.current.route.markReady({feature:'reader',book:'epub',chapter:savedIndex+1});
    }catch{if(request===generation.current){setLoading(false);setFailed(true);void callbacks.current.route.markReady({feature:'reader',unsupported:true});}}
  }
  return <section className="space-y-3" data-testid="epub-reader">
    <div className="flex gap-3 flex-wrap items-center"><button className="secondary-button" disabled={index<=0} onClick={()=>move(chapters[index-1].path,'',index-1)}>{t('book.previous')}</button>
      <label>{t('book.chapterLabel')} <select aria-label={t('book.chapterLabel')} className="border rounded p-2 max-w-full" value={index<0?'':index} onChange={event=>{const next=Number(event.target.value);move(chapters[next].path,'',next);}}>{index<0&&<option value="">{t('book.section')}</option>}{chapters.map((chapter,i)=><option key={i} value={i}>{t('book.chapter',{number:i+1})}</option>)}</select></label>
      <button className="secondary-button" disabled={index<0||index>=chapters.length-1} onClick={()=>move(chapters[index+1].path,'',index+1)}>{t('book.next')}</button>
    </div>
    {loading&&<p role="status">{t('app.loading')}</p>}{failed&&<p role="alert">{t('book.failed')}</p>}{offline&&<p role="status">{t('article.offline')}</p>}
    <iframe key={selection.serial} ref={frame} title={file.name} sandbox="allow-same-origin" src={bookResourceUrl(data.base_url,path)} onLoad={loaded} className="w-full border rounded bg-white" style={{height:'75vh',display:failed?'none':undefined}} data-testid="epub-frame"/>
  </section>;
}
