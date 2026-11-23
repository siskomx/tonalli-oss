import React, { useEffect, useState } from 'react';
import { useApp } from './context.js';
import Input from '../ui/Input.jsx';
import { inspectInvitation } from './remote-invitation.js';

export default function RemoteLibrary() {
  const { csrf, t, canConnectRemote, remoteLibrary } = useApp();
  const [invitation, setInvitation] = useState(''), [identity, setIdentity] = useState(null), [verified, setVerified] = useState(false);
  const [code, setCode] = useState(''), [label, setLabel] = useState(''), [busy, setBusy] = useState(false), [error, setError] = useState(false);
  useEffect(() => {
    let current = true; setIdentity(null); setVerified(false);
    if (invitation) void inspectInvitation(invitation).then(value => { if (current) setIdentity(value); }).catch(() => {});
    return () => { current = false; };
  }, [invitation]);
  if (!canConnectRemote) return null;
  async function perform(disconnect = false) {
    setBusy(true); setError(false);
    try {
      const response = await fetch(`/api/remote/${disconnect ? 'disconnect' : 'connect'}`, { method: 'POST', credentials: 'same-origin', cache: 'no-store',
        headers: { 'Content-Type': 'application/json', 'X-Tonalli-Csrf': csrf }, body: JSON.stringify(disconnect ? {} : { invitation, code, label: label.trim() }) });
      if (!response.ok) throw Error('remote_unavailable');
      try { sessionStorage.removeItem('library-pack'); } catch {}
      window.location.assign('/library');
    } catch { setError(true); setBusy(false); }
  }
  return <div className="app-card space-y-4" data-testid="remote-library">
    <h3>{t('remote.title')}</h3><p>{t('remote.help')}</p>
    {error && <p role="alert">{t('remote.failed')}</p>}
    {remoteLibrary ? <><p role="status">{t('remote.connected')}</p><button className="chat-control" disabled={busy} onClick={() => perform(true)}>{t('remote.disconnect')}</button></> : <>
      <label className="block">{t('remote.invitation')}<textarea name="library-invitation" rows="3" maxLength={66000} value={invitation} onChange={event => setInvitation(event.target.value.trim())} className="block w-full rounded border p-3 break-all" /></label>
      {identity && <div className="space-y-3"><p className="break-all">{identity.address}</p><p>{t('remote.verifyHelp')}</p><code className="block break-all">{identity.fingerprint}</code>
        <label className="flex gap-3 items-start"><input type="checkbox" checked={verified} onChange={event => setVerified(event.target.checked)} />{t('remote.verified')}</label></div>}
      <Input name="library-pairing-code" label={t('sharing.code')} value={code} maxLength={6} inputMode="numeric" autoComplete="off" onChange={event => setCode(event.target.value.replace(/\D/g, '').slice(0, 6))} />
      <Input name="library-device-label" label={t('remote.deviceName')} value={label} maxLength={64} autoComplete="off" onChange={event => setLabel(event.target.value)} />
      <button className="chat-control" disabled={busy || identity?.invitation !== invitation || !verified || !/^\d{6}$/.test(code) || !label.trim()} onClick={() => perform()}>{t(busy ? 'remote.connecting' : 'remote.connect')}</button>
      {busy && <button className="secondary-button" onClick={() => perform(true)}>{t('content.action.cancel')}</button>}
    </>}
  </div>;
}
