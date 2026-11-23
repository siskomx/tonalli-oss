// Adapted from Project NOMAD admin/inertia/components/chat/ChatMessageBubble.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JSX/local translation; no uploads/Continue; model Markdown cannot navigate or fetch.
// Sources list retained from upstream; only validated Core citations link to local articles.
import React from 'react';
import classNames from '../../ui/classNames.js';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

function Markdown({ children }) {
  return <ReactMarkdown remarkPlugins={[remarkGfm]} components={{
    code: ({ node, className, children, ...props }) => !className?.includes('language-')
      ? <code className="bg-gray-800 text-gray-100 px-2 py-0.5 rounded font-mono text-sm" {...props}>{children}</code>
      : <code className="block bg-gray-800 text-gray-100 p-3 rounded-lg overflow-x-auto font-mono text-sm my-2" {...props}>{children}</code>,
    p: ({ children }) => <p className="mb-2 last:mb-0">{children}</p>,
    ul: ({ children }) => <ul className="list-disc pl-5 mb-2">{children}</ul>,
    ol: ({ children }) => <ol className="list-decimal pl-5 mb-2">{children}</ol>,
    li: ({ children }) => <li className="mb-1">{children}</li>,
    h1: ({ children }) => <h1 className="text-xl font-bold mb-2">{children}</h1>,
    h2: ({ children }) => <h2 className="text-lg font-bold mb-2">{children}</h2>,
    h3: ({ children }) => <h3 className="text-base font-bold mb-2">{children}</h3>,
    blockquote: ({ children }) => <blockquote className="border-l-4 border-border-default pl-4 italic my-2">{children}</blockquote>,
    a: ({ children }) => <span>{children}</span>,
    img: ({ alt }) => <span>{alt}</span>,
  }}>{children}</ReactMarkdown>;
}

export default function ChatMessageBubble({ message, t }) {
  return <div className={classNames('max-w-[90%] sm:max-w-[70%] rounded-lg px-4 py-3',
    message.role === 'user' ? 'bg-desert-green text-white' : 'bg-surface-secondary text-text-primary')}>
    {message.isThinking && message.thinking && <div className="mb-3 rounded border border-amber-200 bg-amber-50 px-3 py-2 text-xs">
      <div className="mb-1 flex items-center gap-1.5 font-medium text-amber-700">
        <span>{t('chat.reasoning')}</span><span className="h-1.5 w-1.5 rounded-full bg-amber-500 animate-pulse inline-block" />
      </div><div className="max-h-32 overflow-y-auto"><Markdown>{message.thinking}</Markdown></div>
    </div>}
    {!message.isThinking && message.thinking && <details className="mb-3 rounded border border-border-subtle text-xs">
      <summary className="cursor-pointer px-3 py-2 font-medium text-text-muted hover:text-text-primary select-none">{t('chat.reasoning')}</summary>
      <div className="px-3 pb-3 max-h-48 overflow-y-auto border-t border-border-subtle pt-2"><Markdown>{message.thinking}</Markdown></div>
    </details>}
    <div className={classNames('break-words', message.role === 'assistant' ? 'prose prose-sm max-w-none' : 'whitespace-pre-wrap')}>
      {message.role === 'assistant' ? <Markdown>{message.content}</Markdown> : message.content}
      {message.isStreaming && <span className="inline-block w-2 h-4 ml-1 bg-current animate-pulse" />}
    </div>
    {message.role === 'assistant' && message.truncated && !message.isStreaming
      && <p className="mt-3 border-t border-border-subtle pt-2 text-xs">{t('chat.truncated')}</p>}
    {message.grounding && !message.isStreaming && <div className="mt-3 border-t border-border-subtle pt-2 text-xs text-text-primary">
      {message.grounding.no_sources ? <p>{t('chat.noSources')}</p>
        : (message.grounding.flags.uncited || message.grounding.flags.invalid_citations) && <p role="status">{t('chat.unsupported')}</p>}
      {message.grounding.citations.length > 0 && <><div className="mb-1 font-medium">{t('chat.sources')}</div>
        <ul className="space-y-0.5">{message.grounding.citations.map(number => {
          const source = message.grounding.sources[number - 1];
          return <li key={number}><a href={`/article?${new URLSearchParams({ pack: message.grounding.pack, url: source.url })}`}>
            [{number}] {source.title}</a></li>;
        })}</ul></>}
    </div>}
    <div className={classNames('text-xs mt-2', message.role === 'user' ? 'text-white/70' : 'text-text-muted')}>
      {message.timestamp.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
    </div>
  </div>;
}
