// Adapted from Project NOMAD admin/tests/unit/chat_stream.spec.ts at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: local import; assert stable translated errors rather than arbitrary server branding/text.
import assert from 'node:assert/strict';
import test from 'node:test';
import { chatStreamErrorMessage } from '../src/features/chat/chat_stream.js';
test('chat stream reports an actionable local error without exposing arbitrary server messages', () => {
  assert.equal(chatStreamErrorMessage({ error: true, message: 'untrusted diagnostic' }), 'chat.failed');
  assert.equal(chatStreamErrorMessage({ error: true }), 'chat.failed');
  assert.equal(chatStreamErrorMessage({ done: true }), null);
});
