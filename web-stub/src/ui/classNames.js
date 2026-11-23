// Adapted from Project NOMAD inertia/lib/classNames.ts at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JavaScript conversion and local imports; behavior retained.

export default function classNames(...classes                        )         {
  return classes.filter(Boolean).join(' ');
}