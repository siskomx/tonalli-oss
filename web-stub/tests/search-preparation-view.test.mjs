import assert from 'node:assert/strict';
import test from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import SearchPreparationView from '../src/features/library/SearchPreparationView.js';

for (const language of ['en', 'es']) {
  const messages = JSON.parse(readFileSync(new URL(`../src/locales/${language}.json`, import.meta.url), 'utf8'));
  const t = (key, values = {}) => (messages[key] ?? key).replace(/\{(\w+)\}/g, (_, name) => values[name]);
  const render = (status, extra = {}) => renderToStaticMarkup(React.createElement(SearchPreparationView,
    { status, t, language, onAction() {}, ...extra }));
  test(`${language}: preparation shows actual counters and indeterminate progress without an estimate`, () => {
    const html = render({ state: 'preparing', phase: 'checking', processedArticles: 12, committedArticles: 10 });
    assert.ok(html.includes(t('library.prepare.phase.checking')));
    assert.ok(html.includes(t('library.prepare.count', { count: 10 })));
    assert.match(html, /<progress/);
    assert.doesNotMatch(html, /\b(?:max|value|aria-valuenow)=|\d+%|ETA|estimate|library\.prepare\./);
    assert.ok(html.includes(t('library.prepare.action.pause')));
  });
  test(`${language}: pending stops are not completed and unknown errors remain local`, () => {
    for (const state of ['pausing', 'cancelling']) {
      const html = render({ state, committedArticles: null });
      assert.ok(html.includes(t(`library.prepare.state.${state}`)));
      assert.doesNotMatch(html, /<button/);
      assert.ok(!html.includes(t('library.prepare.state.ready')));
    }
    const failed = render({ state: 'failed', reason: '/private/secret', committedArticles: null });
    assert.ok(failed.includes(t('library.prepare.error.failed')));
    assert.doesNotMatch(failed, /private\/secret|library\.prepare\./);
    const ready = render({ state: 'ready', committedArticles: 12 });
    assert.ok(ready.includes(t('library.prepare.state.ready')));
    assert.doesNotMatch(ready, /<progress|<button/);
  });
}
