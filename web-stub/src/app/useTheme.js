/* Adapted from Project NOMAD admin/inertia/hooks/useTheme.ts at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
 * Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
 * Modified: authenticated private Core persistence replaces localStorage/Adonis KV;
 * follow the device theme when requested and react to preference changes. */
import { useEffect } from 'react';

export function useTheme(theme = 'system') {
  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    const apply = () => document.documentElement.setAttribute('data-theme', theme === 'system' ? (media.matches ? 'dark' : 'light') : theme);
    apply();
    media.addEventListener('change', apply);
    return () => media.removeEventListener('change', apply);
  }, [theme]);
}
