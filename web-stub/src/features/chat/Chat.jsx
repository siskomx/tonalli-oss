// Adapted from Project NOMAD admin/inertia/components/chat/index.tsx streaming callbacks at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: local Core attachment, completed-turn context with visible in-memory transcript,
// request-owned cancellation/abort, stale callback fencing and private Core history; no Inertia/Ollama.
import React, { useEffect, useRef, useState } from 'react';
import { useApp, useRouteReadiness } from '../../app/context.js';
import { displayName } from '../../app/AppName.jsx';
import ChatInterface from './ChatInterface.jsx';
import { cancelChat, loadModel, streamChat } from './client.js';
import { abortActiveStream, clearStreamIfCurrent } from './stream_abort.js';
import { loadPacks } from '../library/client.js';
import useHistory from './useHistory.js';
import ConversationHistory from './ConversationHistory.jsx';
import { Link } from 'react-router-dom';

export default function Chat() {
  const { csrf, t, canManageAi } = useApp();
  const { markReady } = useRouteReadiness('chat');
  const [model, setModel] = useState(null);
  const [messages, setMessages] = useState([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [notice, setNotice] = useState(null);
  const [loading, setLoading] = useState(true);
  const [books, setBooks] = useState([]);
  const [booksLoading, setBooksLoading] = useState(true);
  const [pack, setPack] = useState('');
  const stream = useRef(null);
  const owned = useRef(null);
  const alive = useRef(true);
  const generation = useRef(0);
  const modelRequest = useRef(null);
  const sourceChosen = useRef(false);
  const history = useHistory({ csrf, enabled: canManageAi, t,
    onOpen(record, restored) { sourceChosen.current = true; setMessages(restored); setPack(record.pack ?? ''); setError(null); setNotice(null); },
    onClear() { stop(false); setBusy(false); setMessages([]); setError(null); setNotice(null); } });
  const disabled = busy || history.busy;

  function stop(update = true) {
    const current = owned.current;
    const stoppedGeneration = ++generation.current;
    owned.current = null;
    if (current?.started) {
      // Aborting the browser reader alone is not native cancellation.
      void cancelChat(csrf, current.id).catch(() => {
        if (update && alive.current && generation.current === stoppedGeneration) setNotice('chat.stopUnconfirmed');
      });
    }
    abortActiveStream(stream);
    if (update) {
      setBusy(false); setNotice('chat.stopRequested');
      setMessages((previous) => previous.map((message) => ({ ...message, isStreaming: false, isThinking: false })));
    }
  }
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; modelRequest.current?.abort(); stop(false); };
  }, [csrf]);
  useEffect(() => { void markReady({ available: Boolean(model) }); }, [markReady, model]);

  function refresh() {
    modelRequest.current?.abort();
    const controller = new AbortController();
    modelRequest.current = controller;
    setLoading(true); setError(null);
    loadModel(csrf, controller.signal).then((value) => {
      if (alive.current && !controller.signal.aborted) setModel(value);
    }).catch((failure) => {
      if (alive.current && !controller.signal.aborted) { setModel(null); setError(failure.message === 'chat.unavailable' ? null : failure.message.startsWith('chat.') ? failure.message : 'chat.failed'); }
    }).finally(() => { if (alive.current && !controller.signal.aborted) setLoading(false); });
    return controller;
  }
  useEffect(() => { const controller = refresh(); return () => controller.abort(); }, [csrf]);
  useEffect(() => {
    const controller = new AbortController();
    loadPacks(csrf, controller.signal).then(rows => {
      if (!controller.signal.aborted) { setBooks(rows); if (!sourceChosen.current) setPack(rows[0]?.id ?? ''); }
    }).catch(() => { if (!controller.signal.aborted) setError('chat.booksUnavailable'); })
      .finally(() => { if (!controller.signal.aborted) setBooksLoading(false); });
    return () => controller.abort();
  }, [csrf]);

  async function send(question) {
    if (!model || booksLoading || owned.current || history.isBusy() || history.session && history.session.model !== model.model) return;
    if (new TextEncoder().encode(question).length > (pack ? 4096 : 65536)) { setError('chat.questionTooLong'); return; }
    const id = crypto.randomUUID(), controller = new AbortController();
    const stamp = ++generation.current;
    owned.current = { id, started: false }; stream.current = controller;
    const current = () => alive.current && generation.current === stamp && owned.current?.id === id && !controller.signal.aborted;
    const assistantId = `${id}-assistant`;
    // Reuse NOMAD's role/content projection. Failed or stopped replies are visible but not context.
    const chatMessages = [...messages.filter(message => message.context).map(({ role, content }) => ({ role, content })),
      { role: 'user', content: question }];
    let isFirstChunk = true, isThinkingPhase = true, terminalReason = null, grounding = null, reply = '';
    const user = { id: `${id}-user`, role: 'user', content: question, timestamp: new Date() };
    setMessages((previous) => [...previous, user]);
    setBusy(true); setError(null); setNotice(null);
    try {
      if (canManageAi) await history.save([...messages, { ...user, context: false }], model.model, pack);
      if (!current()) return;
      owned.current.started = true;
      await streamChat({ requestId: id, model: model.model, messages: chatMessages, pack: pack || undefined, csrf, signal: controller.signal,
        onGrounding: value => { if (current()) grounding = value; },
        onChunk: (chunkContent, chunkThinking, done) => {
          if (!current()) return;
          reply += chunkContent;
          if (chunkContent.length > 0 || done) isThinkingPhase = false;
          if (isFirstChunk) {
            isFirstChunk = false;
            setMessages((previous) => [...previous, { id: assistantId, role: 'assistant', content: chunkContent,
              thinking: chunkThinking, timestamp: new Date(), isStreaming: !done,
              isThinking: isThinkingPhase && chunkThinking.length > 0, truncated: terminalReason === 'length', grounding }]);
          } else {
            setMessages((previous) => previous.map((message) => message.id === assistantId ? { ...message,
              content: message.content + chunkContent, thinking: (message.thinking ?? '') + chunkThinking,
              isStreaming: !done, isThinking: isThinkingPhase && Boolean(message.thinking || chunkThinking),
              truncated: terminalReason === 'length', grounding } : message));
          }
        },
        onDoneReason: (reason) => { if (current()) terminalReason = reason; },
      });
      if (current()) {
        const context = ['stop', 'length'].includes(terminalReason) && Boolean(reply.trim());
        setMessages(previous => previous.map(message => message.id === user.id || message.id === assistantId ? { ...message, context } : message));
        // Native generation ended. Saving is a separate operation with no Stop claim.
        owned.current = null; setBusy(false);
        if (canManageAi) {
          const saved = await history.save([...messages, { ...user, context }, { id: assistantId, role: 'assistant', content: reply,
            context, timestamp: new Date(), truncated: terminalReason === 'length', ...(grounding ? { grounding } : {}) }], model.model, pack);
          if (alive.current && generation.current === stamp && !controller.signal.aborted) setNotice(saved ? 'chat.historySaved' : 'chat.historySaveFailed');
        }
      }
    } catch (failure) {
      if (current()) {
        setError(['chat.session', 'chat.unavailable', 'chat.prepareFirst', 'chat.contextFull'].includes(failure.message) ? failure.message : 'chat.failed');
        setMessages((previous) => failure.message === 'chat.session' ? [] : previous.map((message) => ({ ...message, isStreaming: false, isThinking: false })));
        if (['chat.session', 'chat.unavailable'].includes(failure.message)) setModel(null);
      }
    } finally {
      if (current()) { owned.current = null; setBusy(false); }
      clearStreamIfCurrent(stream, controller);
    }
  }
  return <section className="chat-ui flex flex-col gap-3" aria-label={t('nav.chat')}>
    <h2 className="text-xl font-semibold">{t('nav.chat')}</h2>
    <button type="button" className="secondary-button self-start" disabled={disabled || !messages.length}
      onClick={() => { if (owned.current || history.isBusy()) return; history.reset(); setMessages([]); setError(null); setNotice(null); }}>{t('chat.newConversation')}</button>
    {canManageAi && <ConversationHistory history={history} busy={disabled} t={t}
      onOpen={id => { if (!owned.current) return history.open(id); }}
      onRemove={row => { if (!owned.current) return history.remove(row); return false; }} />}
    <p>{t('chat.independent')}</p><p className="text-sm">{t(pack ? 'chat.grounded' : 'chat.noRetrieval')}</p>
    <label>{t('chat.sourceBook')}<select value={pack} disabled={disabled || booksLoading || messages.length > 0} onChange={event => { sourceChosen.current = true; setPack(event.target.value); }}>
      <option value="">{t('chat.general')}</option>{pack && !books.some(book => book.id === pack) && <option value={pack}>{t('chat.missingBook')}</option>}{books.map(book => <option key={book.id} value={book.id}>{book.title}</option>)}
    </select></label>
    {pack && <a href={`/library?${new URLSearchParams({ pack })}`}>{t('chat.prepareBook')}</a>}
    <p role="status">{loading ? t('chat.loadingModel') : model ? t('chat.model', { model: model.model }) : t('chat.unavailable')}</p>
    {error && <p role="alert">{t(error)}</p>}{notice && <p role="status">{t(notice)}</p>}
    {history.error && <p role="alert">{t(history.error)}</p>}
    {history.busy && <p role="status">{t('chat.historySaving')}</p>}
    {notice === 'chat.historySaveFailed' && <button type="button" className="secondary-button self-start" disabled={disabled || !model}
      onClick={async () => { if (owned.current || history.isBusy()) return; const saved = await history.save(messages, model.model, pack); if (alive.current) setNotice(saved ? 'chat.historySaved' : 'chat.historySaveFailed'); }}>{t('chat.retrySave')}</button>}
    {history.session && model?.model !== history.session.model && <p>{t('chat.loadSavedModel', { model: history.session.model })}</p>}
    {(!model || history.session && model.model !== history.session.model) && <Link to="/settings?tab=updates">{t('models.openSettings')}</Link>}
    {!model && !loading && <button className="chat-control self-start" onClick={refresh}>{t('chat.retry')}</button>}
    <ChatInterface messages={messages} onSendMessage={send} isLoading={busy} disabled={!model || loading || booksLoading || history.busy || Boolean(history.session && model.model !== history.session.model)}
      onStop={() => stop()} assistantName={displayName} t={t} />
  </section>;
}
