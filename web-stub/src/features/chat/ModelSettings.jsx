// Adapted from Project NOMAD admin/inertia/pages/settings/models.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: local signed content acquisition, explicit Core load/off, EN/ES;
// no Ollama, remote service, GPU configuration or automatic model activation.
import React, { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { useApp } from '../../app/context.js';
import { loadModels, changeModel } from './client.js';
import { formatBytes } from '../content/format.js';

export default function ModelSettings() {
  const { csrf, t, language, canGetContent, canUseAi } = useApp();
  const queryClient = useQueryClient(), key = ['ai-models', csrf];
  const [selected, setSelected] = useState('');
  const query = useQuery({ queryKey: key, queryFn: ({ signal }) => loadModels(csrf, signal), retry: false,
    refetchInterval: query => ['loading', 'stopping'].includes(query.state.data?.state) ? 1000 : false });
  const mutation = useMutation({ mutationFn: ({ action, model }) => changeModel(csrf, action, model),
    onSuccess: value => queryClient.setQueryData(key, value),
    onSettled: () => { void queryClient.invalidateQueries({ queryKey: key }); void queryClient.invalidateQueries({ queryKey: ['session'] }); } });
  const data = query.data, chosen = data?.models.find(model => model.id === selected);
  useEffect(() => {
    if (data) setSelected(previous => data.models.some(model => model.id === data.model) ? data.model
      : data.models.some(model => model.id === previous) ? previous : data.models[0]?.id ?? '');
  }, [data]);
  useEffect(() => { if (data) void queryClient.invalidateQueries({ queryKey: ['session'] }); }, [data?.state, data?.model]);
  return <section className="app-card space-y-3" aria-label={t('models.title')}>
    <h3>{t('models.title')}</h3><p>{t('models.help')}</p>
    {!data ? <p role="status">{t(query.isError ? 'models.unavailable' : 'app.loading')}</p> : <>
      <p role="status">{t(`models.state.${data.state}`)}</p>
      {data.model && <p>{t(data.state === 'ready' ? 'models.active' : 'models.selected', { model: data.models.find(model => model.id === data.model)?.title ?? data.model })}</p>}
      {data.error && <p role="alert">{t(`models.error.${data.error}`)}</p>}
      {data.models.length > 0 && <label>{t('models.choose')}<select value={selected}
        disabled={mutation.isPending || data.state !== 'off'} onChange={event => setSelected(event.target.value)}>
        {data.models.map(model => <option key={model.id} value={model.id}>{model.title} · {formatBytes(model.bytes, language)}</option>)}
      </select></label>}
      {chosen && <p>{t(chosen.installed ? 'models.installed' : 'models.downloadFirst')}</p>}
      <div className="flex flex-wrap gap-3">
        <button className="chat-control" disabled={mutation.isPending || data.state !== 'off' || !chosen?.installed}
          onClick={() => mutation.mutate({ action: 'load', model: selected })}>{t('models.use')}</button>
        <button className="secondary-button" disabled={mutation.isPending || ['off', 'stopping'].includes(data.state)}
          onClick={() => mutation.mutate({ action: 'off' })}>{t('models.off')}</button>
        {data.state === 'ready' && canUseAi && <Link className="secondary-button" to="/chat">{t('nav.chat')}</Link>}
      </div>
      {canGetContent && data.downloadAvailable && <Link to="/content">{t('models.get')}</Link>}
    </>}
    {mutation.isError && <p role="alert">{t(mutation.error.message === 'chat.session' ? 'chat.session' : 'models.commandFailed')}</p>}
    {query.isError && data && <p role="alert">{t('models.unavailable')}</p>}
    <button className="secondary-button" disabled={query.isFetching || mutation.isPending} onClick={() => query.refetch()}>{t('content.refresh')}</button>
  </section>;
}
