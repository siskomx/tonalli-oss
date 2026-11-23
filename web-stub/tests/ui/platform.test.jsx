import { it, expect } from 'vitest';
import { iosShell } from '../../src/app/platform.js';

it('recognises only the Tonalli iPhone app, not Safari or the other shells', () => {
  expect(iosShell('Mozilla/5.0 TonalliNative/iOS')).toBe(true);
  for (const agent of ['Mozilla/5.0 TonalliNative/Android', 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) Safari', 'Mozilla/5.0 (Windows NT 10.0) Edg/120', '']) {
    expect(iosShell(agent)).toBe(false);
  }
});
