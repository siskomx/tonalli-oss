import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { actionsFor, errorKey, progressFor, projectionFor } from '../src/features/content/presentation.js';

test('verification at100percent downloaded stays verifying and pending stops offer no duplicate command', () => {
  const job = { state: 'verifying', downloadedBytes: 1000, verifiedBytes: 200, totalBytes: 1000 };
  assert.deepEqual(progressFor(job), { value: 200, max: 1000, state: 'verifying' });
  assert.deepEqual(actionsFor('pausing'), []);
  assert.deepEqual(actionsFor('cancelling'), []);
  assert.deepEqual(actionsFor('installing'), []);
  assert.deepEqual(actionsFor('paused'), ['resume', 'cancel']);
  assert.deepEqual(actionsFor('failed'), ['retry', 'cancel']);
  assert.deepEqual(actionsFor('cancelled'), ['retry']);
  assert.deepEqual(actionsFor('completed'), []);
});

test('Core worker failures have specific English and Spanish explanations', () => {
  for (const language of ['en', 'es']) {
    const messages = JSON.parse(readFileSync(new URL(`../src/locales/${language}.json`, import.meta.url), 'utf8'));
    const t = key => messages[key] ?? key;
    for (const code of ['no_space', 'publication_uncertain', 'invalid_content', 'network', 'media_removed', 'io', 'dispatch_rejected']) {
      const key = `content.error.${code}`;
      assert.equal(errorKey(code, t), key, `${language}: ${code} must not hide behind generic failure`);
      assert.ok(messages[key].trim());
    }
    assert.equal(errorKey('unrecognized_future_failure', t), 'content.error.failed');
  }
});

test('storage presentation uses additional backend budget and does not override a non-space refusal', () => {
  const projection = projectionFor({ availableBytes: 5000, requiredBytes: 1500, allowed: false });
  assert.equal(projection.percent, 30);
  assert.equal(projection.remainingBytes, 3500);
  assert.equal(projection.allowed, false);
  assert.equal(projectionFor({ availableBytes: 0, requiredBytes: 1024, allowed: false }).percent, 100);
});
