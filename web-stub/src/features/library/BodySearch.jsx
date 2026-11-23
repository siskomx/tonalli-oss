import React, { useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useRouteReadiness } from '../../app/context.js';
import { loadBodySearch } from './client.js';
import BodyResults from './BodyResults.js';
import SearchPreparation from './SearchPreparation.jsx';

export default function BodySearch({ pack, query, debounced, article }) {
  const { csrf, t, canPrepareSearch } = useApp();
  const { markPending, markReady, markError } = useRouteReadiness('library');
  const result = useQuery({ queryKey: ['body-search', pack, debounced, csrf],
    enabled: Boolean(pack && debounced.trim()), retry: false,
    queryFn: ({ signal }) => loadBodySearch(pack, debounced, csrf, signal) });
  const pending = query !== debounced || result.isFetching;
  const current = query === debounced;
  useEffect(() => {
    if (pending) markPending();
    else if (current && result.error) markError(result.error);
    else void markReady({ library: { mode: 'body', pack, resultCount: current ? result.data?.hits.length ?? 0 : 0 } });
  }, [pending, current, result.error, result.data, pack, markPending, markReady, markError]);
  return <div aria-live="polite">{canPrepareSearch && <SearchPreparation key={pack} pack={pack} />}<BodyResults t={t} article={article} pending={pending}
    error={current ? result.error : null} data={current && query.trim() ? result.data : null} /></div>;
}
