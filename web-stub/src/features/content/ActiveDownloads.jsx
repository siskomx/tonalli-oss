// Adapted from Project NOMAD admin/inertia/components/ActiveDownloads.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: Core authoritative states/bytes, explicit pause/resume/cancel/retry and per-action availability, EN/ES; removed URL/path display, clock-based stalled/ETA guesses and Redis APIs.
import React, { useState } from 'react';
import { IconAlertTriangle, IconLoader2 } from '@tabler/icons-react';
import { actionsFor, errorKey, progressFor } from './presentation.js';
import { formatBytes } from './format.js';

export default function ActiveDownloads({ jobs, titles, onAction, busy = false, disabledActions = [], t, language }) {
  const [confirmingCancel, setConfirmingCancel] = useState(null);
  return <div className="space-y-4">{jobs.map(job => {
    const progress = progressFor(job), actions = actionsFor(job.state);
    const percent = progress.max ? Math.min(100, progress.value / progress.max * 100) : 0;
    return <div key={job.id} className="rounded-lg border border-border-default bg-surface-primary p-4 shadow-sm">
      <div className="flex items-start justify-between gap-3"><h4 className="font-semibold text-desert-green">{Object.hasOwn(titles, job.entryId) ? titles[job.entryId] : job.entryId}</h4>
        <span className="flex items-center gap-2 text-sm" role="status">
          {['queued', 'downloading', 'verifying', 'installing', 'pausing', 'cancelling'].includes(job.state) && <IconLoader2 className="h-4 w-4 animate-spin" aria-hidden="true" />}
          {t(`content.state.${job.state}`)}</span></div>
      {job.state === 'failed' && <p role="alert" className="my-2 flex gap-2 text-sm text-red-800"><IconAlertTriangle className="h-4 w-4 shrink-0" aria-hidden="true" />{t(errorKey(job.errorCode, t))}</p>}
      {!['completed', 'cancelled', 'failed'].includes(job.state) && <>
        <p className="my-2 text-sm text-text-secondary">{formatBytes(progress.value, language)} / {formatBytes(progress.max, language)}</p>
        <div role="progressbar" aria-label={t(`content.state.${job.state}`)} aria-valuemin="0" {...(progress.max ? { 'aria-valuemax': progress.max, 'aria-valuenow': Math.min(progress.value, progress.max) } : {})}
          className="h-5 overflow-hidden rounded-lg border border-border-default bg-surface-secondary">
          <div className="h-full bg-desert-green transition-all" style={{ width: `${percent}%` }} /></div>
      </>}
      {actions.length > 0 && <div className="mt-3 flex flex-wrap gap-2">
        {confirmingCancel === job.id && actions.includes('cancel') ? <>
          <span className="w-full text-sm">{t('content.cancelConfirm')}</span>
          <button type="button" className="chat-control" disabled={busy} onClick={() => { setConfirmingCancel(null); onAction(job.id, 'cancel'); }}>{t('content.confirmCancel')}</button>
          <button type="button" className="chat-control" disabled={busy} onClick={() => setConfirmingCancel(null)}>{t('content.keep')}</button>
        </> : actions.map(action => <button type="button" key={action} className="chat-control" disabled={busy || disabledActions.includes(action)}
          onClick={() => action === 'cancel' ? setConfirmingCancel(job.id) : onAction(job.id, action)}>{t(`content.action.${action}`)}</button>)}
      </div>}
    </div>;
  })}</div>;
}
