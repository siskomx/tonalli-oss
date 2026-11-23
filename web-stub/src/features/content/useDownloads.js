// Adapted from Project NOMAD admin/inertia/hooks/useDownloads.ts at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: private Core endpoint, CSRF-scoped cache and request abort; authoritative states, no Redis/Adonis coupling.
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { loadJobs } from './client.js';

export default function useDownloads({ csrf, enabled }) {
  const client = useQueryClient();
  const queryKey = ['content-jobs', csrf];
  const query = useQuery({ queryKey, queryFn: ({ signal }) => loadJobs(csrf, signal), enabled,
    retry: false, refetchInterval: current => current.state.data?.jobs.some(job =>
      ['queued', 'downloading', 'verifying', 'installing', 'pausing', 'cancelling', 'waiting_wifi'].includes(job.state)) ? 2000 : 3000 });
  return { ...query, invalidate: () => client.invalidateQueries({ queryKey }) };
}
