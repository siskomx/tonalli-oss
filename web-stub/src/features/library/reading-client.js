import {mediaHref} from './media-catalog.js';
let pending=Promise.resolve();
export function readingRequest(csrf,method='GET',body,suffix='',signal){
  // Keep a delayed old position from overwriting a newer one after navigation.
  const request=pending.then(()=>sendReadingRequest(csrf,method,body,suffix,signal));
  pending=request.catch(()=>{});return request;
}
async function sendReadingRequest(csrf,method,body,suffix,signal){
  const response=await fetch(`/api/reading${suffix}`,{method,signal,credentials:'same-origin',cache:'no-store',keepalive:method==='POST',
    headers:{'X-Tonalli-Csrf':csrf,...(body?{'Content-Type':'application/json'}:{})},...(body?{body:JSON.stringify(body)}:{})});
  if(!response.ok)throw Error('reading_unavailable');return response.json();
}
export function readingHref(destination,files=[]){
  if(destination.kind==='article')return `/article?${new URLSearchParams({pack:destination.pack,url:destination.url})}`;
  if(destination.kind==='map')return `/map?${new URLSearchParams({pack:destination.pack})}`;
  if(destination.kind==='media'){const file=files.find(file=>file.id===destination.id);return file?mediaHref(file):`/files?${new URLSearchParams({file:destination.id})}`;}
  return '/library';
}
