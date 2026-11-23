// Adapted from Project NOMAD tests/unit/chat_stream_abort.spec.ts at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JavaScript conversion and local imports; behavior retained.
import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  abortActiveStream,
  clearStreamIfCurrent,
                      
} from '../src/features/chat/stream_abort.js'

test('aborts and releases the active stream', () => {
  const controller = new AbortController()
  const ref                 = { current: controller }

  assert.equal(abortActiveStream(ref), true)
  assert.equal(controller.signal.aborted, true)
  assert.equal(ref.current, null)
})

test('does nothing when no stream is active', () => {
  const ref                 = { current: null }

  assert.equal(abortActiveStream(ref), false)
  assert.equal(ref.current, null)
})

test('clears the controller owned by the settling stream', () => {
  const controller = new AbortController()
  const ref                 = { current: controller }

  assert.equal(clearStreamIfCurrent(ref, controller), true)
  assert.equal(ref.current, null)
})

test('does not clear a replacement stream controller', () => {
  const staleController = new AbortController()
  const replacementController = new AbortController()
  const ref                 = { current: replacementController }

  assert.equal(clearStreamIfCurrent(ref, staleController), false)
  assert.equal(ref.current, replacementController)
})
