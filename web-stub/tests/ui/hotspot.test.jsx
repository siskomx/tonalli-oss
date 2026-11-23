import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { it, expect } from 'vitest';
import HotspotJoin from '../../src/app/HotspotJoin.jsx';
import { wifiQrPayload, isHotspotNetwork } from '../../src/app/hotspot.js';
import en from '../../src/locales/en.json';
import es from '../../src/locales/es.json';

it('builds a standard Wi-Fi payload and escapes reserved characters', () => {
  expect(wifiQrPayload({ ssid: 'DIRECT-a1', password: 'plain123' })).toBe('WIFI:T:WPA;S:DIRECT-a1;P:plain123;;');
  expect(wifiQrPayload({ ssid: 'a;b', password: 'p:"q\\r,s' })).toBe('WIFI:T:WPA;S:a\\;b;P:p\\:\\"q\\\\r\\,s;;');
});

it('shows the temporary network name, password and QR before the address step', () => {
  const markup = renderToStaticMarkup(<HotspotJoin t={key => key} hotspot={{ ssid: 'DIRECT-a1', password: 'secret12' }} />);
  expect(markup).toContain('DIRECT-a1');
  expect(markup).toContain('secret12');
  expect(markup).toContain('data-testid="hotspot-qr"');
  expect(markup.indexOf('sharing.hotspotJoin')).toBeLessThan(markup.indexOf('sharing.hotspotThen'));
});

it('recognises only the two native hotspot network ids', () => {
  expect(['hotspot', 'hotspot-permission'].every(isHotspotNetwork)).toBe(true);
  expect(['wifi', 'hotspot2', ''].some(isHotspotNetwork)).toBe(false);
});

it('has English and Spanish text for every hotspot label', () => {
  for (const key of ['hotspot', 'hotspotHelp', 'hotspotPermissionHelp', 'hotspotAllow', 'hotspotJoin', 'hotspotName', 'hotspotPassword', 'hotspotQr', 'hotspotThen'].map(name => `sharing.${name}`)) {
    expect(en[key]).toBeTruthy();
    expect(es[key]).toBeTruthy();
    expect(es[key]).not.toBe(en[key]);
  }
});
