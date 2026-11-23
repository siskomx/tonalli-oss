import React, { useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useApp } from '../../app/context.js';
import { cancelUpdate, checkForUpdate, downloadUpdate, loadUpdate, saveUpdateSettings } from './client.js';

const megabytes = (bytes) => `${(bytes / 1_000_000).toFixed(bytes < 10_000_000 ? 1 : 0)} MB`;

function Offer({ t, data, run, pending }) {
  const { available: offer, download } = data;
  const [copied, setCopied] = useState(false);
  const copy = () => { void navigator.clipboard?.writeText(offer.page).then(() => setCopied(true), () => setCopied(false)); };
  const running = download.state === 'running';
  return <div className="space-y-3" role="status">
    <h4>{t('update.available')} <strong>{offer.version}</strong></h4>
    {offer.notes && <p className="whitespace-pre-line">{offer.notes}</p>}
    {offer.mode === 'link' ? <>
      <p>{t('update.iphoneHelp')}</p>
      <p className="break-all"><code>{offer.page}</code></p>
      <button type="button" className="secondary-button" onClick={copy}>{copied ? t('update.copied') : t('update.copyLink')}</button>
    </> : <>
      <p>{t('update.size')} {megabytes(offer.bytes)}</p>
      {running && <>
        <progress role="progressbar" max={download.total || offer.bytes} value={download.downloaded} aria-label={t('update.downloading')} />
        <button type="button" className="secondary-button" onClick={() => run('cancel')}>{t('update.cancel')}</button>
      </>}
      {!running && !offer.downloaded && <button type="button" className="chat-control" disabled={pending} onClick={() => run('download')}>{t('update.download')}</button>}
      {!running && offer.downloaded && offer.mode === 'apply' && <>
        <p>{t('update.applyHelp')}</p>
        <a className="chat-control inline-flex" href="/native/apply-update">{t('update.apply')}</a>
      </>}
      {!running && offer.downloaded && offer.mode === 'install' && <>
        <p>{t('update.installHelp')}</p>
        <a className="chat-control inline-flex" href="/native/install-update">{t('update.install')}</a>
      </>}
    </>}
    {download.state === 'failed' && <p role="alert">{t(`update.error.${download.error}`)}</p>}
  </div>;
}

export default function UpdateSettings() {
  const { t, csrf } = useApp();
  const queryClient = useQueryClient();
  const key = ['app-update', csrf];
  const query = useQuery({ queryKey: key, queryFn: () => loadUpdate(csrf),
    refetchInterval: (state) => state.state.data?.download?.state === 'running' ? 1000 : false });
  const [draft, setDraft] = useState(null);
  const automatic = useRef(false);
  const mutation = useMutation({
    mutationFn: ({ action }) => ({ save: () => saveUpdateSettings(csrf, form), check: () => checkForUpdate(csrf),
      download: () => downloadUpdate(csrf), cancel: () => cancelUpdate(csrf) })[action](),
    onSuccess: (data) => { queryClient.setQueryData(key, data); if (mutation.variables?.action === 'save') setDraft(null); },
  });
  const data = query.data;
  const form = draft ?? (data ? { enabled: data.enabled, source: data.source, public_key: data.public_key } : { enabled: false, source: '', public_key: '' });
  const run = (action) => mutation.mutate({ action });
  const checkedOnce = data?.check !== 'never';

  // Only an update source the user turned on is ever contacted, and only here, once per app run.
  useEffect(() => {
    if (data?.supported && data.enabled && data.check === 'never' && !automatic.current) { automatic.current = true; run('check'); }
  }, [data?.supported, data?.enabled, data?.check]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!data) return <div className="app-card"><h3>{t('update.title')}</h3><p role="status">{query.isError ? t('update.failed') : t('app.loading')}</p></div>;
  if (!data.supported) return <div className="app-card space-y-3"><h3>{t('update.title')}</h3><p>{t('update.unsupported')}</p></div>;

  const change = (field) => (event) => setDraft({ ...form, [field]: field === 'enabled' ? event.target.checked : event.target.value });
  const failedMessage = mutation.isError ? mutation.error.message : '';
  return <div className="app-card space-y-4"><h3>{t('update.title')}</h3>
    <p>{t('update.help')}</p><p>{t('update.privacy')}</p>
    <p>{t('update.current')} <strong>{data.current}</strong></p>
    <form className="space-y-3" onSubmit={(event) => { event.preventDefault(); run('save'); }}>
      <label className="flex items-center gap-2"><input type="checkbox" name="enabled" checked={form.enabled} onChange={change('enabled')} />{t('update.enable')}</label>
      <label className="block">{t('update.source')}
        <input className="block w-full border p-2" name="source" value={form.source} placeholder="owner/repo" autoComplete="off" spellCheck="false" onChange={change('source')} /></label>
      <label className="block">{t('update.publicKey')}
        <input className="block w-full border p-2 font-mono" name="public_key" value={form.public_key} autoComplete="off" spellCheck="false" onChange={change('public_key')} /></label>
      <p className="text-sm">{t('update.keyHelp')}</p>
      <div className="flex flex-wrap gap-3">
        <button className="chat-control" disabled={mutation.isPending}>{t('update.save')}</button>
        <button type="button" className="secondary-button" disabled={!data.enabled || draft !== null || mutation.isPending} onClick={() => run('check')}>{t('update.check')}</button>
      </div>
    </form>
    {failedMessage && <p role="alert">{t(failedMessage)}</p>}
    {data.check === 'error' && <p role="alert">{t(`update.error.${data.error}`)}</p>}
    {data.check === 'up_to_date' && <p role="status">{t('update.upToDate')}</p>}
    {data.available && <Offer t={t} data={data} run={run} pending={mutation.isPending} />}
    {checkedOnce && data.check === 'available' && !data.available && <p role="status">{t('update.noPackage')}</p>}
  </div>;
}
