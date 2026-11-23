import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: { environment: 'node', include: ['tests/performance/*.test.js', 'tests/ui/*.test.jsx'] },
});
