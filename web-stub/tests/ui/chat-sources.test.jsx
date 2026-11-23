import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vitest';
import ChatMessageBubble from '../../src/features/chat/ChatMessageBubble.jsx';

it('links only Core-selected citations to local articles, never generated URLs', () => {
  const markup = renderToStaticMarkup(<ChatMessageBubble t={key => key} message={{
    role: 'assistant', timestamp: new Date(0), content: 'A loop [1](https://example.invalid/fake).',
    grounding: { pack: 'book', sources: [{ number: 1, title: 'Bowline', url: 'C/Bowline' },
      { number: 2, title: 'Unused', url: 'C/Unused' }], citations: [1], flags: {} },
  }} />);
  expect(markup).toContain('/article?pack=book&amp;url=C%2FBowline');
  expect(markup).not.toContain('href="https:');
  expect(markup).not.toContain('C%2FUnused');
});

it('shows no-source refusal and flags unsupported answers without invented links', () => {
  for (const grounding of [{ no_sources: true, sources: [], citations: [], flags: {} },
    { sources: [], citations: [], flags: { uncited: true } }]) {
    const markup = renderToStaticMarkup(<ChatMessageBubble t={key => key} message={{
      role: 'assistant', timestamp: new Date(0), content: '', grounding,
    }} />);
    expect(markup).toContain(grounding.no_sources ? 'chat.noSources' : 'chat.unsupported');
    expect(markup).not.toContain('href=');
  }
});
