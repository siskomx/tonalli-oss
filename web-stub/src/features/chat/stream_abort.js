// Adapted from Project NOMAD inertia/components/chat/stream_abort.ts at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JavaScript conversion and local imports; behavior retained.
                                 
                                 
 

export function abortActiveStream(ref                )          {
  const controller = ref.current
  if (!controller) return false

  // Clear ownership before aborting. The rejected stream settles asynchronously,
  // so its finally block must not be allowed to clear a replacement controller.
  ref.current = null
  controller.abort()
  return true
}

export function clearStreamIfCurrent(ref                , controller                 )          {
  if (ref.current !== controller) return false

  ref.current = null
  return true
}
