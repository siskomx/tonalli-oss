/* Adapted from Project NOMAD admin/inertia/components/file-uploader/index.tsx
 * at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e. Copyright 2024-2026 Crosstalk Solutions LLC.
 * Apache-2.0. Modified: JSX, local-only plugins, EN/ES/device theme, owned cleanup,
 * selected-file change filtering and removal of successfully imported files. */
import React, { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import Uppy from '@uppy/core';
import '@uppy/core/css/style.min.css';
import '@uppy/dashboard/css/style.min.css';
import { useUppyEvent } from '@uppy/react';
import Dashboard from '@uppy/react/dashboard';
import es from '@uppy/locales/lib/es_ES';
import en from '@uppy/locales/lib/en_US';
import { useApp } from '../../app/context.js';
import './uploader.css';

const FileUploader = forwardRef(function FileUploader({onUpload,disabled},ref) {
  const {language,preferences}=useApp();
  const previous=useRef([]);
  const [uppy]=useState(()=>new Uppy({debug:false,restrictions:{maxFileSize:64*1024**3,minNumberOfFiles:0,maxNumberOfFiles:100}}));
  useImperativeHandle(ref,()=>({clear:()=>uppy.clear(),remove:(file)=>uppy.removeFiles(uppy.getFiles().filter(row=>row.data===file).map(row=>row.id))}),[uppy]);
  useEffect(()=>()=>uppy.destroy(),[uppy]);
  useUppyEvent(uppy,'state-update',(_,state)=>{
    const files=Object.values(state.files);
    if(files.length===previous.current.length&&files.every((file,index)=>file.id===previous.current[index]))return;
    previous.current=files.map(file=>file.id);
    const transfer=new DataTransfer();
    files.forEach(file=>{if(file.data instanceof File)transfer.items.add(file.data);else if(file.data instanceof Blob)transfer.items.add(new File([file.data],file.name||`${crypto.randomUUID()}.${file.extension}`,{type:file.type,lastModified:Date.now()}));});
    onUpload(transfer.files,new Map(files.map(file=>[file.data,file.meta.relativePath||''])));
  });
  const dark=preferences?.theme==='dark'||(preferences?.theme!=='light'&&matchMedia('(prefers-color-scheme: dark)').matches);
  return <Dashboard uppy={uppy} width="100%" height={250} hideUploadButton proudlyDisplayPoweredByUppy={false} fileManagerSelectionType="both"
    disabled={disabled} locale={language==='es'?es:en} theme={dark?'dark':'light'} />;
});
export default FileUploader;
