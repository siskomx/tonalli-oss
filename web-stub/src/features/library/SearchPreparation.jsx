import React, { useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useApp } from '../../app/context.js';
import { loadPreparation, startPreparation, controlPreparation } from './preparation-client.js';
import SearchPreparationView, { preparationActive } from './SearchPreparationView.js';

export default function SearchPreparation({ pack }) {
  const { csrf, t, language } = useApp();
  const cache = useQueryClient();
  const command = useRef(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const key = ['index-preparation', pack, csrf];
  const status = useQuery({ queryKey: key, enabled: Boolean(pack && csrf), retry: false,
    queryFn: ({ signal }) => loadPreparation(pack, csrf, signal),
    refetchInterval: query => !busy && preparationActive(query.state.data?.state) ? 2000 : false });
  useEffect(() => {
    setBusy(false); setError(null);
    return () => { command.current?.abort(); command.current = null; };
  }, [pack, csrf]);
  useEffect(() => {
    if (status.data?.state === 'ready') void cache.invalidateQueries({ queryKey: ['body-search', pack] });
  }, [status.data?.state, pack, cache]);
  async function act(action) {
    if (command.current) return;
    const controller = new AbortController(); command.current = controller;
    setBusy(true); setError(null);
    try {
      await cache.cancelQueries({ queryKey: key, exact: true });
      if (command.current !== controller) return;
      const next = action === 'start' ? await startPreparation(pack, csrf, controller.signal)
        : await controlPreparation(pack, status.data.jobId, status.data.epoch, action, csrf, controller.signal);
      if (command.current === controller) cache.setQueryData(key, next);
    } catch (failure) {
      if (command.current === controller && failure.name !== 'AbortError') {
        setError(failure);
        // A lost reply does not prove that the native worker rejected the command.
        void cache.invalidateQueries({ queryKey: key });
      }
    } finally {
      if (command.current === controller) { command.current = null; setBusy(false); }
    }
  }
  return <SearchPreparationView status={status.data} loading={status.isFetching} error={error || status.error}
    busy={busy} onAction={act} t={t} language={language} />;
}
