import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, test, vi } from 'vitest';

vi.mock('../../../branding.json', () => ({ default: { displayName: 'Fixture display name' } }));

test('app_name_comes_from_branding', async () => {
  const { default: AppName } = await import('../../src/app/AppName.jsx');
  expect(renderToStaticMarkup(React.createElement(AppName))).toContain('Fixture display name');
});

test('missing_es_key_falls_back_to_en', async () => {
  const { createTranslator } = await import('../../src/locales/index.js');
  const t = createTranslator('es', { en: { missing: 'English fallback' }, es: {} });
  expect(t('missing')).toBe('English fallback');
});

test('every_en_key_exists_in_es', async () => {
  const { default: en } = await import('../../src/locales/en.json');
  const { default: es } = await import('../../src/locales/es.json');
  expect(Object.keys(es).sort()).toEqual(Object.keys(en).sort());
  for (const key of Object.keys(en)) expect(es[key].trim().length).toBeGreaterThan(0);
});

test('device language chooses supported English or Spanish with English fallback', async () => {
  const { resolveLanguage } = await import('../../src/locales/index.js');
  expect(resolveLanguage(['es-MX', 'en-US'])).toBe('es');
  expect(resolveLanguage(['fr-CA', 'es-ES'])).toBe('es');
  expect(resolveLanguage(['fr-CA'])).toBe('en');
});
