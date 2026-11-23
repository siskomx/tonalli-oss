// Adapted from Project NOMAD CountryPickerModal.tsx, Apache-2.0.
// Copyright 2024-2026 Crosstalk Solutions LLC.
// Changes: one native-owned country job, EN/ES, standalone preflight and job controls.
import React, { useEffect, useRef, useState } from 'react';
import { IconCheck } from '@tabler/icons-react';
import classNames from '../../ui/classNames.js';
import { useApp } from '../../app/context.js';
import { formatBytes } from '../content/format.js';

const countries = ['mx', 'us', 'ca'];
const active = state => ['queued','checking','downloading','verifying','pausing','cancelling','finishing'].includes(state);
export default function CountryPicker({ onInstalled }) {
  const { csrf, t } = useApp();
  const [selected, setSelected] = useState('mx');
  const [zoom, setZoom] = useState(12);
  const [rows, setRows] = useState([]);
  const [imports, setImports] = useState([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(null);
  const callbacks = useRef(onInstalled);
  callbacks.current = onInstalled;
  const row = rows.find(row => row.id === selected);
  const busy = pending || rows.some(row => active(row.state));
  async function request(method = 'GET', suffix = '', body, signal) {
    const response = await fetch(`/api/map-management${suffix}`, { method, signal, credentials:'same-origin', cache:'no-store',
      headers:{'X-Tonalli-Csrf':csrf, ...(body ? {'Content-Type':'application/json'} : {})}, ...(body ? {body:JSON.stringify(body)} : {}) });
    if (!response.ok) throw new Error([401,403].includes(response.status) ? 'session' : 'failed');
    if (method !== 'GET') return;
    const data = await response.json();
    if (!Array.isArray(data.rows) || data.rows.length > 3 || data.rows.some(row => !countries.includes(row.id) || typeof row.state !== 'string')) throw new Error('failed');
    if (!Array.isArray(data.imports) || data.imports.length > 32 || data.imports.some(row => !/^mapfile-[a-f0-9]{64}$/.test(row.id) || typeof row.title !== 'string')) throw new Error('failed');
    if (!signal?.aborted) setImports(data.imports);
    return data;
  }
  useEffect(() => {
    const controller = new AbortController();
    let timer;
    async function refresh() {
      try {
        const next = await request('GET', '', null, controller.signal);
        if (controller.signal.aborted) return;
        setRows(next.rows);
        const identity = JSON.stringify([next.rows.map(row => [row.id, row.installed?.generation]),next.imports.map(row=>row.id)]);
        if (generation.current === null) setZoom(next.rows.find(row=>row.id===selected)?.zoom ?? 12);
        if (identity !== generation.current) callbacks.current?.(next.imports);
        generation.current = identity;
      } catch (error) { if (!controller.signal.aborted) setError(error.message); }
      if (!controller.signal.aborted) timer = setTimeout(refresh, 1200);
    }
    void refresh();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [csrf]);
  async function action(estimate, control) {
    setPending(true); setError('');
    try {
      await request('POST', control ? `/${selected}/${control}` : '', control ? null : {id:selected,zoom:!estimate && row?.state==='paused' ? row.zoom : zoom,estimate});
      setRows((await request()).rows);
    } catch (error) { setError(error.message); }
    finally { setPending(false); }
  }
  const checked = row?.zoom === zoom && ['ready','paused','failed'].includes(row?.state) && row.downloadBytes > 0;
  return <details className="my-4 rounded border border-border-default p-4" open>
    <summary className="cursor-pointer font-semibold">{t('map.downloadCountries')}</summary>
    <p className="my-2 text-sm">{t('map.downloadHelp')}</p>
    <ul className="my-3 border border-border-default rounded">
      {countries.map(code => {
        const isSelected = selected === code;
        const isInstalled = rows.some(row => row.id === code && row.installed);
        return <li key={code}><button type="button" aria-pressed={isSelected} onClick={() => {setSelected(code);setZoom(rows.find(row=>row.id===code)?.zoom ?? 12);}}
          className={classNames('w-full flex items-center gap-3 px-4 py-2 text-left text-sm transition-colors cursor-pointer', isSelected ? 'bg-desert-green/10 hover:bg-desert-green/15' : 'hover:bg-surface-primary')}>
          <span className={classNames('w-4 h-4 rounded border flex items-center justify-center shrink-0', isSelected ? 'bg-desert-green border-desert-green' : 'border-border-default')}>
            {isSelected && <IconCheck className="w-3 h-3 text-white" />}</span>
          <span className="flex-1">{t(`map.country.${code}`)}</span>
          {isInstalled && <span className="text-xs text-desert-green">{t('map.installed')}</span>}
        </button></li>;
      })}
    </ul>
    <label>{t('map.detail')} <select value={zoom} disabled={busy} onChange={event => setZoom(Number(event.target.value))} className="rounded border p-2">
      <option value={8}>{t('map.overview')}</option><option value={12}>{t('map.standard')}</option><option value={15}>{t('map.detailed')}</option>
    </select></label>
    {row && <p role="status" className="my-2">{t(`map.job.${row.state}`)}{row.downloadBytes > 0 && ` · ${formatBytes(row.downloadBytes)}`}</p>}
    {row?.state === 'downloading' && <><progress aria-label={t('map.progress')} value={row.completedBytes} max={Math.max(row.downloadBytes,row.completedBytes,1)} /><span className="ml-2">{formatBytes(row.completedBytes)}</span></>}
    {row?.error && (row.state === 'failed' || row.error === 'waiting_wifi') && <p role="status">{t(`map.error.${row.error}`)}</p>}
    {error && <p role="alert">{t(error === 'session' ? 'content.error.session' : 'map.error.failed')}</p>}
    <div className="my-3 flex flex-wrap gap-3">
      <button disabled={busy} onClick={() => action(true)} className="rounded border px-3 py-2">{t('map.checkSize')}</button>
      {(checked || row?.state === 'paused') && <button disabled={busy} onClick={() => action(row.state === 'paused' && row.estimate)} className="rounded bg-desert-green text-white px-3 py-2">{t(row.state === 'paused' ? 'map.resume' : 'map.download')}</button>}
      {row && active(row.state) && <button disabled={pending || ['pausing','cancelling','finishing'].includes(row.state)} onClick={() => action(false,'pause')} className="rounded border px-3 py-2">{t('map.pause')}</button>}
      {row && !['completed','cancelled'].includes(row.state) && <button disabled={pending || ['cancelling','finishing'].includes(row.state)} onClick={() => action(false,'cancel')} className="rounded border px-3 py-2">{t('map.cancel')}</button>}
      {row?.installed && <button disabled={busy} onClick={() => action(false,'remove')} className="rounded border px-3 py-2">{t('map.remove')}</button>}
    </div>
    <p className="my-2 text-sm">{t('map.importHelp')}</p>
    <a href="/native/open-map" className="underline">{t('map.openFile')}</a>
    <ul>{imports.map(entry => <li key={entry.id} className="my-2 flex gap-3"><span>{entry.title}</span>
      <button disabled={busy} onClick={async () => {setPending(true);try {await request('POST',`/${entry.id}/remove`);const next=await request();setRows(next.rows);callbacks.current?.(next.imports);} catch {setError('failed');} finally {setPending(false);}}} className="underline">{t('map.removeFile')}</button></li>)}</ul>
  </details>;
}
