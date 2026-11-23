// Adapted from Project NOMAD admin/inertia/components/StorageProjectionBar.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: actual available/additional backend budget instead of invented total disk/used space, EN/ES, backend-authoritative allowance.
import React from 'react';
import { IconAlertTriangle, IconServer } from '@tabler/icons-react';
import { errorKey, projectionFor } from './presentation.js';
import { formatBytes } from './format.js';

export default function StorageProjectionBar({ budget, t, language }) {
  const projection = projectionFor(budget), size = value => formatBytes(value, language);
  return <div className="rounded-lg border border-border-default bg-surface-secondary p-4">
    <h3 className="mb-3 flex items-center gap-2 font-semibold text-desert-green"><IconServer size={20} aria-hidden="true" />{t('content.storage')}</h3>
    <dl className="grid grid-cols-2 gap-2 text-sm">
      <dt>{t('content.available')}</dt><dd>{size(budget.availableBytes)}</dd>
      <dt>{t('content.remainingDownload')}</dt><dd>{size(budget.downloadBytes)}</dd>
      <dt>{t('content.additionalStaging')}</dt><dd>{size(budget.stagingBytes)}</dd>
      <dt>{t('content.required')}</dt><dd>{size(budget.requiredBytes)}</dd>
    </dl>
    <div className="my-3 h-6 overflow-hidden rounded-lg border border-border-default bg-white" aria-hidden="true">
      <div className={`h-full ${projection.allowed ? 'bg-desert-green' : 'bg-amber-600'}`} style={{ width: `${projection.percent}%` }} /></div>
    {projection.allowed ? <p className="text-sm">{t('content.projectedFree', { bytes: size(projection.remainingBytes) })}</p>
      : <p className="flex gap-2 text-sm text-amber-950"><IconAlertTriangle size={16} className="shrink-0" aria-hidden="true" />{t(errorKey(budget.reason, t))}</p>}
  </div>;
}
