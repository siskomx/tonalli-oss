import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useApp, useRouteReadiness } from '../../app/context.js';
import { loadPacks, loadSuggestions, loadTitlePage, titleWindow } from './client.js';
import Input from '../../ui/Input.jsx';
import Select from '../../ui/Select.jsx';
import { libraryCommit } from './diagnostics.js';
import BodySearch from './BodySearch.jsx';
import { nativeShell } from '../../app/AppLayout.jsx';
import UnifiedLibrary from './UnifiedLibrary.jsx';
import ReadingHistory from './ReadingHistory.jsx';
import RemoteCopy from './RemoteCopy.jsx';

const PAGE = 200;
const HEIGHT = 48;

export default function Library() {
  const { csrf, t, language, canGetContent, setLibraryPack, setLastArticle, beginNavigation,privateDevice,canConnectRemote,remoteLibrary } = useApp();
  const { markReady, markPending, markError } = useRouteReadiness('library');
  const [search, setSearch] = useSearchParams();
  const packs = useQuery({ queryKey: ['installed-packs', csrf], queryFn: ({ signal }) => loadPacks(csrf, signal) });
  const requested = search.get('pack');
  const pack = packs.data?.find(item => item.id === requested)?.id ?? packs.data?.[0]?.id;
  const metadata = packs.data?.find(item => item.id === pack);
  const [mode, setMode] = useState('titles');
  const titles = mode === 'titles';
  const [query, setQuery] = useState('');
  const [debounced, setDebounced] = useState('');
  const [jump, setJump] = useState('1');
  const requestOffset = Number(search.get('offset') || 0);
  const browseWindow = titleWindow(metadata?.total ?? 0, requestOffset);
  const selectedOffset = Math.min(Math.max(0, (metadata?.total ?? 1) - 1),
    Number.isSafeInteger(requestOffset) && requestOffset >= 0 ? requestOffset : 0);
  const element = useRef(null);
  const client = useQueryClient();
  const options = (offset) => ({ queryKey: ['titles', pack ?? null, offset, csrf], enabled: Boolean(pack && titles), queryFn: ({ signal }) => loadTitlePage(pack, offset, csrf, signal) });
  const first = useQuery(options(browseWindow.start));
  const count = browseWindow.count;
  const virtual = useVirtualizer({ count: titles ? count : 0, getScrollElement: () => element.current, estimateSize: () => HEIGHT, overscan: 8 });
  const items = virtual.getVirtualItems();
  const offsets = [...new Set(items.map((item) => browseWindow.start + Math.floor(item.index / PAGE) * PAGE))];
  const pages = useQueries({ queries: offsets.map(options) });
  const byOffset = new Map(offsets.map((offset, index) => [offset, pages[index].data]));
  const rowAt = (index) => byOffset.get(browseWindow.start + Math.floor(index / PAGE) * PAGE)?.rows[index % PAGE];
  const empty = Boolean(packs.data && packs.data.length === 0);
  const loaded = empty || Boolean(first.data && (count === 0 || items.length > 0) && items.every((item) => rowAt(item.index)));
  const pageVersion = pages.map((page) => page.dataUpdatedAt).join(',');
  const itemRange = items.map((item) => item.index).join(',');
  const suggestions = useQuery({ queryKey: ['title-search', pack, debounced, csrf], enabled: Boolean(titles && pack && debounced),
    queryFn: ({ signal }) => loadSuggestions(pack, debounced, csrf, signal) });
  const error = packs.error || (titles && (first.error || pages.find((page) => page.error)?.error));
  useEffect(() => { const timer = setTimeout(() => setDebounced(query), 180); return () => clearTimeout(timer); }, [query]);
  useEffect(() => { setQuery(''); setDebounced(''); }, [pack]);
  useEffect(() => {
    element.current?.scrollTo({ top: (selectedOffset - browseWindow.start) * HEIGHT });
    setJump(String(selectedOffset + 1));
  }, [pack, browseWindow.start, selectedOffset, titles]);
  function changeMode(value) { setQuery(''); setDebounced(''); setMode(value); }
  function choose(value) { setQuery(''); setDebounced(''); setSearch({ pack: value }); }
  function browse(offset) { if (pack) setSearch({ pack, offset: String(offset) }); }
  function go(event) {
    event.preventDefault();
    if (/^[0-9]+$/.test(jump)) { const number = Number(jump); if (Number.isSafeInteger(number) && number >= 1 && number <= (metadata?.total ?? 0)) browse(number - 1); }
  }
  function article(row) {
    const href = `/article?${new URLSearchParams({ pack, url: row.url })}`;
    return { to: href, onClick: event => { if (beginNavigation(href, event)) setLastArticle({ pack, url: row.url }); } };
  }

  useLayoutEffect(() => {
    if (!titles) return;
    const snapshot = libraryCommit(first.data?.pack ?? pack, count, HEIGHT, error ? [] : items, rowAt);
    window.__libraryCommit = snapshot;
    return () => { if (window.__libraryCommit === snapshot) delete window.__libraryCommit; };
  }, [first.data?.pack, pack, count, pageVersion, itemRange, error, titles]);

  useEffect(() => {
    if (!titles) return;
    if (error) markError(error);
    else if (!loaded) markPending();
    else {
      const loadedRows = client.getQueriesData({ queryKey: ['titles', pack ?? null] })
        .reduce((sum, [, page]) => sum + (page?.rows.length || 0), 0);
      void markReady({ library: { total: metadata?.total ?? 0, visibleLimit: count, windowStart: browseWindow.start, loadedRows, renderedRows: items.length,
        firstIndex: items[0] ? browseWindow.start + items[0].index : null, lastIndex: items.at(-1) ? browseWindow.start + items.at(-1).index : null, rowHeight: HEIGHT, pack } });
    }
  }, [loaded, error, pageVersion, itemRange, pack, metadata?.total, browseWindow.start, markReady, markPending, markError, titles]);
  useEffect(() => {
    if (!titles || !first.data) return;
    setLibraryPack(first.data.pack);
  }, [first.data, setLibraryPack, titles]);
  useEffect(() => { if (!titles && pack) setLibraryPack(pack); }, [titles, pack, setLibraryPack]);

  const number = (value) => new Intl.NumberFormat(language).format(value);
  return <section>
    <div className="page-heading"><div><h2>{t('library.heading')}</h2><p>{t('library.subtitle')}</p></div>
      <div className="flex flex-wrap gap-3">{canGetContent && <Link className="chat-control inline-flex" to="/content" onClick={event => beginNavigation('/content', event)}>{t('nav.content')}</Link>}
      {nativeShell() && privateDevice && <a className="secondary-button" href="/native/open">{t('library.openFile')}</a>}
      {canConnectRemote&&!remoteLibrary&&<Link className="secondary-button" to="/settings?tab=sharing">{t('remote.connect')}</Link>}</div></div>
    {canGetContent&&<><ReadingHistory/><UnifiedLibrary packs={packs.data||[]}/></>}
    {error ? <p role="alert">{t('library.failed')}</p> : empty ? !canGetContent&&<div className="my-5 space-y-4 rounded-lg border border-border-default bg-surface-primary p-5">
      <p role="status">{t('library.empty')}</p><p>{t('library.openContent')}</p>
      {canGetContent && <Link className="chat-control inline-flex" to="/content" onClick={event => beginNavigation('/content', event)}>{t('nav.content')}</Link>}
    </div> : <>
      {packs.data && <Select name="installed-content" label={t('library.installed')} value={pack || ''} onChange={choose}
        options={packs.data.map(item => ({ value: item.id, label: item.title }))} />}
      {remoteLibrary && canConnectRemote && <RemoteCopy pack={pack} packs={packs.data || []} />}
      <Select name="library-search-mode" label={t('library.searchMode')} value={mode} onChange={changeMode}
        options={[{ value: 'titles', label: t('library.titleMode') }, { value: 'body', label: t('library.bodyMode') }]} />
      <Input name={titles ? 'title-search' : 'body-search'} label={t(titles ? 'library.search' : 'library.bodySearch')} type="search" value={query} maxLength={4096}
        disabled={!pack} helpText={t(titles ? 'library.prefixHint' : 'library.bodyHint')} onChange={event => setQuery(event.target.value)} />
      {!titles ? <BodySearch key={`${pack}:${csrf}`} pack={pack} query={query} debounced={debounced} article={article} /> : <>
      {query && <div aria-live="polite">
        {suggestions.error ? <p role="alert">{t('library.searchFailed')}</p> : query !== debounced || suggestions.isFetching ? <p>{t('app.loading')}</p>
          : suggestions.data?.pack === pack && <><p>{t('library.titleOnly')}</p>{suggestions.data.hits.length === 0 ? <p>{t('library.noResults')}</p>
            : <ul>{suggestions.data.hits.map(row => <li key={row.url}><Link className="block py-2 underline" {...article(row)}>{row.title}</Link></li>)}</ul>}</>}
      </div>}
      <p className="my-3" data-testid="library-total">{first.data
        ? t('library.range', { start: number(count ? browseWindow.start + 1 : 0), end: number(browseWindow.start + count), total: number(first.data.total) }) : t('app.loading')}</p>
      <div className="my-3 flex flex-wrap items-end gap-3">
        <button className="chat-control" disabled={browseWindow.previous === null} onClick={() => browse(browseWindow.previous)}>{t('library.previous')}</button>
        <button className="chat-control" disabled={browseWindow.next === null} onClick={() => browse(browseWindow.next)}>{t('library.next')}</button>
        <details><summary>{t('settings.advanced')}</summary><form onSubmit={go} className="flex items-end gap-2"><Input name="title-number" label={t('library.titleNumber')} type="number" min="1" max={metadata?.total || 1}
          value={jump} onChange={event => setJump(event.target.value)} /><button className="chat-control" type="submit" disabled={!count}>{t('library.go')}</button></form></details>
      </div>
      <div ref={element} data-testid="library-list" data-row-height={HEIGHT} role="list" aria-label={t('library.heading')}
        className="overflow-auto rounded border border-slate-300 bg-white" style={{ height: '65vh', maxHeight: 640 }}>
        <div style={{ height: virtual.getTotalSize(), width: '100%', position: 'relative' }}>
          {items.map((item) => {
            const row = rowAt(item.index);
            const style = { position: 'absolute', top: 0, left: 0, width: '100%', height: HEIGHT, transform: `translateY(${item.start}px)` };
            return row ? <Link key={`${pack}-${browseWindow.start}-${item.key}`} data-testid="library-row" data-index={browseWindow.start + item.index} data-url={row.url}
              role="listitem" aria-posinset={browseWindow.start + item.index + 1} aria-setsize={metadata?.total} style={style}
              className="flex items-center truncate border-b border-slate-100 px-3 text-sm hover:bg-slate-100"
              {...article(row)}>{row.title}</Link>
              : <div key={item.key} data-testid="library-placeholder" style={style} className="px-3 py-2 text-sm text-slate-500">{t('library.loadingRow')}</div>;
          })}
        </div>
      </div>
      </>}
    </>}
  </section>;
}
