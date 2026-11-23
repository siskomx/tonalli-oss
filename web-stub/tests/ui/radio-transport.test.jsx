import { it, expect } from 'vitest';
import { transportsFor } from '../../src/features/room/transport.js';

it('offers only Bluetooth on the iPhone app, which has no USB host', () => {
  expect(transportsFor('Mozilla/5.0 TonalliNative/iOS')).toEqual(['ble']);
});

it('keeps Bluetooth and USB on Windows, Android and a plain browser', () => {
  for (const agent of ['Mozilla/5.0 TonalliNative/Android', 'Mozilla/5.0 (Windows NT 10.0) Edg/120', '', 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) Safari']) {
    expect(transportsFor(agent)).toEqual(['ble', 'serial']);
  }
});
