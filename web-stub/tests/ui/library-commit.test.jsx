import { expect, test } from 'vitest';
import { libraryCommit } from '../../src/features/library/diagnostics.js';

test('committed rows distinguish missing data without exposing article content', () => {
  const rows = new Map([[50199, { title: 'Existing title', url: 'C/Existing' }]]);
  const items = [{ index: 50199, start: 2007960 }, { index: 50200, start: 2008000 }];
  const snapshot = libraryCommit('wiki', 100000, 40, items, (index) => rows.get(index));
  expect(snapshot).toEqual({ pack: 'wiki', visibleLimit: 100000, rowHeight: 40,
    rows: [{ index: 50199, loaded: true }, { index: 50200, loaded: false }] });
  // A later fetch must not retroactively rewrite the previous commit receipt.
  rows.set(50200, { title: 'Later title', url: 'C/Later' });
  items[0].index = 0;
  expect(snapshot.rows).toEqual([{ index: 50199, loaded: true }, { index: 50200, loaded: false }]);
  expect(libraryCommit('wiki', 0, 40, [], () => undefined).rows).toEqual([]);
});
