import React,{useEffect,useLayoutEffect,useRef,useState} from 'react';
import {useVirtualizer} from '@tanstack/react-virtual';
import {useApp} from '../../app/context.js';
import {bookResourceUrl} from './book-client.js';

function initialPosition(saved,count){
  try{
    if(saved?.kind==='epub'){
      const value=JSON.parse(saved.location);
      if(value.reader==='comic'&&Number.isInteger(value.page)&&value.page>=1&&value.page<=count&&Number.isFinite(value.ratio)&&value.ratio>=0&&value.ratio<=1)return{page:value.page,ratio:value.ratio,mode:'scroll'};
    }
  }catch{}
  return{page:saved?.kind==='page'&&saved.number>=1&&saved.number<=count?saved.number:1,ratio:0,mode:'page'};
}
export default function Comic({data,route,remember}){
  const {t}=useApp(),pages=data.book.chapters,start=initialPosition(data.saved,pages.length);
  const[page,setPage]=useState(start.page),[mode,setMode]=useState(start.mode),[jump,setJump]=useState({...start,serial:0}),[failed,setFailed]=useState(false),[loading,setLoading]=useState(true);
  const callbacks=useRef({page,mode,route,remember});callbacks.current={page,mode,route,remember};
  function move(number){setPage(number);setJump(current=>({page:number,ratio:0,serial:current.serial+1}));setFailed(false);setLoading(true);route.markPending();}
  function changeMode(value){setMode(value);setJump(current=>({page,ratio:0,serial:current.serial+1}));setFailed(false);setLoading(true);route.markPending();}
  function ready(number,bad=false){
    if(callbacks.current.mode!=='page'||callbacks.current.page!==number)return;setLoading(false);setFailed(bad);
    if(!bad)callbacks.current.remember({kind:'page',number});void callbacks.current.route.markReady({feature:'reader',book:'comic',page:number,unsupported:bad});
  }
  return <section className="space-y-3" data-testid="comic-reader">
    <div className="flex flex-wrap gap-3 items-center"><button className="secondary-button" disabled={page<=1} onClick={()=>move(page-1)}>{t('document.previous')}</button>
      <label>{t('document.page')} <input aria-label={t('document.page')} className="w-24 border rounded p-2" type="number" min="1" max={pages.length} value={page} onChange={event=>{const value=Number(event.target.value);if(Number.isInteger(value)&&value>=1&&value<=pages.length)move(value);}}/></label><span>{t('document.of',{count:pages.length})}</span>
      <button className="secondary-button" disabled={page>=pages.length} onClick={()=>move(page+1)}>{t('document.next')}</button>
      <label>{t('book.mode')} <select aria-label={t('book.mode')} className="border rounded p-2" value={mode} onChange={event=>changeMode(event.target.value)}><option value="page">{t('book.pages')}</option><option value="scroll">{t('book.webtoon')}</option></select></label>
    </div>
    {mode==='page'?<>{loading&&<p role="status">{t('app.loading')}</p>}{failed&&<p role="alert">{t('book.imageFailed')}</p>}
      <img key={page} src={bookResourceUrl(data.base_url,pages[page-1].path)} alt={t('document.canvas',{number:page})} className="max-w-full max-h-[75vh] object-contain mx-auto" style={{display:failed?'none':undefined}} onLoad={()=>ready(page)} onError={()=>ready(page,true)} data-testid="comic-image"/>
    </>:<Webtoon data={data} jump={jump} route={route} onPage={setPage} remember={remember}/>}
  </section>;
}

function Webtoon({data,jump,route,onPage,remember}){
  const {t}=useApp(),host=useRef(null),nodes=useRef(new Map()),loaded=useRef(new Set()),failed=useRef(new Set()),pendingRestore=useRef(null),serial=useRef(0),lastJump=useRef(-1),last=useRef({page:jump.page,ratio:jump.ratio||0}),timer=useRef(null),pending=useRef(null),mounted=useRef(true),callbacks=useRef({route,onPage,remember});callbacks.current={route,onPage,remember};
  const[width,setWidth]=useState(600),[version,setVersion]=useState(0),[restored,setRestored]=useState(false);
  const list=useVirtualizer({count:data.book.chapters.length,getScrollElement:()=>host.current,estimateSize:()=>Math.max(200,width*1.4),overscan:1});
  function flush(){clearTimeout(timer.current);const value=pending.current;pending.current=null;if(value)callbacks.current.remember({kind:'epub',location:JSON.stringify({reader:'comic',...value})});}
  function restoreIfLoaded(){
    const target=pendingRestore.current;if(!target)return;const node=nodes.current.get(target.page-1),image=node?.querySelector('img');
    if(!image?.complete||!image.naturalWidth)return;
    const ticket=serial.current;list.measureElement(node);const item=list.getVirtualItems().find(item=>item.index===target.page-1);if(!item)return;
    // The virtual item's cached size may still be the estimate in this load callback.
    list.scrollToOffset(item.start+node.getBoundingClientRect().height*target.ratio);pendingRestore.current=null;
    requestAnimationFrame(()=>requestAnimationFrame(()=>{if(mounted.current&&serial.current===ticket)setRestored(true);}));
  }
  useEffect(()=>{const observer=new ResizeObserver(entries=>{if(entries[0].contentRect.width>0)setWidth(entries[0].contentRect.width);});observer.observe(host.current);return()=>observer.disconnect();},[]);
  useLayoutEffect(()=>{
    const target=lastJump.current===jump.serial?last.current:{page:jump.page,ratio:jump.ratio||0};lastJump.current=jump.serial;
    serial.current++;setRestored(false);pendingRestore.current=target;list.measure();list.scrollToIndex(target.page-1,{align:'start'});
    const frame=requestAnimationFrame(restoreIfLoaded);return()=>cancelAnimationFrame(frame);
  },[jump.serial,width,list]);
  const items=list.getVirtualItems(),current=items.find(item=>item.end>(list.scrollOffset||0)+1),index=current?.index;
  useEffect(()=>{
    if(!restored||index===undefined||(!loaded.current.has(index)&&!failed.current.has(index)))return;
    const number=index+1;callbacks.current.onPage(number);
    if(!failed.current.has(index)){
      const value={page:number,ratio:Math.max(0,Math.min(1,((list.scrollOffset||0)-current.start)/Math.max(1,current.size)))};value.ratio=Number(value.ratio.toFixed(6));last.current=value;pending.current=value;clearTimeout(timer.current);timer.current=setTimeout(flush,600);
    }
    void callbacks.current.route.markReady({feature:'reader',book:'webtoon',page:number,unsupported:failed.current.has(index)});
  },[index,version,restored,list.scrollOffset]);
  useEffect(()=>{mounted.current=true;window.addEventListener('pagehide',flush);return()=>{mounted.current=false;serial.current++;flush();window.removeEventListener('pagehide',flush);};},[]);
  function loadedImage(index){loaded.current.add(index);const node=nodes.current.get(index);if(node)list.measureElement(node);restoreIfLoaded();setVersion(value=>value+1);}
  function failedImage(index){failed.current.add(index);if(pendingRestore.current?.page===index+1){pendingRestore.current=null;setRestored(true);}setVersion(value=>value+1);}
  function userScroll(){if(pendingRestore.current){pendingRestore.current=null;serial.current++;setRestored(true);}}
  return <div ref={host} className="overflow-auto border rounded bg-white" style={{height:'70vh'}} onWheel={userScroll} onTouchStart={userScroll} data-testid="webtoon-scroll">
    <div className="relative w-full" style={{height:list.getTotalSize()}}>{items.map(item=><div key={item.key} data-index={item.index} ref={node=>{if(node)nodes.current.set(item.index,node);else nodes.current.delete(item.index);list.measureElement(node);}} className="absolute top-0 left-0 w-full" style={{transform:`translateY(${item.start}px)`}}>
      {failed.current.has(item.index)?<p role="alert" className="p-4">{t('book.imageFailed')}</p>:<img src={bookResourceUrl(data.base_url,data.book.chapters[item.index].path)} alt={t('document.canvas',{number:item.index+1})} width="1000" height="1400" className="block w-full h-auto" onLoad={()=>loadedImage(item.index)} onError={()=>failedImage(item.index)}/>}
    </div>)}</div>
  </div>;
}
