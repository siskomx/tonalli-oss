// Adapted from Project NOMAD admin/inertia/components/chat/index.tsx session handlers at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: private Core revisions, local metadata, abort ownership and no server title-generation job.
import { useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { listConversations, openConversation, removeConversation, saveConversation, visibleMessages } from './history-client.js';

export default function useHistory({ csrf, enabled, t, onOpen, onClear }) {
  const cache = useQueryClient();
  const listing = useQuery({ queryKey: ['ai-history', csrf], enabled, retry: false,
    queryFn: ({ signal }) => listConversations(csrf, signal) });
  const [session, setSession] = useState(null), [busy, setBusy] = useState(false), [error, setError] = useState(null);
  const active = useRef(null), pending = useRef(null), alive = useRef(true);
  useEffect(() => { alive.current = true; return () => {
    alive.current = false; pending.current?.abort(); cache.removeQueries({ queryKey: ['ai-history', csrf] });
  }; }, [csrf]);
  useEffect(() => {
    if (error === 'chat.session' || listing.error?.message === 'chat.session') {
      active.current = null; setSession(null); onClear();
    }
  }, [error, listing.error?.message]);
  function reset() { active.current = null; setSession(null); setError(null); }
  async function run(work, commit) {
    if (!enabled || pending.current) return false;
    const controller = new AbortController(); pending.current = controller; setBusy(true); setError(null);
    try {
      const value = await work(controller.signal);
      if (!alive.current || controller.signal.aborted) return false;
      commit(value);
      void cache.invalidateQueries({ queryKey: ['ai-history', csrf] });
      return true;
    } catch (failure) {
      if (alive.current && !controller.signal.aborted) setError(failure.message.startsWith('chat.') ? failure.message : 'chat.historyFailed');
      return false;
    } finally {
      if (pending.current === controller) { pending.current = null; if (alive.current) setBusy(false); }
    }
  }
  function save(messages, model, pack) {
    const current = active.current;
    const id = current?.id ?? crypto.randomUUID().replaceAll('-', '');
    const title = current?.title ?? Array.from((messages.find(message => message.role === 'user')?.content ?? t('chat.conversation'))
      .replace(/\p{Cc}/gu, ' ').replace(/\s+/g, ' ').trim()).slice(0, 120).join('');
    return run(signal => saveConversation(csrf, id, { revision: current?.revision ?? 0, title, model,
      pack: pack || null, messages: visibleMessages(messages) }, signal), value => {
      active.current = value; setSession(value);
    });
  }
  function open(id) {
    return run(signal => openConversation(csrf, id, signal), value => {
      active.current = value; setSession(value);
      onOpen(value, value.messages.map((message, index) => ({ ...message, id: `${value.id}-${index}`, timestamp: new Date(message.timestamp) })));
    });
  }
  function remove(row) {
    return run(signal => removeConversation(csrf, row, signal), () => {
      if (active.current?.id === row.id) { reset(); onClear(); }
    });
  }
  return { session, busy, error: error ?? listing.error?.message,
    sessions: error === 'chat.session' || listing.error?.message === 'chat.session' ? [] : listing.data ?? [], loading: listing.isPending,
    refresh: () => listing.refetch(), isBusy: () => pending.current !== null, reset, save, open, remove };
}
