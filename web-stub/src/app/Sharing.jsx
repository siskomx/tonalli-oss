import React, { useEffect, useRef, useState } from 'react';
import { useApp } from './context.js';
import SharingAddress from './SharingAddress.jsx';
import HotspotJoin from './HotspotJoin.jsx';
import PersonalHotspot from './PersonalHotspot.jsx';
import { iosShell } from './platform.js';
import { isHotspotNetwork, personalHotspotNetwork } from './hotspot.js';
import Input from '../ui/Input.jsx';
import { sharedSelection } from './sharing-selection.js';
import {searchMedia} from '../features/library/media-catalog.js';
import {useMusic} from '../features/music/MusicPlayer.jsx';

export default function Sharing() {
  const { csrf, t } = useApp();
  const catalog=useMusic();
  const [data, setData] = useState(null), [error, setError] = useState(false), [busy, setBusy] = useState(false);
  const [hiddenPacks, setHiddenPacks] = useState([]), [network, setNetwork] = useState('');
  const [hiddenMedia,setHiddenMedia]=useState([]),[query,setQuery]=useState(''),[page,setPage]=useState(0);
  const toggle = (setter, id, share) => setter(current => share ? current.filter(item => item !== id) : [...current.filter(item => item !== id), id]);
  const active = useRef(true);
  async function request(path = '', body) {
    const reply = await fetch(`/api/sharing${path}`, { method: body ? 'POST' : 'GET', credentials: 'same-origin', cache: 'no-store',
      headers: { 'X-Tonalli-Csrf': csrf, ...(body ? { 'Content-Type': 'application/json' } : {}) }, ...(body ? { body: JSON.stringify(body) } : {}) });
    if (!reply.ok) throw new Error('sharing');
    return reply.json();
  }
  async function refresh() {
    try { const value = await request(); if (active.current) { setData(value); setNetwork(current => value.status.networks.some(n => n.id === current) ? current : value.status.networks[0]?.id || ''); } }
    catch { if (active.current) setError(true); }
  }
  useEffect(() => {
    active.current = true; void refresh();
    const timer = setInterval(refresh, 2000);
    return () => { active.current = false; clearInterval(timer); };
  }, [csrf]);
  async function perform(path, body = {}) {
    setBusy(true); setError(false);
    try { await request(path, body); await refresh(); }
    catch { if (active.current) setError(true); }
    finally { if (active.current) setBusy(false); }
  }
  const status = data?.status, disabled = busy || status?.busy;
  const available=catalog?.files||[],matched=searchMedia(available,query);
  const packs = data?.packs || [];
  const { packs: sharedPacks, media: validMedia, total, shared, hidden, tooMany } = sharedSelection(packs, available, hiddenPacks, hiddenMedia);
  const sharedNowPacks = (data?.shared_packs || []).map(id => packs.find(pack => pack.id === id)).filter(Boolean);
  const sharedNowFiles = data?.selected_media || [];
  const title = pack => {
    const country = /^(?:country-)?(mx|us|ca)(?:-|$)/.exec(pack.id)?.[1];
    return pack.kind === 'map' ? (country ? t(`map.country.${country}`) : t('map.mapFile')) : pack.title;
  };
  return <div className="app-card space-y-4" data-testid="sharing">
    <h3>{t('sharing.title')}</h3><p>{t('sharing.help')}</p>
    {(error || status?.error) && <p role="alert">{t('sharing.error')}</p>}
    {!status && !error && <p role="status">{t('sharing.loading')}</p>}
    {status && <>
      <p role="status">{t(status.busy ? 'sharing.starting' : status.running ? 'sharing.on' : 'sharing.off')}</p>
      {status.running ? <>
        <p>{t('sharing.connect')}</p>
        {status.hotspot && <HotspotJoin hotspot={status.hotspot} t={t} />}
        {iosShell() && <PersonalHotspot networks={status.networks} running url={status.url} t={t} />}
        <h4>{t('sharing.sharedNow')}</h4><p>{t('sharing.removeHelp')}</p>
        {sharedNowPacks.length + sharedNowFiles.length === 0 ? <p>{t('sharing.nothingShared')}</p> : <ul className="space-y-3" data-testid="sharing-now">
          {sharedNowPacks.map(pack => <li key={pack.id} className="flex flex-wrap gap-3 items-center"><span className="break-words">{title(pack)}</span>
            <button type="button" className="secondary-button" disabled={disabled} aria-label={t('sharing.removeOne', { name: title(pack) })} onClick={() => perform('/hide', { packs: [pack.id] })}>{t('sharing.removeShort')}</button></li>)}
          {sharedNowFiles.map(file => <li key={file.id} className="flex flex-wrap gap-3 items-center"><span className="break-words">{file.name}</span>
            <button type="button" className="secondary-button" disabled={disabled} aria-label={t('sharing.removeOne', { name: file.name })} onClick={() => perform('/hide', { media: [file.id] })}>{t('sharing.removeShort')}</button></li>)}
        </ul>}
        {!!sharedNowFiles.length && <p>{t('sharing.snapshotHelp')}</p>}
        <SharingAddress status={status} t={t} />
        {status.native_invitation && <details><summary>{t('remote.shareInvitation')}</summary><p className="my-3">{t('remote.shareHelp')}</p>
          <textarea readOnly aria-label={t('remote.invitation')} className="w-full rounded border p-3" rows="3" value={status.native_invitation} onFocus={event=>event.target.select()} />
          <button className="secondary-button" onClick={async()=>{try{await navigator.clipboard.writeText(status.native_invitation);}catch{window.prompt(t('sharing.copyHelp'),status.native_invitation);}}}>{t('remote.copyInvitation')}</button>
        </details>}
        <p>{t('sharing.code')} <strong data-testid="sharing-code">{status.pairing_code}</strong></p>
        <p>{t('sharing.trust')}</p>
        <details><summary>{t('sharing.certificate')}</summary><p className="my-3">{t('sharing.certificateHelp')}</p><code className="break-all">{status.certificate_sha256}</code></details>
        <button className="chat-control" disabled={disabled} onClick={() => perform('/stop')}>{t('sharing.stop')}</button>
        <h4>{t('sharing.devices')}</h4>
        {status.devices.length === 0 ? <p>{t('sharing.noDevices')}</p> : <ul className="space-y-3">{status.devices.map(device => <li key={device.id} className="flex flex-wrap gap-3 items-center">
          <span>{device.label}</span><button className="secondary-button" disabled={disabled} onClick={() => perform(`/devices/${encodeURIComponent(device.id)}/disconnect`)}>{t('sharing.disconnect')}</button>
        </li>)}</ul>}
      </> : <>
        <p role="status" data-testid="sharing-summary">{t('sharing.summary',{shared,total,hidden})}</p>
        {hidden > 0 && <button type="button" className="secondary-button" disabled={disabled} onClick={() => { setHiddenPacks([]); setHiddenMedia([]); }}>{t('sharing.shareAll')}</button>}
        <h4>{t('sharing.choose')}</h4>
        {packs.length === 0 ? <p>{t('sharing.empty')}</p> : <fieldset disabled={disabled} className="space-y-3"><legend className="sr-only">{t('sharing.choose')}</legend>
          {packs.map(pack => <label key={pack.id} className="flex gap-3 items-center"><input type="checkbox" checked={!hiddenPacks.includes(pack.id)} onChange={event => toggle(setHiddenPacks, pack.id, event.target.checked)} />{title(pack)}</label>)}
        </fieldset>}
        <h4>{t('sharing.media')}</h4><p>{t('sharing.mediaHelp')}</p>
        {!available.length?<p>{t('sharing.noFiles')}</p>:<>
        <Input name="sharing-media-search" label={t('unified.search')} type="search" value={query} maxLength={512} onChange={event=>{setQuery(event.target.value);setPage(0);}}/>
        <fieldset disabled={disabled} className="space-y-3"><legend className="sr-only">{t('sharing.media')}</legend>
          {matched.slice(page*50,page*50+50).map(file=><label key={file.id} className="flex gap-3 items-center break-words"><input type="checkbox" data-media-id={file.id} checked={!hiddenMedia.includes(file.id)} onChange={event=>toggle(setHiddenMedia,file.id,event.target.checked)}/><span>{file.name}<small className="block">{file.folder}</small></span></label>)}
        </fieldset></>}
        {matched.length>50&&<div className="flex gap-3"><button className="secondary-button" disabled={!page} onClick={()=>setPage(page-1)}>{t('library.previous')}</button><button className="secondary-button" disabled={(page+1)*50>=matched.length} onClick={()=>setPage(page+1)}>{t('library.next')}</button></div>}
        {tooMany && <p role="alert">{t('sharing.tooMany')}</p>}
        {status.networks.length > 1 && <label className="block">{t('sharing.network')}<select className="block rounded border p-3" value={network} disabled={disabled} onChange={event => setNetwork(event.target.value)}>{status.networks.map(n => <option key={n.id} value={n.id}>{isHotspotNetwork(n.id) ? t('sharing.hotspot') : personalHotspotNetwork(n) ? t('sharing.personalHotspot') : n.label}</option>)}</select></label>}
        {iosShell() && <PersonalHotspot networks={status.networks} running={false} url={null} t={t} />}
        {!network && <p>{t('sharing.noNetwork')}</p>}
        {isHotspotNetwork(network) && <p>{t(network === 'hotspot' ? 'sharing.hotspotHelp' : 'sharing.hotspotPermissionHelp')}</p>}
        <p>{t('room.chatOnly')}</p>
        {network === 'hotspot-permission' ? <a className="chat-control inline-flex" href="/native/hotspot-permission">{t('sharing.hotspotAllow')}</a> : <button className="chat-control" disabled={disabled || !network || tooMany} onClick={() => perform('/start', { network, packs: sharedPacks, media: validMedia })}>{t('sharing.start')}</button>}
        {status.busy && <button className="secondary-button" disabled={busy} onClick={() => perform('/stop')}>{t('sharing.stop')}</button>}
      </>}
    </>}
    <p>{t('sharing.lifetime')}</p>
  </div>;
}
