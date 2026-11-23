import React from 'react';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { it, expect } from 'vitest';
import Tabs from '../../src/ui/Tabs.jsx';
import { sharedSelection, MAX_SHARED_FILES } from '../../src/app/sharing-selection.js';
import en from '../../src/locales/en.json';
import es from '../../src/locales/es.json';

const tabs = [{ id: 'a', label: 'First', content: <p>one</p> }, { id: 'b', label: 'Second', badge: 3, content: <p>two</p> }];

it('renders an accessible tab list and only the selected panel', () => {
  const markup = renderToStaticMarkup(<Tabs tabs={tabs} label="Sections" initial="b" />);
  expect(markup).toContain('role="tablist"');
  expect(markup).toContain('aria-label="Sections"');
  expect(markup.match(/role="tab"/g)).toHaveLength(2);
  expect(markup).toContain('two');
  expect(markup).not.toContain('one');
  expect(markup).toContain('(3)');
  expect(markup).toMatch(/aria-selected="true"[^>]*>Second/s);
});

it('opens the first tab when the requested tab does not exist', () => {
  const markup = renderToStaticMarkup(<Tabs tabs={tabs} label="Sections" value="missing" />);
  expect(markup).toContain('one');
  expect(markup).not.toContain('two');
});

it('shares the whole library unless an item was removed', () => {
  const packs = [{ id: 'book' }, { id: 'map' }], files = [{ id: 'f1' }, { id: 'f2' }, { id: 'f3' }];
  expect(sharedSelection(packs, files, [], [])).toMatchObject({ packs: ['book', 'map'], media: ['f1', 'f2', 'f3'], total: 5, shared: 5, hidden: 0, tooMany: false });
  expect(sharedSelection(packs, files, ['map'], ['f2'])).toMatchObject({ packs: ['book'], media: ['f1', 'f3'], shared: 3, hidden: 2 });
  expect(sharedSelection(packs, files, ['gone'], ['gone'])).toMatchObject({ shared: 5, hidden: 0 });
});

it('links that expect a specific Settings card open its tab', () => {
  const read = path => readFileSync(new URL(`../../src/${path}`, import.meta.url), 'utf8');
  for (const [path, tab] of [['features/library/Library.jsx', 'sharing'], ['features/library/RemoteCopy.jsx', 'sharing'], ['app/AppLayout.jsx', 'sharing'],
    ['features/room/Room.jsx', 'sharing'], ['features/chat/Chat.jsx', 'updates'], ['features/content/Content.jsx', 'updates']]) {
    expect(read(path)).toContain(`/settings?tab=${tab}`);
  }
  expect(read('app/Settings.jsx')).toContain("get('tab')");
});

it('flags a library larger than the server limit', () => {
  const files = Array.from({ length: MAX_SHARED_FILES + 1 }, (_, index) => ({ id: `f${index}` }));
  expect(sharedSelection([], files, [], []).tooMany).toBe(true);
  expect(sharedSelection([], files, [], ['f0']).tooMany).toBe(false);
});

it('has English and Spanish text for tabs and whole-library sharing', () => {
  for (const key of ['settings.tabs', 'settings.tab.general', 'settings.tab.sharing', 'settings.tab.updates', 'settings.tab.advanced',
    'content.tabs', 'content.tab.catalog', 'content.tab.downloads', 'content.tab.installed', 'content.tab.nearby',
    'sharing.summary', 'sharing.shareAll', 'sharing.tooMany', 'sharing.sharedNow', 'sharing.removeHelp',
    'sharing.nothingShared', 'sharing.removeOne', 'sharing.removeShort']) {
    expect(en[key]).toBeTruthy();
    expect(es[key]).toBeTruthy();
  }
  expect(en['sharing.summary']).toContain('{hidden}');
  expect(es['sharing.summary']).toContain('{hidden}');
});
