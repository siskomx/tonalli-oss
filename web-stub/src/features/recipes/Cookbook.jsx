import React,{useEffect,useRef,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {useQuery,useQueryClient} from '@tanstack/react-query';
import {useApp,useRouteReadiness} from '../../app/context.js';
import AdminSettings from '../../app/AdminSettings.jsx';
import {mediaCategory,mediaHref,searchMedia} from '../library/media-catalog.js';
import {readingRequest} from '../library/reading-client.js';
import {parseRecipe,recipePosition,recipeProgress} from './recipe.js';
import Input from '../../ui/Input.jsx';

export default function Cookbook(){
  const {csrf,t,beginNavigation}=useApp(),route=useRouteReadiness('cookbook'),[params,setParams]=useSearchParams(),cache=useQueryClient();
  const[query,setQuery]=useState(''),[page,setPage]=useState(0),[adding,setAdding]=useState(false);
  const files=useQuery({queryKey:['managed-media',csrf],staleTime:0,refetchOnMount:'always',queryFn:async({signal})=>{
    const response=await fetch('/api/files',{signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}});
    if(!response.ok)throw Error('files_unavailable');return(await response.json()).files;
  }});
  const recipes=(files.data||[]).filter(file=>mediaCategory(file.name)==='recipes'),selected=recipes.find(file=>file.id===params.get('file')),rows=searchMedia(recipes,query);
  useEffect(()=>{if(files.error)route.markError(files.error);else if(files.data&&!selected)void route.markReady({feature:'cookbook'});},[files.data,files.error,selected,route]);
  function saved(file){setAdding(false);setParams({file:file.id});void cache.invalidateQueries({queryKey:['managed-media',csrf]});}
  return <section className="space-y-5" data-testid="cookbook-page"><h2>{t('nav.cookbook')}</h2><p>{t('recipe.help')}</p>
    <div className="flex flex-wrap gap-3"><Link className="secondary-button" to="/files" onClick={event=>beginNavigation('/files',event)}>{t('recipe.import')}</Link><button className="secondary-button" onClick={()=>setAdding(true)}>{t('recipe.add')}</button></div>
    {adding&&<RecipeEditor onSaved={saved} onCancel={()=>setAdding(false)}/>}
    {files.isPending&&<p role="status">{t('app.loading')}</p>}{files.error&&<p role="alert">{t('files.error.files_unavailable')}</p>}
    {params.get('file')&&!selected&&files.data&&<p role="alert">{t('gallery.missing')}</p>}
    {selected&&!adding&&<Recipe key={`${selected.id}:${selected.version}`} file={selected} route={route} onSaved={saved}/>}
    <Input name="recipe-search" label={t('unified.search')} type="search" maxLength={512} value={query} onChange={event=>{setQuery(event.target.value);setPage(0);}}/>
    {!files.isPending&&!rows.length&&<p>{t('recipe.empty')}</p>}
    <ul className="space-y-3">{rows.slice(page*100,page*100+100).map(file=><li key={file.id}><Link className="underline break-words" to={mediaHref(file)} onClick={event=>{setAdding(false);beginNavigation(mediaHref(file),event);}}>{file.name}</Link><p className="text-sm break-words">{file.folder}</p></li>)}</ul>
    {rows.length>100&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*100>=rows.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
  </section>;
}
function Recipe({file,route,onSaved}){
  const{csrf,t,beginNavigation}=useApp(),cache=useQueryClient(),callbacks=useRef(route);callbacks.current=route;
  const[data,setData]=useState(null),[error,setError]=useState(false),[saveError,setSaveError]=useState(false),[editing,setEditing]=useState(false),[retry,setRetry]=useState(0);
  const mounted=useRef(false),latest=useRef(0);
  useEffect(()=>{
    let closed=false;mounted.current=true;const controller=new AbortController();setError(false);setData(null);
    async function open(){
      const [response,history]=await Promise.all([fetch(`/api/files/${file.id}/text`,{signal:controller.signal,credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf}}),readingRequest(csrf,'GET',undefined,'',controller.signal)]);
      if(!response.ok)throw Error('recipe_failed');const value=await response.json(),recipe=parseRecipe(value.text);
      const saved=history.entries.find(entry=>entry.destination.kind==='media'&&entry.destination.id===file.id)?.position;
      if(!closed){setData({...value,recipe,checks:recipeProgress(saved,value.file.sha256,recipe)});void callbacks.current.markReady({feature:'cookbook'});}
    }
    void open().catch(failure=>{if(!closed&&failure.name!=='AbortError'){setError(true);void callbacks.current.markReady({feature:'cookbook',unsupported:true});}});
    return()=>{closed=true;mounted.current=false;controller.abort();};
  },[csrf,file.id,file.version,retry]);
  function save(checks){
    const serial=++latest.current;setData(current=>({...current,checks}));
    void readingRequest(csrf,'POST',{destination:{kind:'media',id:file.id},position:recipePosition(data.file.sha256,checks.ingredients,checks.steps),title:file.name,bookmarked:false})
      .then(()=>{if(mounted.current&&latest.current===serial)setSaveError(false);void cache.invalidateQueries({queryKey:['reading',csrf]});})
      .catch(()=>{if(mounted.current&&latest.current===serial)setSaveError(true);});
  }
  function toggle(kind,index){const values=data.checks[kind];save({...data.checks,[kind]:values.includes(index)?values.filter(value=>value!==index):[...values,index]});}
  return <section className="app-card space-y-4" data-testid="recipe-viewer"><h3 className="break-words">{file.name}</h3>
    <Link className="underline" to={`/file?file=${file.id}`} onClick={event=>beginNavigation(`/file?file=${file.id}`,event)}>{t('gallery.file')}</Link>
    {error?<><p role="alert">{t('recipe.failed')}</p><button className="secondary-button" onClick={()=>setRetry(value=>value+1)}>{t('files.refresh')}</button></>:!data?<p role="status">{t('app.loading')}</p>:<>
      <p>{t('recipe.private')}</p>
      {data.recipe.notes.map((note,index)=><p key={index} className="whitespace-pre-wrap break-words">{note}</p>)}
      {['ingredients','steps'].map(kind=><fieldset key={kind} className="space-y-3" data-testid={`recipe-${kind}`}><legend className="font-bold">{t(`recipe.${kind}`)}</legend>
        {data.recipe[kind].map((label,index)=><label key={index} className="flex items-start gap-3"><input type="checkbox" checked={data.checks[kind].includes(index)} onChange={()=>toggle(kind,index)}/><span className="whitespace-pre-wrap break-words">{label}</span></label>)}
      </fieldset>)}
      <button className="secondary-button" onClick={()=>save({ingredients:[],steps:[]})}>{t('recipe.reset')}</button>
      {saveError&&<div role="alert"><p>{t('reading.failed')}</p><button className="secondary-button" onClick={()=>save(data.checks)}>{t('recipe.retry')}</button></div>}
      <details><summary>{t('recipe.source')}</summary><pre className="whitespace-pre-wrap break-words max-h-96 overflow-auto">{data.text}</pre></details>
      <button className="secondary-button" onClick={()=>setEditing(true)}>{t('recipe.edit')}</button>
      {editing&&<RecipeEditor initial={data} onSaved={onSaved} onCancel={()=>setEditing(false)}/>}
    </>}
  </section>;
}
function RecipeEditor({initial,onSaved,onCancel}){
  return <AdminSettings>{({token,onLocked})=><Editor initial={initial} token={token} onLocked={onLocked} onSaved={onSaved} onCancel={onCancel}/>}</AdminSettings>;
}
function Editor({initial,token,onLocked,onSaved,onCancel}){
  const{csrf,t}=useApp(),[name,setName]=useState(''),[text,setText]=useState(initial?.text||t('recipe.example')),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const mounted=useRef(true);useEffect(()=>()=>{mounted.current=false;},[]);
  async function submit(event){
    event.preventDefault();setBusy(true);setError('');
    try{
      parseRecipe(text);
      if(!initial&&(!name.trim()||/[\\/\x00-\x1f]/.test(name)||name.trim().length>230))throw Error('name');
      const response=await fetch(initial?`/api/files/${initial.file.id}/text`:`/api/files/upload?${new URLSearchParams({name:`${name.trim().replace(/\.cook$/i,'')}.cook`,folder:'Recipes'})}`,{
        method:initial?'PUT':'POST',credentials:'same-origin',cache:'no-store',headers:{'X-Tonalli-Csrf':csrf,'X-Tonalli-Admin':token,'Content-Type':initial?'application/json':'application/octet-stream'},body:initial?JSON.stringify({version:initial.file.version,text}):new TextEncoder().encode(text)});
      if(!response.ok){if(response.status===423)onLocked();throw Error(response.status===409?'changed':'save');}
      const result=await response.json();if(mounted.current)onSaved(result.file||result);
    }catch(failure){if(mounted.current)setError(failure.message==='changed'?'recipe.changed':'recipe.saveFailed');}finally{if(mounted.current)setBusy(false);}
  }
  return <form className="space-y-3" onSubmit={submit} data-testid="recipe-editor"><h3>{t(initial?'recipe.edit':'recipe.add')}</h3><p>{t('recipe.editorHelp')}</p>
    {!initial&&<label className="block">{t('recipe.name')}<input className="block border rounded p-2 w-full" required maxLength={230} value={name} disabled={!token||busy} onChange={event=>setName(event.target.value)}/></label>}
    <label className="block" htmlFor="recipe-source">{t('recipe.source')}</label><textarea id="recipe-source" className="block border rounded p-2 w-full min-h-64" maxLength={65536} value={text} disabled={!token||busy} onChange={event=>setText(event.target.value)}/>
    {error&&<p role="alert">{t(error)}</p>}<button className="chat-control" disabled={!token||busy}>{t('recipe.save')}</button><button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>{t('files.cancel')}</button>
  </form>;
}
