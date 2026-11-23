// Adapted from Project NOMAD admin/inertia/components/drug-reference/IngestStatus.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: PhaseBlock presentation uses Core phases/counters and EN/ES; removed FDA data, estimated percentages/ETA, clock and server polling.
import React from 'react';

const h = React.createElement;
export const preparationActive = state => ['queued', 'preparing', 'pausing', 'cancelling'].includes(state);
const failures = new Set(['no_space', 'media_removed', 'source_unavailable', 'stale_source', 'publication_uncertain', 'busy', 'session', 'request_unconfirmed']);
export const preparationErrorKey = reason => `library.prepare.error.${failures.has(reason) ? reason : 'failed'}`;

export default function SearchPreparationView({ status, loading, error, busy, onAction, t, language }) {
  const state = status?.state;
  const running = preparationActive(state);
  const completed = state === 'ready';
  const failed = state === 'failed' || state === 'stale_source';
  const accent = failed ? 'text-red-700' : completed ? 'text-green-700' : running ? 'text-blue-700' : 'text-text-secondary';
  const actions = !status ? [] : ['unprepared', 'cancelled'].includes(state) ? ['start']
    : ['paused', 'failed'].includes(state) ? ['resume', 'cancel']
      : ['queued', 'preparing'].includes(state) ? ['pause', 'cancel'] : state === 'stale_source' ? ['cancel'] : [];
  const title = loading && !status ? t('library.prepare.loading')
    : state === 'preparing' && status.phase ? t(`library.prepare.phase.${status.phase}`)
      : t(`library.prepare.state.${state || 'unprepared'}`);
  return h('section', { className: 'my-4 rounded-lg border border-border-default p-4 space-y-3', 'aria-label': t('library.prepare.title') },
    h('div', { className: 'space-y-1', role: 'status' },
      h('div', { className: 'flex items-baseline justify-between gap-3' },
        h('span', { className: `text-sm font-semibold ${accent}` }, title, running ? ' …' : '')),
      h('p', { className: 'text-xs text-text-secondary' }, t(completed ? 'library.prepare.readyHint' : 'library.prepare.hint')),
      status?.committedArticles !== null && status?.committedArticles !== undefined
        && h('div', { className: 'text-xs text-text-secondary tabular-nums' },
          t('library.prepare.count', { count: status.committedArticles.toLocaleString(language) })),
      running && h('progress', { 'aria-label': title, className: 'w-full h-2' }),
      failed && h('p', { className: 'text-xs text-red-700' }, t(preparationErrorKey(state === 'stale_source' ? state : status.reason)))),
    error && h('p', { role: 'alert', className: 'text-sm text-red-700' }, t(preparationErrorKey(error.code))),
    actions.length > 0 && h('div', { className: 'flex flex-wrap gap-2' }, actions.map(action =>
      h('button', { type: 'button', className: 'chat-control', key: action, disabled: busy || loading,
        onClick: () => onAction(action) }, t(`library.prepare.action.${action}`)))));
}
