import React, { useEffect, useState } from 'react';
import { useApp } from './context.js';
import { nativeShell } from './AppLayout.jsx';

export async function notificationsEnabled(fetchState = fetch, signal) {
  try {
    const response = await fetchState('/native/notifications/state', { cache: 'no-store', signal });
    if (!response.ok) return null;
    const state = await response.json();
    return typeof state?.enabled === 'boolean' ? state.enabled : null;
  } catch { return null; }
}

export function NotificationNoticeView({ t }) {
  return <div className="app-card space-y-3" role="status"><h3>{t('notifications.title')}</h3><p>{t('notifications.help')}</p>
    <a className="chat-control inline-flex" href="/native/notifications">{t('notifications.enable')}</a></div>;
}

export default function NotificationNotice() {
  const { t } = useApp();
  const supported = nativeShell() && /Android/.test(navigator.userAgent);
  const [enabled, setEnabled] = useState(null);
  useEffect(() => {
    if (!supported) return undefined;
    const controller = new AbortController();
    const refresh = () => { void notificationsEnabled(fetch, controller.signal).then(value => { if (!controller.signal.aborted) setEnabled(value); }); };
    refresh();
    const timer = setInterval(refresh, 2000);
    document.addEventListener('visibilitychange', refresh);
    window.addEventListener('focus', refresh);
    return () => { controller.abort(); clearInterval(timer); document.removeEventListener('visibilitychange', refresh); window.removeEventListener('focus', refresh); };
  }, [supported]);
  return supported && enabled === false ? <NotificationNoticeView t={t} /> : null;
}
