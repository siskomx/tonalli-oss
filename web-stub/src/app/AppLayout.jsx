/* Adapted from Project NOMAD StyledSidebar.tsx, AppLayout.tsx and SettingsLayout.tsx.
 * Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
 * Modified for React Router, local sessions, EN/ES and native pickers; no server services or branding. */
import React, { useEffect, useRef, useState } from 'react';
import { Link, useLocation } from 'react-router-dom';
import { IconBooks, IconDownload, IconMap, IconNotebook, IconSettings, IconMenu2, IconX, IconSun, IconFiles, IconMusic, IconMovie,IconPhoto,IconShare } from '@tabler/icons-react';
import { useApp } from './context.js';
import AppName from './AppName.jsx';

export const nativeShell = () => /TonalliNative\//.test(navigator.userAgent);

export default function AppLayout({ children, privateNotes, privateAi, libraryHref }) {
  const { t, canGetContent, canBrowseShared, canUseRoom,canManageAi,beginNavigation, lastArticle, preferences,remoteLibrary } = useApp();
  const { pathname } = useLocation();
  const drawer = useRef(null);
  const [mobile, setMobile] = useState(() => matchMedia('(max-width: 900px)').matches);
  useEffect(() => {
    const media = matchMedia('(max-width: 900px)');
    const update = () => setMobile(media.matches);
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, []);
  const items = [
    ['library', libraryHref, IconBooks],
    ...(canGetContent ? [['content', '/content', IconDownload]] : []),
    ...(preferences?.show_maps !== false ? [['map', '/map', IconMap]] : []),
    ...(privateNotes && preferences?.show_notes !== false ? [['notes', '/notes', IconNotebook]] : []),
    ...(privateNotes ? [['files', '/files', IconFiles]] : []),
    ...(privateNotes ? [['music', '/music', IconMusic]] : []),
    ...(privateNotes ? [['videos', '/videos', IconMovie]] : []),
    ...(privateNotes ? [['gallery', '/gallery', IconPhoto]] : []),
    ...(privateNotes ? [['reader', '/reader', IconBooks]] : []),
    ...(privateNotes ? [['cookbook', '/cookbook', IconNotebook]] : []),
    ...(privateNotes ? [['workshop', '/workshop', IconFiles]] : []),
    ...(canBrowseShared ? [['shared','/shared',IconShare]] : []),
    ...(canUseRoom ? [['room','/room',IconShare]] : []),
    ...(privateAi || canManageAi ? [['chat', '/chat', IconSun]] : []),
    ['settings', '/settings', IconSettings],
  ];
  const sidebar = () => <div className="sidebar-content">
    <Link to={libraryHref} className="app-brand" onClick={event => { beginNavigation(libraryHref, event); drawer.current?.close(); }}>
      <IconSun size={32} aria-hidden="true" /><span><AppName /></span>
    </Link>
    <nav aria-label={t('app.navigation')}><ul className="space-y-1">{items.map(([name, href, Icon]) => {
      const current = pathname === href.split('?')[0] || (name === 'library' && pathname === '/article');
      return <li key={name}><Link data-testid={`nav-${name}`} to={href} aria-current={current ? 'page' : undefined}
        className={`sidebar-item ${current ? 'sidebar-current' : ''}`}
        onClick={event => { beginNavigation(href, event); drawer.current?.close(); }}>
        <Icon size={24} aria-hidden="true" />{t(`nav.${name}`)}</Link></li>;
    })}</ul></nav>
    <p className="sidebar-caption">{t('app.offline')}</p>
  </div>;
  return <div className="app-layout">
    {!mobile && <aside className="desktop-sidebar">{sidebar()}</aside>}
    <header className="mobile-header"><button className="icon-button" onClick={() => drawer.current.showModal()} aria-label={t('app.menu')}><IconMenu2 /></button><AppName /></header>
    {mobile && <dialog ref={drawer} className="mobile-drawer" onClick={event => { if (event.target === drawer.current) drawer.current.close(); }}>
      <button className="icon-button drawer-close" onClick={() => drawer.current.close()} aria-label={t('app.closeMenu')}><IconX /></button>{sidebar()}
    </dialog>}
    <main id="main-content" className="app-main">
      {remoteLibrary && <p className="app-card my-3">{t('remote.connected')} <Link className="underline" to="/settings?tab=sharing">{t('remote.connectionSettings')}</Link></p>}
      {pathname === '/article' && <Link className="reading-return" to={libraryHref} onClick={event => beginNavigation(libraryHref, event)}>← {t('library.heading')}</Link>}
      {pathname === '/library' && lastArticle && <Link className="reading-return" to={`/article?${new URLSearchParams(lastArticle)}`} onClick={event => beginNavigation(`/article?${new URLSearchParams(lastArticle)}`, event)}>{t('library.continue')} →</Link>}
      {children}
    </main>
  </div>;
}
