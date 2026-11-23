import React, { useEffect, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useApp, useRouteReadiness } from '../../app/context.js';
import { loadCatalog, preflight, startDownload, controlJob, removeInstalled } from './client.js';
import { errorKey } from './presentation.js';
import { formatBytes } from './format.js';
import useDownloads from './useDownloads.js';
import ContentSelector from './ContentSelector.jsx';
import ActiveDownloads from './ActiveDownloads.jsx';
import StorageProjectionBar from './StorageProjectionBar.jsx';
import NearbyCopies from './NearbyCopies.jsx';
import RemoteCopy from '../library/RemoteCopy.jsx';
import NotificationNotice from '../../app/NotificationNotice.jsx';
import Tabs from '../../ui/Tabs.jsx';

export default function Content() {
  const { csrf, t, language, beginNavigation, canManageAi } = useApp();
  const { markReady, markPending, markError } = useRouteReadiness('content');
  const client = useQueryClient();
  const catalogKey = ['content-catalog', csrf];
  const catalog = useQuery({ queryKey: catalogKey, queryFn: ({ signal }) => loadCatalog(csrf, signal), retry: false });
  const downloads = useDownloads({ csrf, enabled: catalog.data?.downloadAvailable === true });
  const [selected, setSelected] = useState(null), [busy, setBusy] = useState(false);
  const [error, setError] = useState(null), [removeId, setRemoveId] = useState(null), [tab, setTab] = useState('catalog');
  const command = useRef(null), alive = useRef(true), lastStates = useRef(new Map());
  const entry = catalog.data?.entries.find(item => item.id === selected);
  const budget = useQuery({ queryKey: ['content-preflight', csrf, selected, catalog.data?.revision],
    queryFn: ({ signal }) => preflight(selected, catalog.data.revision, csrf, signal), enabled: Boolean(entry && catalog.data.downloadAvailable), retry: false, staleTime: 0 });
  const jobs = downloads.data?.jobs || [];
  const active = jobs.some(job => ['queued', 'downloading', 'verifying', 'installing', 'pausing', 'cancelling', 'waiting_wifi'].includes(job.state));

  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; command.current?.abort(); };
  }, [csrf]);
  useEffect(() => {
    if (catalog.error) markError(catalog.error);
    else if (!catalog.data) markPending();
    else void markReady({ catalogAvailable: true, downloadAvailable: catalog.data.downloadAvailable });
  }, [catalog.data, catalog.error, markReady, markPending, markError]);
  useEffect(() => {
    if (jobs.some(job => job.state === 'completed' && lastStates.current.get(job.id) !== 'completed')) {
      void client.invalidateQueries({ queryKey: catalogKey });
      void client.invalidateQueries({ queryKey: ['installed-packs', csrf] });
    }
    lastStates.current = new Map(jobs.map(job => [job.id, job.state]));
  }, [downloads.data, csrf]);

  async function run(operation) {
    if (command.current || !catalog.data?.downloadAvailable) return;
    const controller = new AbortController(); command.current = controller;
    const current = () => alive.current && command.current === controller && !controller.signal.aborted;
    setBusy(true); setError(null);
    try {
      await client.cancelQueries({ queryKey: ['content-jobs', csrf] });
      const result = await operation(controller.signal);
      if (!current()) return;
      if (result.job) client.setQueryData(['content-jobs', csrf], previous => ({ jobs: [...(previous?.jobs || []).filter(job => job.id !== result.job.id), result.job] }));
      if (result.removed === false) setError(result.reason || 'failed');
      setRemoveId(null);
      void client.invalidateQueries({ queryKey: catalogKey });
      void client.invalidateQueries({ queryKey: ['content-preflight', csrf] });
      void client.invalidateQueries({ queryKey: ['installed-packs', csrf] });
      void downloads.invalidate();
    } catch (failure) {
      if (current()) setError(failure.code || 'failed');
    } finally {
      if (current()) { command.current = null; setBusy(false); }
    }
  }

  if (!catalog.data) return <section><h2 className="text-lg font-semibold">{t('content.heading')}</h2>
    <p role={catalog.error ? 'alert' : 'status'}>{catalog.error ? t(errorKey(catalog.error.code, t)) : t('app.loading')}</p>
    {catalog.error && <button type="button" className="chat-control mt-3" onClick={() => catalog.refetch()}>{t('content.refresh')}</button>}<RemoteCopy statusOnly /><NearbyCopies /></section>;
  const titles = Object.fromEntries([...jobs.map(job => ({ id: job.entryId, title: t('content.previousDownload') })),
    ...catalog.data.installed, ...catalog.data.entries].map(item => [item.id, item.title]));
  const browse = <>
    <ContentSelector entries={catalog.data.entries} selectedId={selected} onSelect={id => { setSelected(id); setError(null); }} disabled={busy} t={t} language={language} />
    {catalog.data.entries.length === 0 && <p>{t('content.catalogEmpty')}</p>}
    {entry && <div className="space-y-3">
      <h3 className="font-semibold">{entry.title}</h3>
      {catalog.data.downloadAvailable && (budget.data ? <StorageProjectionBar budget={budget.data} t={t} language={language} />
        : <p role={budget.error ? 'alert' : 'status'}>{budget.error ? t(errorKey(budget.error.code, t)) : t('app.loading')}</p>)}
      <div className="flex flex-wrap gap-3">
        {catalog.data.downloadAvailable && <button type="button" className="chat-control" disabled={busy || budget.isFetching} onClick={() => budget.refetch()}>{t('content.checkStorage')}</button>}
        <button type="button" className="chat-control" disabled={busy || active || !downloads.data || !catalog.data.downloadAvailable || !budget.data?.allowed || budget.isFetching}
          onClick={() => { setTab('downloads'); void run(signal => startDownload(entry.id, catalog.data.revision, csrf, signal)); }}>{t(entry.installedVersion && entry.installedVersion !== entry.version ? 'content.update' : 'content.download')}</button>
      </div>
    </div>}
  </>;
  const downloadsPanel = <div className="space-y-3"><h3 className="font-semibold">{t('content.downloads')}</h3>
      {downloads.error && <p role="alert">{t(errorKey(downloads.error.code, t))}</p>}
      {!downloads.data && !downloads.error ? <p role="status">{t('app.loading')}</p> : <ActiveDownloads jobs={jobs} titles={titles} busy={busy} t={t} language={language}
        onAction={(id, action) => run(signal => controlJob(id, action, csrf, signal))} />}
      {downloads.data && jobs.length === 0 && <p>{t('content.jobsEmpty')}</p>}
      <button type="button" className="chat-control" disabled={busy} onClick={() => downloads.invalidate()}>{t('content.refresh')}</button>
    </div>;
  const installedPanel = <div className="space-y-3"><h3 className="font-semibold">{t('content.installed')}</h3>
      {catalog.data.installed.length === 0 ? <p>{t('content.installedEmpty')}</p> : <ul className="space-y-3">{catalog.data.installed.map(item => <li key={item.id} className="rounded border border-border-default bg-white p-4">
        <h4 className="font-semibold">{item.title}</h4><p className="my-2 text-sm">{item.version} · {formatBytes(item.bytes, language)} · {t(item.readable ? 'content.ready' : 'content.unavailable')}</p>
        {item.kind === 'zim' && item.readable && <Link className="chat-control mr-3 inline-flex" to={`/library?${new URLSearchParams({ pack: item.id })}`}
          onClick={event => beginNavigation(`/library?${new URLSearchParams({ pack: item.id })}`, event)}>{t('content.openLibrary')}</Link>}
        {item.kind === 'model' && item.readable && (canManageAi
          ? <Link className="chat-control mr-3 inline-flex" to="/settings?tab=updates">{t('models.openSettings')}</Link>
          : <p>{t('models.versionUnavailable')}</p>)}
        {removeId === item.id ? <><p className="mb-2">{t('content.removeConfirm', { title: item.title })}</p><div className="flex flex-wrap gap-2">
          <button type="button" className="chat-control" disabled={busy || !catalog.data.downloadAvailable} onClick={() => run(signal => removeInstalled(item.id, csrf, signal))}>{t('content.confirmRemove')}</button>
          <button type="button" className="chat-control" disabled={busy} onClick={() => setRemoveId(null)}>{t('content.keep')}</button></div></>
          : <button type="button" className="chat-control" disabled={busy || !catalog.data.downloadAvailable} onClick={() => setRemoveId(item.id)}>{t('content.remove')}</button>}
      </li>)}</ul>}
    </div>;
  const activeCount = jobs.filter(job => ['queued', 'downloading', 'verifying', 'installing', 'pausing', 'cancelling', 'waiting_wifi'].includes(job.state)).length;
  const tabs = [
    { id: 'catalog', label: t('content.tab.catalog'), content: browse },
    ...(catalog.data.downloadAvailable ? [{ id: 'downloads', label: t('content.tab.downloads'), badge: activeCount || undefined, content: downloadsPanel }] : []),
    { id: 'installed', label: t('content.tab.installed'), badge: catalog.data.installed.length || undefined, content: installedPanel },
    { id: 'nearby', label: t('content.tab.nearby'), content: <NearbyCopies /> },
  ];
  return <section className="space-y-6">
    <div><h2 className="text-xl font-semibold">{t('content.heading')}</h2><p className="mt-2 text-sm">{t('content.openLocal')}</p></div>
    <NotificationNotice />
    <RemoteCopy statusOnly />
    {!catalog.data.downloadAvailable && <p role="status" className="rounded border border-amber-300 bg-amber-50 p-3">{t('content.downloadsUnavailable')}</p>}
    {error && <p role="alert">{t(errorKey(error, t))}</p>}
    <Tabs tabs={tabs} label={t('content.tabs')} value={tab} onChange={setTab} />
  </section>;
}
