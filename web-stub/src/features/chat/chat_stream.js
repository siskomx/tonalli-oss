// Adapted from Project NOMAD admin/inertia/lib/chat_stream.ts at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JavaScript; stable translated error key instead of arbitrary server text.
export function chatStreamErrorMessage(event) {
  if (!event || typeof event !== 'object' || !('error' in event) || !event.error) return null;
  return 'chat.failed';
}
