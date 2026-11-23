import React,{useEffect,useState} from 'react';
import {IconMovie} from '@tabler/icons-react';
import {useApp} from '../../app/context.js';
import {videoRequest} from './client.js';
export default function Poster({id}){
  const {csrf}=useApp(),[url,setUrl]=useState('');
  useEffect(()=>{
    setUrl('');if(!id)return;const controller=new AbortController();let closed=false;
    videoRequest(csrf,'poster',{id},controller.signal).then(value=>{if(!closed)setUrl(value.url);}).catch(()=>{});
    return()=>{closed=true;controller.abort();};
  },[id,csrf]);
  return <div className="aspect-video bg-black/10 flex items-center justify-center overflow-hidden rounded" data-testid="video-poster">
    {url?<img className="w-full h-full object-contain" alt="" src={url} loading="lazy" onError={()=>setUrl('')}/>:<IconMovie size={48} aria-hidden="true"/>}
  </div>;
}
