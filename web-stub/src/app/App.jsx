import React, { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Navigate, Route, Routes, useLocation } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { displayName } from './AppName.jsx';
import { AppContext } from './context.js';
import { startRouteSwitch } from './performance.js';
import { createTranslator, resolveLanguage } from '../locales/index.js';
import { loadSessionInfo } from '../../session.js';

import AppLayout from './AppLayout.jsx';
import Settings from './Settings.jsx';
import { useTheme } from './useTheme.js';
import MusicPlayer from '../features/music/MusicPlayer.jsx';

const Library = lazy(() => import('../features/library/Library.jsx'));
const Article = lazy(() => import('../features/article/Article.jsx'));
const MapRoute = lazy(() => import('../features/map/Map.jsx'));
const Notes = lazy(() => import('../features/notes/Notes.jsx'));
const Chat = lazy(() => import('../features/chat/Chat.jsx'));
const Content = lazy(() => import('../features/content/Content.jsx'));
const Files = lazy(() => import('../features/files/Files.jsx'));
const Music = lazy(() => import('../features/music/Music.jsx'));
const Videos = lazy(() => import('../features/video/Videos.jsx'));
const Gallery = lazy(() => import('../features/gallery/Gallery.jsx'));
const FilePage = lazy(() => import('../features/files/FilePage.jsx'));
const Reader = lazy(() => import('../features/documents/Reader.jsx'));
const Cookbook = lazy(() => import('../features/recipes/Cookbook.jsx'));
const Workshop = lazy(() => import('../features/workshop/Workshop.jsx'));
const Room = lazy(() => import('../features/room/UnifiedChat.jsx'));
const BrokerChat = lazy(() => import('../features/room/BrokerChat.jsx'));
const twoFrames = () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
const rememberedLibrary = () => { try { return sessionStorage.getItem('library-pack') || undefined; } catch { return undefined; } };
const browserKey = () => window.location.pathname + window.location.search;

export default function App() {
  const location = useLocation();
  const key = location.pathname + location.search;
  const language = useMemo(() => resolveLanguage(), []);
  const t = useMemo(() => createTranslator(language), [language]);
  const navigation = useRef({ target: key, generation: 0 });
  const [navigationVersion, setNavigationVersion] = useState(0);
  const completion = useRef(null);
  const readiness = useRef(0);
  const [libraryPack, setLibraryPack] = useState(rememberedLibrary);
  const [lastArticle, setLastArticle] = useState(null);
  const session = useQuery({ queryKey: ['session'], queryFn: () => loadSessionInfo() });
  const preferences = useQuery({ queryKey: ['preferences', session.data?.csrf], enabled: session.data?.privateNotes === true, queryFn: async () => {
    const response = await fetch('/api/preferences', { credentials: 'same-origin', cache: 'no-store', headers: { 'X-Tonalli-Csrf': session.data.csrf } });
    if (!response.ok) throw new Error('preferences_unavailable');
    return response.json();
  } });
  useTheme(preferences.data?.theme);
  const build = useQuery({ queryKey: ['build'], queryFn: async () => {
    const response = await fetch('/assets/ui-build.json', { credentials: 'same-origin', cache: 'no-store' });
    if (!response.ok) throw new Error('build_metadata_unavailable');
    const metadata = await response.json();
    window.__perf.bundleKb = (metadata.initialJsBytes + metadata.initialCssBytes) / 1024;
    return metadata;
  } });

  const requestNavigation = useCallback((target, measure = true) => {
    const destination = new URL(target, window.location.origin);
    const targetKey = destination.pathname + destination.search;
    const generation = navigation.current.generation + 1;
    navigation.current = { target: targetKey, generation };
    readiness.current++;
    completion.current = measure ? { target: targetKey, generation, finish: startRouteSwitch(window.__perf) } : null;
    window.__ready = { route: destination.pathname, ready: false, error: null };
    setNavigationVersion(generation);
  }, []);
  const beginNavigation = useCallback((target, event) => {
    // Match Router's same-tab click contract before invalidating this tab.
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey
      || (event.currentTarget.target && event.currentTarget.target !== '_self')) return false;
    if (target !== browserKey()) requestNavigation(target);
    return true;
  }, [requestNavigation]);
  useLayoutEffect(() => {
    // BrowserHistory can advance while Suspense retains the previous feature.
    if (key !== browserKey()) return;
    if (navigation.current.target !== key) requestNavigation(key, window.__perf.firstRenderMs !== null);
    document.documentElement.lang = language;
    document.title = displayName;
  }, [key, language, requestNavigation]);
  useEffect(() => {
    const popped = () => {
      if (navigation.current.target !== browserKey()) requestNavigation(browserKey(), window.__perf.firstRenderMs !== null);
    };
    window.addEventListener('popstate', popped);
    return () => window.removeEventListener('popstate', popped);
  }, [requestNavigation]);

  const bindRoute = useCallback((feature) => {
    const generation = navigationVersion;
    const route = `/${feature}`;
    const valid = () => navigation.current.generation === generation && navigation.current.target === key
      && browserKey() === key && key.split('?', 1)[0] === route;
    return {
      markPending() {
        if (valid()) { readiness.current++; window.__ready = { route, ready: false, error: null }; }
      },
      markError(error) {
        if (valid()) { readiness.current++; window.__ready = { route, ready: false, error: String(error?.message || error) }; }
      },
      async markReady(details) {
        if (!valid()) return;
        const version = ++readiness.current;
        window.__ready = { route, ready: false, error: null };
        await twoFrames();
        if (!valid() || version !== readiness.current) return;
        if (window.__perf.firstRenderMs === null) window.__perf.firstRenderMs = performance.now();
        if (completion.current?.target === key && completion.current.generation === generation) {
          completion.current.finish();
          completion.current = null;
        }
        window.__ready = { route, ready: true, error: null, ...details };
      },
    };
  }, [key, navigationVersion]);
  useEffect(() => {
    if (session.error || build.error) {
      readiness.current++;
      window.__ready = { route: window.location.pathname, ready: false, error: String((session.error || build.error).message) };
    }
  }, [session.error, build.error]);
  useEffect(() => {
    if (libraryPack) { try { sessionStorage.setItem('library-pack', libraryPack); } catch {} }
  }, [libraryPack]);
  const value = { csrf: session.data?.csrf, language, t, libraryPack, setLibraryPack, lastArticle, setLastArticle,
    preferences: preferences.data, refreshPreferences: async () => { const result = await preferences.refetch(); if (result.error) throw result.error; },
    canGetContent: session.data?.privateContent === true,
    canManageMaps: session.data?.privateNotes === true,
    canReadProgress: session.data?.privateNotes === true,
    canBrowseShared: session.data?.sharedMedia === true,
    canHostTv: session.data?.privateTv === true,
    canManageAi: session.data?.privateAiManagement === true,
    canUseAi: session.data?.privateAi === true,
    canConnectRemote: session.data?.privateRemote === true,remoteLibrary:session.data?.remoteLibrary===true,
    canUseRoom:session.data?.lanRoom===true,privateDevice:session.data?.privateNotes===true,canUseMesh:session.data?.privateMesh===true,
    canPrepareSearch: session.data?.privateIndexPreparation === true,
    beginNavigation, bindRoute };
  const libraryHref = `/library${libraryPack ? `?${new URLSearchParams({ pack: libraryPack })}` : ''}`;

  if(session.data?.chatBroker)return <Suspense fallback={<p role="status">{t('app.loading')}</p>}><BrokerChat language={language} initialSession={{csrf:session.data.csrf,chat_user:session.data.chatUser}}/></Suspense>;
  return <AppContext.Provider value={value}>
    <AppLayout privateNotes={session.data?.privateNotes} privateAi={session.data?.privateAi} libraryHref={libraryHref}>
      <MusicPlayer>
      {session.isError || build.isError ? <p role="alert">{t('app.failed')}</p> : !session.data || !build.data
        ? <p role="status">{t('app.loading')}</p>
        : <Suspense key={location.pathname === '/notes' ? 'notes' : location.pathname === '/files' ? 'files' : location.pathname === '/chat' ? 'chat' : location.pathname === '/content' ? 'content' : 'public'} fallback={<p role="status">{t('app.loading')}</p>}>
          <Routes>
            <Route path="/" element={<Navigate replace to={libraryHref} />} />
            <Route path="/library" element={<Library />} />
            <Route path="/article" element={<Article />} />
            <Route path="/settings" element={<Settings />} />
            <Route path="/map" element={<MapRoute />} />
            <Route path="/content" element={session.data.privateContent === true
              ? <Content key={session.data.csrf} /> : <Navigate replace to={libraryHref} />} />
            <Route path="/notes" element={session.data.privateNotes === true
              ? <Notes key={session.data.csrf} /> : <Navigate replace to={libraryHref} />} />
            <Route path="/files" element={session.data.privateNotes === true ? <Files key={session.data.csrf}/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/music" element={session.data.privateNotes === true ? <Music/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/videos" element={session.data.privateNotes === true ? <Videos/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/gallery" element={session.data.privateNotes === true ? <Gallery/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/file" element={session.data.privateNotes === true ? <FilePage/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/reader" element={session.data.privateNotes === true ? <Reader/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/cookbook" element={session.data.privateNotes === true ? <Cookbook/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/workshop" element={session.data.privateNotes === true ? <Workshop/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/room" element={session.data.lanRoom===true?<Room/>:<Navigate replace to={libraryHref}/>} />
            <Route path="/shared" element={session.data.sharedMedia === true ? <FilePage shared/> : <Navigate replace to={libraryHref}/>} />
            <Route path="/chat" element={session.data.privateAi === true || session.data.privateAiManagement === true
              ? <Chat key={session.data.csrf} /> : <Navigate replace to={libraryHref} />} />
            <Route path="*" element={<Navigate replace to={`/library${location.search}`} />} />
          </Routes>
        </Suspense>}
      </MusicPlayer>
    </AppLayout>
  </AppContext.Provider>;
}
