// Adapted from Project NOMAD admin/inertia/components/chat/ChatInterface.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JSX/translated props instead of Inertia; text-only composer; no upload,
// suggestions or history-dependent Continue; explicit Stop and unavailable-model state.
import React, { useState, useRef, useEffect } from 'react';
import { IconSend, IconWand } from '@tabler/icons-react';
import classNames from '../../ui/classNames.js';
import ChatMessageBubble from './ChatMessageBubble.jsx';
import ChatAssistantAvatar from './ChatAssistantAvatar.jsx';
import BouncingDots from './BouncingDots.jsx';

export default function ChatInterface({ messages, onSendMessage, isLoading = false, disabled = false, onStop, assistantName, t }) {
  const [input, setInput] = useState('');
  const messagesEndRef = useRef(null);
  const textareaRef = useRef(null);
  useEffect(() => { messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' }); }, [messages]);
  const handleSubmit = (event) => {
    event.preventDefault();
    if (input.trim() && !isLoading && !disabled) {
      onSendMessage(input.trim()); setInput('');
      if (textareaRef.current) textareaRef.current.style.height = 'auto';
    }
  };
  const handleKeyDown = (event) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); handleSubmit(event); }
  };
  const handleInput = (event) => {
    setInput(event.target.value); event.target.style.height = 'auto';
    event.target.style.height = `${Math.min(event.target.scrollHeight, 200)}px`;
  };
  return <div className="flex-1 flex flex-col min-h-0 bg-surface-primary shadow-sm">
    <div className="flex-1 overflow-y-auto px-6 py-4 space-y-6" aria-live="polite" aria-relevant="additions text">
      {messages.length === 0 ? <div className="h-full flex items-center justify-center"><div className="text-center max-w-md">
        <IconWand className="h-16 w-16 text-desert-green mx-auto mb-4 opacity-50" aria-hidden="true" />
        <h3 className="text-lg font-medium text-text-primary mb-2">{t('chat.start')}</h3>
        <p className="text-text-muted text-sm">{t('chat.independent')}</p>
      </div></div> : <>{messages.map((message) => <div key={message.id} className={classNames('flex gap-4', message.role === 'user' ? 'justify-end' : 'justify-start')}>
        {message.role === 'assistant' && <ChatAssistantAvatar />}<ChatMessageBubble message={message} t={t} />
      </div>)}</>}
      {isLoading && <div className="flex gap-4 justify-start"><ChatAssistantAvatar /><div className="max-w-[85%] sm:max-w-[70%] rounded-lg px-4 py-3 bg-surface-secondary text-text-primary">
        <BouncingDots text={t('chat.generating')} />
      </div></div>}
      <div ref={messagesEndRef} />
    </div>
    <div className="border-t border-border-subtle bg-surface-primary px-6 py-4 flex-shrink-0 min-h-[90px]">
      <form onSubmit={handleSubmit} className="flex gap-3 items-end">
        <div className="flex-1 relative"><textarea ref={textareaRef} value={input} onChange={handleInput} onKeyDown={handleKeyDown}
          aria-label={t('chat.question')} placeholder={t('chat.placeholder', { name: assistantName })}
          className="w-full resize-none rounded-lg border border-border-default px-4 py-3 focus:outline-none focus:ring-2 focus:ring-desert-green disabled:bg-surface-secondary disabled:text-text-muted"
          rows={1} maxLength={16000} disabled={isLoading || disabled} style={{ maxHeight: '200px' }} /></div>
        {isLoading ? <button type="button" onClick={onStop} className="chat-control">{t('chat.stop')}</button>
          : <button type="submit" aria-label={t('chat.send')} disabled={!input.trim() || disabled}
            className={classNames('min-h-11 min-w-11 p-3 rounded-lg flex-shrink-0 mb-2 bg-desert-green text-white focus-visible:outline-2 focus-visible:outline-offset-2 disabled:opacity-50')}>
            <IconSend className="h-6 w-6" aria-hidden="true" />
          </button>}
      </form>
    </div>
  </div>;
}
