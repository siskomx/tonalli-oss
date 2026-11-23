import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import test from 'node:test';
import * as adapter from '../scripts/android-webview-adapter.mjs';

const response = { kind: 'response', path: '/library', status: 200, mime: 'text/html' };
const commit = { kind: 'commit', path: '/library' };
const same = { kind: 'same_document', path: '/library' };

test('the captured check-r2 ambiguous duplicate commits still fail uniqueness', () => {
  // Actual target/medicine-webview-phone-r1/check-r2.json; its Playwright event
  // types alone cannot establish which commit, if any, was same-document.
  assert.throws(() => adapter.qualifyFreshEntry([response, commit, commit]), /fresh_entry_not_unique/);
});

test('same-document events cannot conceal a preload or precede the actual document', () => {
  assert.throws(() => adapter.qualifyFreshEntry([response, same, commit]), /fresh_entry_preloaded/);
  assert.throws(() => adapter.qualifyFreshEntry([response, commit, { ...same, path: '/' }]), /fresh_entry_preloaded/);
  assert.throws(() => adapter.qualifyFreshEntry([response, commit, same, commit]), /fresh_entry_not_unique/);
  assert.doesNotThrow(() => adapter.qualifyFreshEntry([response, commit, same]));
});

test('public CDP distinguishes a new main document from History navigation without retaining identities', async () => {
  const session = new EventEmitter();
  const commands = [];
  session.send = async (method) => {
    commands.push(method);
    return method === 'Page.getFrameTree' ? { frameTree: { frame: { id: 'PRIVATE_MAIN' } } } : {};
  };
  const events = [response];
  const stop = await adapter.observeFreshEntryDocuments(session, event => events.push(event));
  session.emit('Page.frameNavigated', { frame: { id: 'PRIVATE_MAIN', url: 'about:blank' }, type: 'Navigation' });
  session.emit('Page.frameNavigated', { frame: { id: 'PRIVATE_CHILD', parentId: 'PRIVATE_MAIN', url: 'http://127.0.0.1:1234/zim/probe/C/Article' } });
  session.emit('Page.frameNavigated', { frame: { id: 'PRIVATE_MAIN', url: 'http://127.0.0.1:1234/library?token=DO_NOT_RETAIN' }, loaderId: 'PRIVATE_LOADER' });
  session.emit('Page.navigatedWithinDocument', { frameId: 'PRIVATE_CHILD', url: 'http://127.0.0.1:1234/not-main' });
  session.emit('Page.navigatedWithinDocument', { frameId: 'PRIVATE_MAIN', url: 'http://127.0.0.1:1234/library?pack=probe#DO_NOT_RETAIN', navigationType: 'historyApi' });
  assert.deepEqual(events, [response, commit, same]);
  assert.doesNotThrow(() => adapter.qualifyFreshEntry(events));
  assert.deepEqual(commands, ['Page.getFrameTree', 'Page.enable']);
  assert.ok(!JSON.stringify(events).includes('PRIVATE') && !JSON.stringify(events).includes('DO_NOT_RETAIN'));
  stop();
  assert.equal(session.listenerCount('Page.frameNavigated'), 0);
  assert.equal(session.listenerCount('Page.navigatedWithinDocument'), 0);
});
