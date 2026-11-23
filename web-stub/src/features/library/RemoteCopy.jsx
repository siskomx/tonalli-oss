import React, { useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useApp } from '../../app/context.js';
import ActiveDownloads from '../content/ActiveDownloads.jsx';
import { Link } from 'react-router-dom';
import { canCopySearch, copyStartBody, copySearchNote } from './copy-search.js';

export default function RemoteCopy({ pack, packs = [], kind = 'zim', statusOnly = false }) {
  const { csrf, t, language, canConnectRemote, remoteLibrary } = useApp();
  const client = useQueryClient(), [busy, setBusy] = useState(false), [error, setError] = useState(false);
  const [withSearch, setWithSearch] = useState(true);
  async function call(path, body) {
    const response = await fetch(path, { method: body ? 'POST' : 'GET', credentials: 'same-origin', cache: 'no-store',
      headers: { 'Content-Type': 'application/json', 'X-Tonalli-Csrf': csrf }, ...(body ? { body: JSON.stringify(body) } : {}) });
    if (!response.ok) throw Error('copy_unavailable');
    return response;
  }
  const status = useQuery({ queryKey: ['remote-copy', csrf], enabled: canConnectRemote,
    queryFn: async () => (await call('/api/remote')).json(), refetchInterval: 1000 });
  const copy = status.data?.copy;
  const active = copy && ['preparing', 'downloading', 'verifying', 'pausing', 'cancelling'].includes(copy.state);
  const unfinished = copy?.id && ['paused', 'failed'].includes(copy.state);
  const title = copy?.kind === 'map' ? t('map.mapFile') : copy?.title || packs.find(item => item.id === copy?.id)?.title || t('copy.title');
  const job = copy?.id && copy.state !== 'idle' ? { id: 'nearby-copy', entryId: copy.id,
    state: copy.state === 'complete' ? 'completed' : copy.state === 'preparing' ? 'queued' : copy.state,
    downloadedBytes: copy.downloaded_bytes, totalBytes: copy.total_bytes, verifiedBytes: copy.verified_bytes, errorCode: copy.error } : null;
  async function perform(action, id = copy?.id) {
    setBusy(true); setError(false);
    try {
      if (action === 'open') {
        if (remoteLibrary) await call('/api/remote/disconnect', {});
        try { sessionStorage.removeItem('library-pack'); } catch {}
        window.location.assign(`/${copy.kind === 'map' ? 'map' : 'library'}?${new URLSearchParams({ pack: copy.local_id })}`);
      } else {
        await call(['pause', 'cancel'].includes(action) ? '/api/remote/copy/control' : '/api/remote/copy',
          ['pause', 'cancel'].includes(action) ? { action } : copyStartBody(id, copy?.kind || kind, action === 'start' ? withSearch : copy?.search != null));
        await client.invalidateQueries({ queryKey: ['remote-copy', csrf] });
      }
    } catch { setError(true); } finally { setBusy(false); }
  }
  if (!canConnectRemote || statusOnly && (!job || copy.state === 'cancelled')) return null;
  return <div className="app-card my-4 space-y-3" data-testid="remote-copy">
    <h3>{t('copy.title')}</h3><p>{t(statusOnly ? 'copy.progressHelp' : kind === 'map' ? 'copy.mapHelp' : 'copy.help')}</p>
    {active && <p>{t(status.data?.background ? 'copy.backgroundHelp' : 'copy.foregroundHelp')}</p>}
    {(error || status.error) && <p role="alert">{t('content.error.copy_failed')}</p>}
    {job && <ActiveDownloads jobs={[job]} titles={{ [copy.id]: title }} onAction={(_, action) => perform(action)} busy={busy}
      disabledActions={remoteLibrary ? [] : ['resume', 'retry']} t={t} language={language} />}
    {copySearchNote(copy) && <p role="status">{t(copySearchNote(copy))}</p>}
    {unfinished && <p>{t('copy.finishCurrent')}</p>}
    {unfinished && !remoteLibrary && <p>{t('copy.reconnectHelp')} <Link className="underline" to="/settings?tab=sharing">{t('copy.reconnect')}</Link></p>}
    {!statusOnly && canCopySearch(kind) && <label className="flex gap-3 items-start"><input type="checkbox" checked={withSearch} disabled={active || unfinished || busy} onChange={event => setWithSearch(event.target.checked)} />{t('copy.withSearch')}</label>}
    <div className="flex flex-wrap gap-3">
      {!statusOnly && <button className="chat-control" disabled={!pack || active || unfinished || busy} onClick={() => perform('start', pack)}>{t(kind === 'map' ? 'copy.saveMap' : 'copy.save')}</button>}
      {copy?.state === 'complete' && copy.local_id && <button className="secondary-button" disabled={busy} onClick={() => perform('open')}>{t(copy.kind === 'map' ? 'copy.openMap' : 'content.openLibrary')}</button>}
    </div>
  </div>;
}
