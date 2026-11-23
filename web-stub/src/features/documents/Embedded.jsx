import React,{useState} from 'react';
import {Link} from 'react-router-dom';
import {useQueryClient} from '@tanstack/react-query';
import {useApp} from '../../app/context.js';
import AdminSettings from '../../app/AdminSettings.jsx';
import Pdf from './Pdf.jsx';
import Book from './Book.jsx';

export default function Embedded({article,route,onBack}){
  const {t,canManageMaps}=useApp(),[saving,setSaving]=useState(false);
  return <section className="space-y-4" data-testid="embedded-document">
    {onBack&&<button className="secondary-button" onClick={onBack}>{t('article.back')}</button>}
    <p className="text-sm">{t('article.documentLimit')}</p>
    {article.kind==='pdf'?<Pdf article={article} route={route}/>:<Book article={article} route={route}/>}
    {canManageMaps&&<button className="secondary-button" onClick={()=>setSaving(value=>!value)}>{t('article.saveDocument')}</button>}
    {saving&&<AdminSettings>{({token,onLocked})=><SaveDocument article={article} token={token} onLocked={onLocked}/>}</AdminSettings>}
  </section>;
}
function SaveDocument({article,token,onLocked}){
  const {csrf,t,beginNavigation}=useApp(),cache=useQueryClient(),[busy,setBusy]=useState(false),[failed,setFailed]=useState(false),[file,setFile]=useState(null);
  async function save(){
    setBusy(true);setFailed(false);
    try{
      const response=await fetch('/api/documents/import',{method:'POST',credentials:'same-origin',cache:'no-store',headers:{'Content-Type':'application/json','X-Tonalli-Csrf':csrf,'X-Tonalli-Admin':token},body:JSON.stringify({pack:article.pack,url:article.url})});
      if(response.status===423)onLocked();if(!response.ok)throw Error('document_copy_unavailable');
      setFile(await response.json());await cache.invalidateQueries({queryKey:['managed-media',csrf]});
    }catch{setFailed(true);}finally{setBusy(false);}
  }
  const href=file?`/file?file=${file.id}`:'';
  return <div className="space-y-3">{failed&&<p role="alert">{t('article.copyFailed')}</p>}{file?<>
    <p role="status">{t('article.copied')}</p><Link className="secondary-button" to={href} onClick={event=>beginNavigation(href,event)}>{t('gallery.file')}</Link>
  </>:<><p>{t('article.copyHelp')}</p><button className="secondary-button" disabled={!token||busy} onClick={()=>void save()}>{t('article.copy')}</button></>}</div>;
}
