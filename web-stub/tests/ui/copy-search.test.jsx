import { it, expect } from 'vitest';
import { canCopySearch, copyStartBody, copySearchNote } from '../../src/features/library/copy-search.js';
import en from '../../src/locales/en.json';
import es from '../../src/locales/es.json';

it('asks for article search only for books and only when chosen', () => {
  expect(canCopySearch('zim')).toBe(true);
  expect(canCopySearch('map')).toBe(false);
  expect(copyStartBody('book', 'zim', true)).toEqual({ id: 'book', search: true });
  expect(copyStartBody('book', 'zim', false)).toEqual({ id: 'book' });
  expect(copyStartBody('map', 'map', true)).toEqual({ id: 'map' });
});

it('maps each reported search outcome to one message and ignores anything else', () => {
  const notes = ['copying', 'copied', 'unavailable', 'failed', 'cancelled'].map(search => copySearchNote({ search }));
  expect(new Set(notes).size).toBe(5);
  expect(notes.every(Boolean)).toBe(true);
  for (const copy of [undefined, null, {}, { search: null }, { search: 'requested' }, { search: 'toString' }, { search: '__proto__' }]) {
    expect(copySearchNote(copy)).toBeNull();
  }
});

it('has English and Spanish text for the search option and every outcome', () => {
  for (const key of ['withSearch', 'searchProgress', 'searchCopied', 'searchUnavailable', 'searchFailed', 'searchCancelled'].map(name => `copy.${name}`)) {
    expect(en[key]).toBeTruthy();
    expect(es[key]).toBeTruthy();
    expect(es[key]).not.toBe(en[key]);
  }
});
