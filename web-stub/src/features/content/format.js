// Adapted from Project NOMAD admin/inertia/lib/util.ts formatBytes at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: locale-aware binary-unit display, zero and unit bounds; no unrelated utility functions.
export function formatBytes(bytes, language = 'en') {
  const sizes = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
  const index = bytes === 0 ? 0 : Math.min(sizes.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  return `${new Intl.NumberFormat(language, { maximumFractionDigits: 1 }).format(bytes / Math.pow(1024, index))} ${sizes[index]}`;
}
