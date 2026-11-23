// Adapted from Project NOMAD admin/inertia/components/WikipediaSelector.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: multiple independent installed packs, Core metadata only, accessible selection buttons, EN/ES; no URL authority or single-Wikipedia removal.
import React from 'react';
import { IconCheck, IconWorld, IconAlertTriangle } from '@tabler/icons-react';
import classNames from '../../ui/classNames.js';
import { formatBytes } from './format.js';

export default function ContentSelector({ entries, selectedId, onSelect, disabled = false, t, language }) {
  return <div className="w-full">
    <div className="mb-4 flex items-center gap-3"><IconWorld className="h-6 w-6 text-desert-green" aria-hidden="true" />
      <h3 className="text-xl font-semibold text-text-primary">{t('content.catalog')}</h3></div>
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
      {entries.map(option => {
        const selected = selectedId === option.id;
        const installed = option.installedVersion !== null;
        const ready = installed && option.installedReadable === true;
        return <button key={option.id} type="button" aria-pressed={selected} disabled={disabled}
          onClick={() => onSelect(option.id)}
          className={classNames('relative flex min-h-44 flex-col rounded-lg border-2 p-4 text-left transition-all',
            disabled ? 'cursor-not-allowed opacity-60' : 'hover:shadow-md focus-visible:outline-2 focus-visible:outline-offset-2',
            selected ? 'border-desert-green bg-green-50' : 'border-border-subtle bg-surface-primary hover:border-border-default')}>
          <span className="mb-2 flex flex-wrap gap-2 text-xs">
            {ready && <span className="flex items-center gap-1 rounded-full bg-desert-green px-2 py-1 text-white"><IconCheck size={12} aria-hidden="true" />{t('content.ready')}</span>}
            {installed && !ready && <span className="flex items-center gap-1 rounded-full bg-amber-100 px-2 py-1 text-amber-950"><IconAlertTriangle size={12} aria-hidden="true" />
              {t(option.installedReadable === false ? 'content.unavailable' : 'content.unchecked')}</span>}
            {selected && <span className="rounded-full bg-lime-100 px-2 py-1 text-lime-950">{t('content.selected')}</span>}
          </span>
          <span className="mb-1 text-lg font-semibold text-text-primary">{option.title}</span>
          <span className="mb-3 grow text-sm text-text-secondary">{t(`content.kind.${option.kind}`)} · {option.language.toUpperCase()} · {option.version}</span>
          {installed && <span className="mb-2 text-sm text-text-secondary">{t('content.installedVersion', { version: option.installedVersion })}</span>}
          <span className="flex items-center gap-3"><span aria-hidden="true" className={classNames('flex h-5 w-5 items-center justify-center rounded-full border-2',
            selected ? 'border-desert-green bg-desert-green' : 'border-border-default')}>{selected && <IconCheck size={12} className="text-white" />}</span>
            <span className="rounded bg-surface-secondary px-2 py-1 text-sm font-medium">{formatBytes(option.bytes, language)}</span></span>
        </button>;
      })}
    </div>
  </div>;
}
