import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import { it, expect } from 'vitest';
import { AppContext } from '../../src/app/context.js';
import ModelSettings from '../../src/features/chat/ModelSettings.jsx';

function render(state) {
  const client = new QueryClient();
  client.setQueryData(['ai-models', 'csrf'], { state, model: state === 'ready' ? 'model' : null,
    error: null, downloadAvailable: true, models: [{ id: 'model', title: 'Local model', version: '1', bytes: 1024, installed: true }] });
  const markup = renderToStaticMarkup(<MemoryRouter><QueryClientProvider client={client}>
    <AppContext.Provider value={{ csrf: 'csrf', t: key => key, language: 'en', canGetContent: true, canUseAi: state === 'ready' }}><ModelSettings /></AppContext.Provider>
  </QueryClientProvider></MemoryRouter>);
  client.clear(); return markup;
}

it('keeps a second load unavailable while loading or stopping', () => {
  for (const state of ['loading', 'stopping']) {
    const markup = render(state);
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>models.use<\/button>/);
    expect(markup).toContain(`models.state.${state}`);
    expect(markup).not.toContain('href="/chat"');
  }
});

it('offers chat only after a model is actually ready', () => {
  expect(render('off')).not.toContain('href="/chat"');
  expect(render('ready')).toContain('href="/chat"');
});
