import React, { useEffect, useRef, useState } from 'react';
import { useApp } from './context.js';

export default function AdminSettings({ children }) {
  const { csrf, t, preferences, refreshPreferences } = useApp();
  const [configured, setConfigured] = useState(null), [token, setToken] = useState('');
  const [secret, setSecret] = useState(''), [confirmation, setConfirmation] = useState('');
  const [draft, setDraft] = useState(null), [busy, setBusy] = useState(false), [error, setError] = useState(''), [saved, setSaved] = useState(false);
  const active = useRef(false);
  async function request(path, method = 'GET', body, admin = '') {
    const response = await fetch(`/api/${path}`, { method, credentials: 'same-origin', cache: 'no-store',
      headers: { 'X-Tonalli-Csrf': csrf, ...(admin ? {'X-Tonalli-Admin': admin} : {}), ...(body ? {'Content-Type':'application/json'} : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}) });
    if (!response.ok) {
      if (response.status === 423) setToken('');
      throw new Error(response.status === 401 ? 'wrong' : response.status === 429 ? 'wait' : response.status === 409 ? 'changed' : 'unavailable');
    }
    return response.json();
  }
  useEffect(() => {
    active.current = true;
    request('admin').then(value => { if (active.current) setConfigured(value.configured); }).catch(() => { if (active.current) setError('unavailable'); });
    return () => {
      active.current = false;
      void fetch('/api/admin/lock', {method:'POST', credentials:'same-origin', keepalive:true, headers:{'X-Tonalli-Csrf':csrf}}).catch(() => {});
    };
  }, [csrf]);
  useEffect(() => { if (preferences) setDraft(preferences); }, [preferences]);
  async function perform(work) {
    setBusy(true); setError(''); setSaved(false);
    try { await work(); } catch (failure) { if (active.current) setError(failure.message); }
    finally { if (active.current) setBusy(false); }
  }
  function unlock(event) {
    event.preventDefault();
    if (!configured && secret !== confirmation) { setError('mismatch'); return; }
    const passphrase = secret; setSecret(''); setConfirmation('');
    void perform(async () => {
      const value = await request(`admin/${configured ? 'login' : 'setup'}`, 'POST', {passphrase});
      if (!active.current) { await request('admin/lock', 'POST'); return; }
      setToken(value.token); setConfigured(true); await refreshPreferences();
    });
  }
  return <section className="app-card space-y-4" data-testid="admin-settings">
    <h3>{t('admin.heading')}</h3><p>{t('admin.help')}</p>
    {error && <p role="alert">{t(`admin.error.${error}`)}</p>}
    {saved && <p role="status">{t('admin.saved')}</p>}
    {configured === null ? <p role="status">{t('app.loading')}</p> : !token ? <form className="space-y-4" onSubmit={unlock}>
      <p>{t(configured ? 'admin.loginHelp' : 'admin.setupHelp')}</p>
      <label className="block">{t('admin.password')}<input className="block border rounded p-2 w-full" type="password" autoComplete={configured ? 'current-password' : 'new-password'} minLength={12} maxLength={1024} required value={secret} onChange={event => setSecret(event.target.value)} /></label>
      {!configured && <label className="block">{t('admin.confirm')}<input className="block border rounded p-2 w-full" type="password" autoComplete="new-password" minLength={12} maxLength={1024} required value={confirmation} onChange={event => setConfirmation(event.target.value)} /></label>}
      <button className="chat-control" disabled={busy}>{t(configured ? 'admin.unlock' : 'admin.create')}</button>
    </form> : <>
      {draft && !children && <form className="space-y-4" onSubmit={event => { event.preventDefault(); void perform(async () => {
        await request('preferences', 'PUT', draft, token); await refreshPreferences(); if (active.current) setSaved(true);
      }); }}>
        <div><label htmlFor="admin-theme" className="block">{t('admin.theme')}</label><select id="admin-theme" className="block border rounded p-2" value={draft.theme} onChange={event => setDraft({...draft,theme:event.target.value})}>
          {['system','light','dark'].map(theme => <option key={theme} value={theme}>{t(`admin.theme.${theme}`)}</option>)}
        </select></div>
        <fieldset className="space-y-3"><legend>{t('admin.menu')}</legend>
          <label className="flex gap-3 items-center"><input type="checkbox" checked={draft.show_maps} onChange={event => setDraft({...draft,show_maps:event.target.checked})} />{t('nav.map')}</label>
          <label className="flex gap-3 items-center"><input type="checkbox" checked={draft.show_notes} onChange={event => setDraft({...draft,show_notes:event.target.checked})} />{t('nav.notes')}</label>
        </fieldset>
        <p>{t('admin.menuHelp')}</p><button className="chat-control" disabled={busy}>{t('admin.save')}</button>
      </form>}
      <button className="secondary-button" disabled={busy} onClick={() => {setToken(''); void perform(() => request('admin/lock','POST'));}}>{t('admin.lock')}</button>
      <p>{t('admin.session')}</p>
    </>}
    {children?.({token,onLocked:()=>setToken('')})}
  </section>;
}
