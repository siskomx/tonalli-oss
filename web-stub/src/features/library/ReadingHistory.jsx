import React,{useState} from 'react';
import {Link} from 'react-router-dom';
import {useQuery} from '@tanstack/react-query';
import {useApp} from '../../app/context.js';
import {readingHref,readingRequest} from './reading-client.js';
import {useMusic} from '../music/MusicPlayer.jsx';

export default function ReadingHistory(){
  const {csrf,t,beginNavigation}=useApp();const [failed,setFailed]=useState(false),[busy,setBusy]=useState(false);
  const media=useMusic();
  const history=useQuery({queryKey:['reading',csrf],staleTime:0,refetchOnMount:'always',queryFn:({signal})=>readingRequest(csrf,'GET',undefined,'',signal)});
  async function bookmark(entry){setBusy(true);setFailed(false);try{await readingRequest(csrf,'POST',{destination:entry.destination,bookmarked:!entry.bookmarked},'/bookmark');await history.refetch();}catch{setFailed(true);}finally{setBusy(false);}}
  return <section className="app-card my-5 space-y-3" data-testid="reading-history"><h3>{t('reading.heading')}</h3>
    {(failed||history.error)&&<p role="alert">{t('reading.failed')}</p>}
    {!history.isPending&&!history.data?.entries.length&&!history.error&&<p>{t('reading.empty')}</p>}
    <ul className="space-y-3">{history.data?.entries.map(entry=>{
      const unavailable=history.data.unavailable.some(destination=>JSON.stringify(destination)===JSON.stringify(entry.destination));
      const href=readingHref(entry.destination,media?.files);
      return <li key={JSON.stringify(entry.destination)} className="flex flex-wrap items-center gap-3">
        {unavailable?<span>{entry.title} — {t('reading.unavailable')}</span>:<Link className="underline break-words" to={href} onClick={event=>beginNavigation(href,event)}>{entry.title}</Link>}
        <button className="secondary-button" disabled={busy} aria-pressed={entry.bookmarked} onClick={()=>void bookmark(entry)}>{t(entry.bookmarked?'reading.unbookmark':'reading.bookmark')}</button>
      </li>;
    })}</ul>
  </section>;
}
