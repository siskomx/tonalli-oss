import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { it, expect } from 'vitest';
import { NotificationNoticeView, notificationsEnabled } from '../../src/app/NotificationNotice.jsx';
import en from '../../src/locales/en.json';
import es from '../../src/locales/es.json';

const answer = (body, ok = true) => async () => ({ ok, json: async () => body });

it('offers the native recovery link only through the notice', () => {
  const markup = renderToStaticMarkup(<NotificationNoticeView t={key => key} />);
  expect(markup).toContain('href="/native/notifications"');
  expect(markup).toContain('notifications.title');
});

it('reads only a boolean enabled flag from the native state', async () => {
  expect(await notificationsEnabled(answer({ enabled: false }))).toBe(false);
  expect(await notificationsEnabled(answer({ enabled: true }))).toBe(true);
  for (const bad of [{ enabled: 'no' }, {}, null]) expect(await notificationsEnabled(answer(bad))).toBeNull();
  expect(await notificationsEnabled(answer({ enabled: false }, false))).toBeNull();
  expect(await notificationsEnabled(async () => { throw new Error('offline'); })).toBeNull();
});

it('has English and Spanish text for every notice label', () => {
  for (const key of ['notifications.title', 'notifications.help', 'notifications.enable']) {
    expect(en[key]).toBeTruthy();
    expect(es[key]).toBeTruthy();
    expect(es[key]).not.toBe(en[key]);
  }
});
