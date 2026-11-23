import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useApp, useRouteReadiness } from '../../app/context.js';
import { createNotesClient } from './client.js';
import NotesSecurity from './NotesSecurity.jsx';

export default function Notes() {
  const { csrf, t } = useApp();
  const { markReady, markPending } = useRouteReadiness('notes');
  const client = useMemo(() => createNotesClient(csrf), [csrf]);
  const work = useRef({ generation: 0, controller: null });
  const [unlocked, setUnlocked] = useState(false);
  const [kind, setKind] = useState('passphrase');
  const [credential, setCredential] = useState('');
  const [newPassphrase, setNewPassphrase] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [recovery, setRecovery] = useState('');
  const [rows, setRows] = useState([]);
  const [selected, setSelected] = useState(null);
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [editingId, setEditingId] = useState(null), [confirmDelete, setConfirmDelete] = useState(false);
  const [notice, setNotice] = useState('');

  function cancel() {
    work.current.controller?.abort();
    const generation = ++work.current.generation;
    const controller = new AbortController();
    work.current.controller = controller;
    return { signal: controller.signal, current: () => generation === work.current.generation };
  }
  function clear() {
    setEditingId(null); setConfirmDelete(false); setNotice('');
    setNewPassphrase(''); setConfirmation(''); setRecovery('');
    setCredential(''); setRows([]); setSelected(null); setTitle(''); setBody(''); setQuery(''); setUnlocked(false);
  }
  function failed(failure) {
    if (failure.status === 423 || failure.status === 403 || (failure.status === 401 && failure.code !== 'unlock_failed')
      || ['notes_unavailable', 'invalid_response', 'rotation_uncertain'].includes(failure.code)) clear();
    setError(failure.code || 'notes_unavailable');
  }
  async function perform(operation) {
    const active = cancel();
    setBusy(true); setError(null);
    try { await operation(active); }
    catch (failure) { if (active.current() && failure.name !== 'AbortError') failed(failure); }
    finally { if (active.current()) setBusy(false); }
  }

  useEffect(() => {
    // Notes never enter the shared query cache. A departed session cannot repaint.
    clear();
    void perform(async (active) => {
      const notes = await client.list(active.signal);
      if (active.current()) { setRows(notes); setUnlocked(true); }
    });
    return () => {
      ++work.current.generation;
      work.current.controller?.abort();
      // Navigation/transport failure cannot promise delivery of this separate lock.
      void client.lock({ keepalive: true }).catch(() => {});
    };
  }, [client]);
  useEffect(() => { if (busy) markPending(); else void markReady(); }, [busy, markReady, markPending]);

  function unlock(event) {
    event.preventDefault();
    const secret = credential;
    setCredential('');
    void perform(async (active) => {
      try { await client.unlock(kind, secret, active.signal); }
      catch (failure) { if (failure.code !== 'already_unlocked') throw failure; }
      if (!active.current()) return;
      const notes = await client.list(active.signal);
      if (active.current()) { setRows(notes); setUnlocked(true); }
    });
  }
  function create(event) {
    event.preventDefault();
    if (newPassphrase !== confirmation) { setError('mismatch'); return; }
    const secret = newPassphrase;
    setNewPassphrase(''); setConfirmation('');
    void perform(async active => {
      const key = await client.create(secret, active.signal);
      if (active.current()) { setRecovery(key); setUnlocked(true); setRows([]); }
    });
  }
  function lock() {
    cancel(); clear(); setError(null); setBusy(true);
    const generation = work.current.generation;
    void client.lock().catch((failure) => {
      if (generation === work.current.generation) setError('lock_unconfirmed');
    }).finally(() => { if (generation === work.current.generation) setBusy(false); });
  }
  function search(event) {
    event.preventDefault();
    setSelected(null);
    void perform(async (active) => {
      const notes = await client.search(query, active.signal);
      if (active.current()) setRows(notes);
    });
  }
  function add(event) {
    event.preventDefault();
    void perform(async (active) => {
      let added;
      if (editingId) { await client.update(editingId, title, body, active.signal); added = { id: editingId }; }
      else added = await client.add(title, body, active.signal);
      if (!active.current()) return;
      setTitle(''); setBody(''); setQuery(''); setEditingId(null);
      const notes = await client.list(active.signal);
      if (!active.current()) return;
      const note = await client.get(added.id, active.signal);
      if (active.current()) { setRows(notes); setSelected(note); }
    });
  }
  function select(id) {
    setSelected(null); setConfirmDelete(false);
    void perform(async (active) => {
      const note = await client.get(id, active.signal);
      if (active.current()) setSelected(note);
    });
  }
  function remove() {
    const id = selected.id;
    void perform(async active => {
      await client.remove(id, active.signal);
      if (!active.current()) return;
      setSelected(null); setConfirmDelete(false);
      if (editingId === id) { setEditingId(null); setTitle(''); setBody(''); }
      const notes = await client.list(active.signal);
      if (active.current()) setRows(notes);
    });
  }
  const message = error && t(`notes.error.${error}`) !== `notes.error.${error}`
    ? t(`notes.error.${error}`) : t('notes.error.notes_unavailable');
  const input = 'block w-full rounded border border-slate-400 p-2';
  const button = 'min-h-11 rounded border border-slate-500 bg-slate-100 px-3 py-2 text-slate-900 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-slate-700';
  return <section aria-label={t('notes.heading')} className="space-y-4">
    <h2 className="text-lg font-semibold">{t('notes.heading')}</h2>
    {error && <p role="alert">{message}</p>}
    {busy && <p role="status">{t('notes.working')}</p>}
    {notice && <p role="status">{notice}</p>}
    {!unlocked ? <>
      {error === 'lock_unconfirmed' && <button className={button} data-testid="notes-retry-lock" onClick={lock} disabled={busy}>{t('notes.retryLock')}</button>}
      <p>{t('notes.initialize')}</p>
      <form onSubmit={unlock} autoComplete="off" className="space-y-3">
        <label>{t('notes.unlockWith')}<select className={input} value={kind} disabled={busy}
          onChange={(event) => { setKind(event.target.value); setCredential(''); }}>
          <option value="passphrase">{t('notes.passphrase')}</option>
          <option value="recovery_key">{t('notes.recovery')}</option>
        </select></label>
        <label>{t(kind === 'passphrase' ? 'notes.passphrase' : 'notes.recovery')}
          <input data-testid="notes-credential" className={input} type="password" value={credential} autoComplete="off"
            disabled={busy} onChange={(event) => setCredential(event.target.value)} /></label>
        <button className={button} data-testid="notes-unlock" disabled={busy || !credential} type="submit">{t('notes.unlock')}</button>
      </form>
      <details className="app-card" open={error === 'notes_not_initialized' ? true : undefined}><summary>{t('notes.create')}</summary>
        <p className="my-3">{t('notes.setupHelp')}</p>
        <form onSubmit={create} autoComplete="off" className="space-y-3">
          <label>{t('notes.passphrase')}<input className={input} type="password" value={newPassphrase} maxLength={4096} disabled={busy} onChange={event => setNewPassphrase(event.target.value)} /></label>
          <label>{t('notes.confirmPassphrase')}<input className={input} type="password" value={confirmation} maxLength={4096} disabled={busy} onChange={event => setConfirmation(event.target.value)} /></label>
          <button className={button} data-testid="notes-create" disabled={busy || !newPassphrase || !confirmation}>{t('notes.create')}</button>
        </form>
      </details>
    </> : <>
      {recovery && <div className="app-card space-y-4"><p>{t('notes.recoveryHelp')}</p><code data-testid="notes-recovery-key" className="block break-all select-all">{recovery}</code>
        <button className={button} onClick={() => setRecovery('')}>{t('notes.savedKey')}</button></div>}
      <button className={button} data-testid="notes-lock" onClick={lock}>{t('notes.lock')}</button>
      {!recovery && <>
      <NotesSecurity t={t} busy={busy} onExport={secret => { setNotice(''); void perform(async active => {
        const key = await client.exportRecovery(secret, active.signal); if (active.current()) setRecovery(key);
      }); }} onChange={(kind, credential, next) => { setNotice(''); void perform(async active => {
        await client.changePassphrase(kind, credential, next, active.signal); if (active.current()) setNotice(t('notes.passphraseChanged'));
      }); }} />
      <form onSubmit={search} className="flex gap-3">
        <label className="grow">{t('notes.search')}<input data-testid="notes-search" className={input} value={query}
          onChange={(event) => setQuery(event.target.value)} /></label>
        <button className={button} data-testid="notes-search-submit" disabled={busy} type="submit">{t('notes.find')}</button>
      </form>
      <ul className="space-y-2">{rows.map((note) => <li key={note.id}><button className={button} data-testid="notes-row" disabled={busy}
        onClick={() => select(note.id)}>{note.title}</button></li>)}</ul>
      {!rows.length && !busy && <p>{t('notes.empty')}</p>}
      {selected && <article data-testid="notes-selected"><h3 className="font-semibold">{selected.title}</h3>
        <p className="whitespace-pre-wrap">{selected.body}</p>
        <div className="my-3 flex flex-wrap gap-3"><button className={button} disabled={busy} data-testid="notes-edit" onClick={() => { setEditingId(selected.id); setTitle(selected.title); setBody(selected.body); }}>{t('notes.edit')}</button>
          <button className={button} disabled={busy} onClick={() => setConfirmDelete(true)}>{t('notes.delete')}</button></div>
        {confirmDelete && <div className="space-y-3"><p>{t('notes.deleteConfirm')}</p><button className={button} data-testid="notes-confirm-delete" disabled={busy} onClick={remove}>{t('notes.delete')}</button>
          <button className={button} onClick={() => setConfirmDelete(false)}>{t('notes.keep')}</button></div>}
        </article>}
      <form onSubmit={add} autoComplete="off" className="space-y-3">
        <h3 className="font-semibold">{t(editingId ? 'notes.edit' : 'notes.addHeading')}</h3>
        <label>{t('notes.title')}<input data-testid="notes-title" className={input} value={title}
          disabled={busy} onChange={(event) => setTitle(event.target.value)} /></label>
        <label>{t('notes.body')}<textarea data-testid="notes-body" className={input} value={body}
          disabled={busy} onChange={(event) => setBody(event.target.value)} /></label>
        <button className={button} data-testid="notes-add" disabled={busy} type="submit">{t(editingId ? 'notes.saveChanges' : 'notes.add')}</button>
        {editingId && <button className={button} type="button" disabled={busy} onClick={() => { setEditingId(null); setTitle(''); setBody(''); }}>{t('notes.cancelEdit')}</button>}
      </form>
      </>}
    </>}
  </section>;
}
