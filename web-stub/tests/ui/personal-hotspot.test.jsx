import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { it, expect } from 'vitest';
import PersonalHotspot from '../../src/app/PersonalHotspot.jsx';
import { personalHotspotNetwork, personalHotspotStep } from '../../src/app/hotspot.js';
import en from '../../src/locales/en.json';
import es from '../../src/locales/es.json';

const wifi = { id: '192.168.1.20', label: 'Wi-Fi' };
const hotspot = { id: '172.20.10.1', label: 'Personal Hotspot' };
const markup = props => renderToStaticMarkup(<PersonalHotspot t={key => key} {...props} />);

it('knows which network is the iPhone Personal Hotspot', () => {
  expect(personalHotspotNetwork(hotspot)).toBe(true);
  expect(personalHotspotNetwork(wifi)).toBe(false);
  expect(personalHotspotNetwork({ id: 'hotspot', label: 'Phone hotspot' })).toBe(false);
  expect(personalHotspotNetwork(undefined)).toBe(false);
});

it('asks the user to turn the hotspot on until iOS shows it', () => {
  expect(personalHotspotStep([wifi], false, null)).toBe('off');
  expect(personalHotspotStep([], false, null)).toBe('off');
  const text = markup({ networks: [wifi], running: false, url: null });
  expect(text).toContain('sharing.iosHotspotOff');
  expect(text).toContain('sharing.iosHotspotWait');
  expect(text).not.toContain('sharing.iosHotspotReady');
});

it('says what to do once the hotspot is on and ready to be chosen', () => {
  expect(personalHotspotStep([wifi, hotspot], false, null)).toBe('ready');
  const text = markup({ networks: [wifi, hotspot], running: false, url: null });
  expect(text).toContain('sharing.iosHotspotReady');
  expect(text).not.toContain('sharing.iosHotspotOff');
});

it('tells the other device how to join only while sharing runs on the hotspot', () => {
  expect(personalHotspotStep([wifi, hotspot], true, 'https://172.20.10.1:41233/pair')).toBe('joining');
  expect(markup({ networks: [wifi, hotspot], running: true, url: 'https://172.20.10.1:41233/pair' })).toContain('sharing.iosHotspotJoin');
  // Sharing on ordinary Wi-Fi needs no hotspot help at all.
  expect(personalHotspotStep([wifi, hotspot], true, 'https://192.168.1.20:41233/pair')).toBe('none');
  expect(markup({ networks: [wifi, hotspot], running: true, url: 'https://192.168.1.20:41233/pair' })).toBe('');
});

it('never claims to know the hotspot name or password', () => {
  expect(en['sharing.iosHotspotReady']).toMatch(/cannot read|can't read/i);
  expect(es['sharing.iosHotspotReady']).toMatch(/no puede leer/i);
});

it('has English and Spanish text for the iPhone hotspot help', () => {
  for (const name of ['iosHotspotTitle', 'iosHotspotOff', 'iosHotspotWait', 'iosHotspotReady', 'iosHotspotJoin', 'personalHotspot']) {
    expect(en[`sharing.${name}`]).toBeTruthy();
    expect(es[`sharing.${name}`]).toBeTruthy();
    expect(es[`sharing.${name}`]).not.toBe(en[`sharing.${name}`]);
  }
});
