// Adapted from Project NOMAD admin/inertia/components/chat/ChatSidebar.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: shared-layout details instead of a second fixed drawer; Root history,
// EN/ES and individual confirmed removal; no Inertia, server modals or branding.
import React, { useState } from 'react';
import { IconMessage } from '@tabler/icons-react';
import classNames from '../../ui/classNames.js';

export default function ConversationHistory({ history, busy, t, onOpen, onRemove }) {
  const [confirming, setConfirming] = useState(null);
  return <details className="app-card space-y-3">
    <summary className="cursor-pointer font-semibold min-h-11">{t('chat.savedConversations')}</summary>
    <p>{t('chat.historyPrivacy')}</p>
    {history.loading ? <p role="status">{t('app.loading')}</p> : !history.sessions.length && <p>{t('chat.historyEmpty')}</p>}
    <button type="button" className="secondary-button" disabled={busy} onClick={history.refresh}>{t('chat.historyRefresh')}</button>
    <ul className="max-h-80 overflow-y-auto space-y-3">{history.sessions.map(row => <li key={row.id} className="flex flex-wrap gap-3 items-center">
      <button type="button" disabled={busy} aria-current={history.session?.id === row.id ? 'true' : undefined}
        onClick={() => onOpen(row.id)} className={classNames('flex-1 min-w-0 min-h-11 text-left px-3 py-2 rounded-lg transition-colors group',
          history.session?.id === row.id ? 'bg-desert-green text-white' : 'hover:bg-surface-secondary text-text-primary')}>
        <div className="flex items-start gap-2"><IconMessage aria-hidden="true" className={classNames('h-5 w-5 mt-0.5 shrink-0',
          history.session?.id === row.id ? 'text-white' : 'text-text-muted')} />
          <div className="flex-1 min-w-0"><div className="font-medium text-sm break-words">{row.title}</div></div>
        </div>
      </button>
      <button type="button" className="secondary-button" disabled={busy} onClick={() => setConfirming(row)} aria-label={t('chat.deleteNamed', { title: row.title })}>{t('chat.deleteConversation')}</button>
    </li>)}</ul>
    {confirming && <div className="space-y-3"><p>{t('chat.confirmDelete', { title: confirming.title })}</p>
      <button type="button" className="chat-control" disabled={busy} onClick={async () => { if (await onRemove(confirming)) setConfirming(null); }}>{t('chat.deleteConversation')}</button>
      <button type="button" className="secondary-button" disabled={busy} onClick={() => setConfirming(null)}>{t('chat.keepConversation')}</button>
    </div>}
  </details>;
}
