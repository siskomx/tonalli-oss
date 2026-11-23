import React from 'react';
import { useApp, useRouteReadiness } from './context.js';
import { nativeShell } from './AppLayout.jsx';
import { iosShell } from './platform.js';
import { Link, useLocation } from 'react-router-dom';
import Tabs from '../ui/Tabs.jsx';
import Sharing from './Sharing.jsx';
import TvSharing from './TvSharing.jsx';
import AdminSettings from './AdminSettings.jsx';
import RemoteLibrary from './RemoteLibrary.jsx';
import NotificationNotice from './NotificationNotice.jsx';
import ChatConnections from '../features/room/ChatConnections.jsx';
import UpdateSettings from '../features/update/UpdateSettings.jsx';
const ModelSettings = React.lazy(() => import('../features/chat/ModelSettings.jsx'));

export default function Settings() {
  const { t, language, canManageMaps, canHostTv, canManageAi } = useApp();
  const { markReady } = useRouteReadiness('settings');
  // Other screens link to a specific card, so /settings?tab=sharing|updates|advanced opens that tab.
  const requested = new URLSearchParams(useLocation().search).get('tab');
  React.useEffect(() => { void markReady(); }, [markReady]);
  const general = <>
    <div className="app-card space-y-4"><h3>{t('settings.language')}</h3>
      <p>{language === 'es' ? 'Español' : 'English'}</p><p>{t('settings.deviceLanguage')}</p>
      <p>{t('settings.private')}</p>
    </div>
    <NotificationNotice />
    {canManageMaps && <>
      <div className="app-card space-y-3"><h3>{t('settings.content')}</h3><p>{t('settings.contentHelp')}</p>
        <div className="flex flex-wrap gap-3"><Link className="chat-control" to="/content">{t('nav.content')}</Link><Link className="secondary-button" to="/map">{t('nav.map')}</Link></div></div>
      <div className="app-card space-y-3"><h3>{t('settings.notes')}</h3><p>{t('settings.notesHelp')}</p><Link className="secondary-button" to="/notes">{t('nav.notes')}</Link></div>
    </>}
    <div className="app-card space-y-3"><h3>{t('settings.about')}</h3><p>{t('settings.aboutText')}</p>
      <a className="underline" href="/assets/project-nomad-NOTICE.txt">{t('settings.notices')}</a></div>
  </>;
  const sharing = <>
    {canManageMaps && <Sharing />}
    {canManageMaps && canHostTv && <TvSharing />}
    <RemoteLibrary />
    <ChatConnections />
  </>;
  const updates = <>
    {canManageAi && <React.Suspense fallback={<p role="status">{t('app.loading')}</p>}><ModelSettings /></React.Suspense>}
    {canManageMaps && <UpdateSettings />}
  </>;
  const advanced = <>
    <AdminSettings />
    <details className="app-card"><summary>{t('settings.advanced')}</summary>
      <p className="my-4">{t('settings.advancedHelp')}</p>
      {nativeShell() && !iosShell() && <a className="chat-control inline-flex" href="/native/advanced">{t('settings.openAdvanced')}</a>}
    </details>
  </>;
  const tabs = [
    { id: 'general', label: t('settings.tab.general'), content: general },
    { id: 'sharing', label: t('settings.tab.sharing'), content: sharing },
    ...(canManageAi || canManageMaps ? [{ id: 'updates', label: t('settings.tab.updates'), content: updates }] : []),
    ...(canManageMaps ? [{ id: 'advanced', label: t('settings.tab.advanced'), content: advanced }] : []),
  ];
  return <section className="space-y-6">
    <h2>{t('nav.settings')}</h2>
    <Tabs tabs={tabs} label={t('settings.tabs')} initial={requested || 'general'} />
  </section>;
}
