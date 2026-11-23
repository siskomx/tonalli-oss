// Adapted from Project NOMAD admin/inertia/components/BouncingDots.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JSX, local upstream classNames helper instead of clsx, accessible status.
import React from 'react';
import classNames from '../../ui/classNames.js';
export default function BouncingDots({ text, containerClassName, textClassName }) {
  return <div role="status" className={classNames('flex items-center justify-center gap-2', containerClassName)}>
    <span className={classNames('text-text-secondary', textClassName)}>{text}</span>
    <span className="flex gap-1 mt-1" aria-hidden="true">
      <span className="w-1.5 h-1.5 bg-text-secondary rounded-full animate-bounce" style={{ animationDelay: '0ms' }} />
      <span className="w-1.5 h-1.5 bg-text-secondary rounded-full animate-bounce" style={{ animationDelay: '150ms' }} />
      <span className="w-1.5 h-1.5 bg-text-secondary rounded-full animate-bounce" style={{ animationDelay: '300ms' }} />
    </span>
  </div>;
}
