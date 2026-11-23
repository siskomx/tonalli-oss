import React, { useState } from 'react';

// Mounted only while this vault is unlocked; credentials never enter shared storage.
export default function NotesSecurity({ t, busy, onExport, onChange }) {
  const [kind, setKind] = useState('passphrase');
  const [credential, setCredential] = useState('');
  const [next, setNext] = useState(''), [repeat, setRepeat] = useState('');
  const input = 'block w-full rounded border border-border-default p-3';
  const clear = () => { setCredential(''); setNext(''); setRepeat(''); };
  return <details className="app-card space-y-4"><summary>{t('notes.protection')}</summary>
    <form autoComplete="off" className="space-y-4" onSubmit={event => {
      event.preventDefault(); const old = credential, value = next; clear(); onChange(kind, old, value);
    }}>
      <label>{t('notes.confirmWith')}<select className={input} value={kind} disabled={busy} onChange={event => { setKind(event.target.value); clear(); }}>
        <option value="passphrase">{t('notes.passphrase')}</option><option value="recovery_key">{t('notes.recovery')}</option>
      </select></label>
      <label>{t(kind === 'passphrase' ? 'notes.currentPassphrase' : 'notes.recovery')}<input className={input} type="password" autoComplete="off" maxLength={4096} value={credential} disabled={busy} onChange={event => setCredential(event.target.value)} /></label>
      <label>{t('notes.newPassphrase')}<input className={input} type="password" autoComplete="off" maxLength={4096} value={next} disabled={busy} onChange={event => setNext(event.target.value)} /></label>
      <label>{t('notes.confirmPassphrase')}<input className={input} type="password" autoComplete="off" maxLength={4096} value={repeat} disabled={busy} onChange={event => setRepeat(event.target.value)} /></label>
      {repeat && next !== repeat && <p role="status">{t('notes.mismatch')}</p>}
      <div className="flex flex-wrap gap-3">
        <button className="chat-control" data-testid="notes-change-passphrase" disabled={busy || !credential || !next || next !== repeat}>{t('notes.changePassphrase')}</button>
        {kind === 'passphrase' && <button className="secondary-button" type="button" data-testid="notes-export-recovery" disabled={busy || !credential} onClick={() => { const secret = credential; clear(); onExport(secret); }}>{t('notes.exportRecovery')}</button>}
      </div>
      <p>{t('notes.recoveryPrivate')}</p>
    </form>
  </details>;
}
