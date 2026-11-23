import React, {useCallback,useEffect,useRef,useState} from 'react';
import {useApp,useRouteReadiness} from '../../app/context.js';
import AdminSettings from '../../app/AdminSettings.jsx';
import {nativeShell} from '../../app/AppLayout.jsx';
import FileUploader from './FileUploader.jsx';
import {formatBytes} from '../content/format.js';
import {importFolder} from './folders.js';
import {useSearchParams} from 'react-router-dom';
import {readingRequest} from '../library/reading-client.js';
import {useQueryClient} from '@tanstack/react-query';
import {Link} from 'react-router-dom';

export default function Files() {
  const {t}=useApp();
  return <section className="space-y-6"><h2>{t('files.heading')}</h2><p>{t('files.help')}</p>
    <AdminSettings>{({token,onLocked})=><FileControls token={token} onLocked={onLocked}/>}</AdminSettings>
  </section>;
}
function FileControls({token,onLocked}) {
  const queryClient=useQueryClient();
  const [search]=useSearchParams();const requested=search.get('file');const opened=useRef(null);
  const {csrf,t,language,beginNavigation}=useApp();const {markReady}=useRouteReadiness('files');
  const [rows,setRows]=useState([]),[files,setFiles]=useState([]),[folder,setFolder]=useState(''),[filter,setFilter]=useState('');
  const [busy,setBusy]=useState(false),[progress,setProgress]=useState(0),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const [editing,setEditing]=useState(null),[rename,setRename]=useState(null),[removing,setRemoving]=useState(null),[page,setPage]=useState(0);
  const [nativeTicket,setNativeTicket]=useState('');
  const [loaded,setLoaded]=useState(false);
  const [historyError,setHistoryError]=useState(false);
  const android=nativeShell()&&/Android/.test(navigator.userAgent);
  const active=useRef(false),request=useRef(null),uploader=useRef(null),cancelled=useRef(false),confirmation=useRef(null);
  const selected=useCallback((value,paths)=>setFiles(Array.from(value).map(file=>({file,path:paths?.get(file)||file.webkitRelativePath||''}))),[]);
  async function api(path='',method='GET',body) {
    const response=await fetch(`/api/files${path}`,{method,credentials:'same-origin',cache:'no-store',signal:request.current?.signal,
      headers:{'X-Tonalli-Csrf':csrf,...(token?{'X-Tonalli-Admin':token}:{}),...(body?{'Content-Type':'application/json'}:{})},...(body?{body:JSON.stringify(body)}:{})});
    if(!response.ok){if(response.status===423)onLocked();const value=await response.json().catch(()=>({}));throw new Error(value.error||'files_unavailable');}
    return response.json();
  }
  async function refresh(){const value=await api();if(active.current){setRows(value.files);setPage(0);void queryClient.invalidateQueries({queryKey:['managed-media',csrf]});}}
  useEffect(()=>{
    active.current=true;request.current=new AbortController();
    refresh().catch(()=>{if(active.current)setError('files_unavailable');}).finally(()=>{if(active.current)setLoaded(true);});
    function resumed(){if(android&&!document.hidden){setNativeTicket('');void refresh().catch(()=>{if(active.current)setError('files_unavailable');});}}
    document.addEventListener('visibilitychange',resumed);
    return()=>{document.removeEventListener('visibilitychange',resumed);active.current=false;cancelled.current=true;request.current?.abort();request.current?.xhr?.abort();};
  },[csrf,android]);
  useEffect(()=>{if(loaded)void markReady();},[loaded,markReady]);
  useEffect(()=>{
    if(!loaded||!requested||opened.current===requested)return;
    opened.current=requested;const file=rows.find(row=>row.id===requested);
    if(!file){setError('file_missing');return;}
    setFilter(file.folder);setPage(Math.floor(rows.filter(row=>!file.folder||row.folder===file.folder).findIndex(row=>row.id===requested)/100));
    if(file.bytes<=65536&&/\.(txt|md|csv|json|toml|ya?ml|log|cook)$/i.test(file.name))void perform(()=>openText(file));
  },[loaded,requested,rows]);
  useEffect(()=>{if(removing&&!confirmation.current?.open)confirmation.current?.showModal();else if(!removing)confirmation.current?.close();},[removing]);
  useEffect(()=>{if(!token){setFiles([]);setNativeTicket('');cancelled.current=true;request.current?.xhr?.abort();}},[token]);
  async function perform(operation){setBusy(true);setError('');setNotice('');try{await operation();}catch(failure){if(active.current&&failure.name!=='AbortError')setError(failure.message);}finally{if(active.current)setBusy(false);}}
  async function openText(row){const value=await api(`/${row.id}/text`);if(active.current){setEditing(value);setRename(null);void readingRequest(csrf,'POST',{destination:{kind:'media',id:row.id},position:{kind:'start'},title:row.name,bookmarked:false}).then(()=>queryClient.invalidateQueries({queryKey:['reading',csrf]})).catch(()=>{if(active.current)setHistoryError(true);});}}
  function upload(file,folder) {
    return new Promise((resolve,reject)=>{
      const xhr=new XMLHttpRequest();request.current.xhr=xhr;
      xhr.open('POST',`/api/files/upload?${new URLSearchParams({name:file.name,folder})}`);xhr.withCredentials=true;
      xhr.setRequestHeader('X-Tonalli-Csrf',csrf);xhr.setRequestHeader('X-Tonalli-Admin',token);xhr.setRequestHeader('Content-Type','application/octet-stream');
      xhr.upload.onprogress=event=>{if(active.current&&event.lengthComputable)setProgress(Math.round(event.loaded/event.total*100));};
      xhr.onload=()=>{request.current.xhr=null;if(xhr.status===200)resolve();else{if(xhr.status===423)onLocked();let code='files_unavailable';try{code=JSON.parse(xhr.responseText).error||code;}catch{}reject(new Error(code));}};
      xhr.onerror=()=>reject(new Error('files_unavailable'));xhr.onabort=()=>reject(new DOMException('Cancelled','AbortError'));xhr.send(file);
    });
  }
  function importFiles(event){event.preventDefault();const batch=files;cancelled.current=false;
    void perform(async()=>{let added=0;
      try{for(const selected of batch){if(cancelled.current)break;const file=selected.file;setProgress(0);await upload(file,importFolder(folder,selected.path,file.name));added++;uploader.current?.remove(file);}}finally{await refresh();if(active.current)setNotice(t('files.added',{count:added}));}
    });
  }
  const visible=rows.filter(row=>!filter||row.folder===filter);const pageRows=visible.slice(page*100,page*100+100);
  const editable=row=>row.bytes<=65536&&/\.(txt|md|csv|json|toml|ya?ml|log|cook)$/i.test(row.name);
  return <div className="space-y-5" data-testid="managed-files">
    {historyError&&<p role="alert">{t('reading.failed')}</p>}
    {error&&<p role="alert">{t(`files.error.${error}`)===`files.error.${error}`?t('files.error.files_unavailable'):t(`files.error.${error}`)}</p>}
    {notice&&<p role="status">{notice}</p>}
    {token&&<form onSubmit={android?event=>{event.preventDefault();void perform(async()=>{const reply=await api('/native-import','POST',{folder});if(active.current)setNativeTicket(reply.ticket);});}:importFiles} className="space-y-3">
      <h3>{t('files.add')}</h3><p>{t(android?'files.nativeHelp':'files.copyHelp')}</p>{!android&&<FileUploader ref={uploader} onUpload={selected} disabled={busy}/>}
      <label className="block">{t('files.folder')}<input className="block border rounded p-2 w-full" value={folder} maxLength={1024} disabled={busy} onChange={event=>{setFolder(event.target.value);setNativeTicket('');}}/></label>
      <button className="chat-control" disabled={busy||(!android&&!files.length)}>{t(android?'files.prepareNative':'files.import')}</button>
      {android&&nativeTicket&&<a className="secondary-button" href={`/native/import-media/${nativeTicket}`}>{t('files.chooseNative')}</a>}
      {busy&&<><progress aria-label={t('files.progress')} max={100} value={progress}/><button type="button" className="secondary-button" onClick={()=>{cancelled.current=true;request.current.xhr?.abort();}}>{t('files.cancel')}</button></>}
    </form>}
    <div className="flex flex-wrap gap-3 items-end"><label htmlFor="files-folder">{t('files.browseFolder')}</label><select id="files-folder" className="border rounded p-2" value={filter} onChange={event=>{setFilter(event.target.value);setPage(0);}}>
      <option value="">{t('files.allFolders')}</option>{[...new Set(rows.map(row=>row.folder).filter(Boolean))].sort().map(name=><option key={name} value={name}>{name}</option>)}
    </select><button className="secondary-button" disabled={busy} onClick={()=>void perform(refresh)}>{t('files.refresh')}</button></div>
    {!visible.length?<p>{t('files.empty')}</p>:<ul className="space-y-3">{pageRows.map(row=><li className="rounded border border-border-default p-3 space-y-2" key={row.id} data-testid="managed-file" data-file-id={row.id}>
      <h3 className="break-words">{row.name}</h3><p className="break-words">{row.folder} {formatBytes(row.bytes,language)}</p>
      <div className="flex flex-wrap gap-2">
        <Link className="secondary-button" to={`/file?${new URLSearchParams({file:row.id})}`} onClick={event=>beginNavigation(`/file?${new URLSearchParams({file:row.id})}`,event)}>{t('file.open')}</Link>
        {nativeShell()&&<a className="secondary-button" href={`/native/export-media/${row.id}`}>{t('files.download')}</a>}
        {editable(row)&&<button className="secondary-button" disabled={busy} onClick={()=>void perform(()=>openText(row))}>{t('files.viewText')}</button>}
        {token&&<><button className="secondary-button" disabled={busy} onClick={()=>{setRename({...row});setEditing(null);}}>{t('files.rename')}</button><button className="secondary-button" disabled={busy} onClick={()=>setRemoving(row)}>{t('files.delete')}</button></>}
      </div>
    </li>)}</ul>}
    {visible.length>100&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*100>=visible.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
    {rename&&<form className="space-y-3" onSubmit={event=>{event.preventDefault();void perform(async()=>{await api(`/${rename.id}/rename`,'POST',{version:rename.version,name:rename.name});if(active.current)setRename(null);await refresh();});}}>
      <label className="block">{t('files.newName')}<input className="block border rounded p-2 w-full" required maxLength={240} value={rename.name} onChange={event=>setRename({...rename,name:event.target.value})}/></label>
      <button className="chat-control" disabled={busy||!token}>{t('files.saveName')}</button><button type="button" className="secondary-button" onClick={()=>setRename(null)}>{t('files.cancel')}</button>
    </form>}
    {editing&&<form className="space-y-3" onSubmit={event=>{event.preventDefault();void perform(async()=>{await api(`/${editing.file.id}/text`,'PUT',{version:editing.file.version,text:editing.text});if(active.current)setEditing(null);await refresh();});}}>
      <h3>{editing.file.name}</h3><p>{t('files.textLimit')}</p><label className="block" htmlFor="file-text">{t('files.text')}</label><textarea id="file-text" className="block border rounded p-2 w-full min-h-64" value={editing.text} maxLength={65536} readOnly={!token} onChange={event=>setEditing({...editing,text:event.target.value})}/>
      {token&&<button className="chat-control" disabled={busy}>{t('files.saveText')}</button>}<button type="button" className="secondary-button" onClick={()=>setEditing(null)}>{t('files.close')}</button>
    </form>}
    <dialog ref={confirmation} className="app-card max-w-lg" onCancel={()=>setRemoving(null)} aria-labelledby="file-delete-heading">
      <h3 id="file-delete-heading">{t('files.confirmDelete')}</h3><p className="break-words my-4">{removing?.name}</p><p>{t('files.deleteHelp')}</p>
      <div className="flex flex-wrap gap-3 my-4"><button className="secondary-button" disabled={busy} onClick={()=>setRemoving(null)}>{t('files.cancel')}</button><button className="chat-control" disabled={busy||!token} onClick={()=>void perform(async()=>{await api(`/${removing.id}`,'DELETE',{version:removing.version,confirmed:true});if(active.current)setRemoving(null);await refresh();})}>{t('files.confirmRemove')}</button></div>
    </dialog>
  </div>;
}
