import React, { useState } from 'react';
import { Link } from 'react-router-dom';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useApp } from '../../app/context.js';
import AdminSettings from '../../app/AdminSettings.jsx';
import { formatBytes } from './format.js';

export default function NearbyCopies() {
  const { csrf, t, language, beginNavigation } = useApp();
  const client = useQueryClient(), [removing, setRemoving] = useState(null), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const key = ['nearby-copies', csrf];
  const copies = useQuery({ queryKey: key, retry: false, queryFn: async () => {
    const response = await fetch('/api/copies', { credentials: 'same-origin', cache: 'no-store', headers: { 'X-Tonalli-Csrf': csrf } });
    if (!response.ok) throw Error('copies_unavailable');
    return (await response.json()).copies;
  } });
  async function remove(id, token, onLocked) {
    setBusy(true); setError('');
    try {
      const response = await fetch(`/api/copies/${encodeURIComponent(id)}`, { method: 'DELETE', credentials: 'same-origin', cache: 'no-store', headers: { 'X-Tonalli-Csrf': csrf, 'X-Tonalli-Admin': token } });
      const value = await response.json().catch(() => ({}));
      if (response.status === 423) { onLocked(); throw Error('locked'); }
      if (value.reason === 'in_use' || value.error === 'in_use') throw Error('inUse');
      if (!response.ok) throw Error('removeFailed');
      setRemoving(null);
      await client.invalidateQueries({ queryKey: key });
      await client.invalidateQueries({ queryKey: ['installed-packs', csrf] });
    } catch (failure) { setError(failure.message); } finally { setBusy(false); }
  }
  return <div className="app-card space-y-4" data-testid="nearby-copies">
    <h3>{t('copies.heading')}</h3><p>{t('copies.help')}</p>
    {copies.error ? <p role="alert">{t('copies.loadFailed')}</p>
      : !copies.data ? <p role="status">{t('app.loading')}</p> : copies.data.length === 0 ? <p>{t('copies.empty')}</p> : <AdminSettings>{({ token, onLocked }) => <>
        {error && <p role="alert">{t(`copies.${['locked', 'inUse'].includes(error) ? error : 'removeFailed'}`)}</p>}
        <ul className="space-y-4">{copies.data.map(row => {
          const title = row.kind === 'map' ? t('map.mapFile') : row.title;
          const href = `/${row.kind === 'map' ? 'map' : 'library'}?${new URLSearchParams({ pack: row.id })}`;
          return <li key={row.id} data-copy-id={row.id} className="rounded border p-4 space-y-3">
            <h4>{title}</h4><p>{formatBytes(row.bytes, language)}</p>
            {row.removalPending ? <p role="status">{t('copies.cleanupPending')}</p> : <Link className="secondary-button inline-flex" to={href} onClick={event => beginNavigation(href, event)}>{t(row.kind === 'map' ? 'copy.openMap' : 'content.openLibrary')}</Link>}
            {removing === row.id ? <><p>{t('copies.confirm', { title })}</p><div className="flex flex-wrap gap-3">
              <button className="chat-control" disabled={!token || busy} onClick={() => remove(row.id, token, onLocked)}>{t('content.confirmRemove')}</button>
              <button className="secondary-button" disabled={busy} onClick={() => setRemoving(null)}>{t('content.keep')}</button>
            </div></> : <button className="chat-control ml-3" disabled={!token || busy} onClick={() => { setRemoving(row.id); setError(''); }}>{t(row.removalPending ? 'copies.finishRemoval' : 'content.remove')}</button>}
          </li>;
        })}</ul>
      </>}</AdminSettings>}
    <button className="secondary-button" disabled={busy || copies.isFetching} onClick={() => copies.refetch()}>{t('content.refresh')}</button>
  </div>;
}
