import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { it, expect } from 'vitest';
import { AppContext } from '../../src/app/context.js';
import UpdateSettings from '../../src/features/update/UpdateSettings.jsx';

const base = {
  supported: true, platform: 'windows-x64', current: '0.1.0-preview.2', enabled: true,
  source: 'https://github.com/example/tonalli/releases/download/update-feed/', public_key: 'ab'.repeat(32),
  check: 'never', error: null, available: null,
  download: { state: 'idle', downloaded: 0, total: 0, error: null },
};
const offer = (extra = {}) => ({
  version: '0.1.0-preview.3', published: '2026-11-16', notes: 'Smaller AI model for older phones.',
  page: 'https://example.org/release', bytes: 30_401_170, downloaded: false, mode: 'apply', ...extra,
});

function render(status) {
  const client = new QueryClient();
  client.setQueryData(['app-update', 'csrf'], status);
  const markup = renderToStaticMarkup(<QueryClientProvider client={client}>
    <AppContext.Provider value={{ csrf: 'csrf', t: key => key, language: 'en' }}><UpdateSettings /></AppContext.Provider>
  </QueryClientProvider>);
  client.clear(); return markup;
}

it('is off and asks for a source and trusted key before anything can be checked', () => {
  const markup = render({ ...base, enabled: false, source: '', public_key: '' });
  expect(markup).toContain('update.title');
  expect(markup).toContain('update.privacy');
  expect(markup).toMatch(/<input[^>]*type="checkbox"[^>]*>/);
  expect(markup).toContain('name="source"');
  expect(markup).toContain('name="public_key"');
  expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>update.check<\/button>/);
  expect(markup).not.toContain('update.download');
});

it('keeps the current version visible and shows nothing new when up to date', () => {
  const markup = render({ ...base, check: 'up_to_date' });
  expect(markup).toContain('0.1.0-preview.2');
  expect(markup).toContain('update.upToDate');
  expect(markup).not.toContain('update.download');
});

it('shows the release notes and a download button for an available update', () => {
  const markup = render({ ...base, check: 'available', available: offer() });
  expect(markup).toContain('0.1.0-preview.3');
  expect(markup).toContain('Smaller AI model for older phones.');
  expect(markup).toContain('update.download');
  expect(markup).not.toContain('/native/apply-update');
});

it('offers the platform step only after the package is downloaded and verified', () => {
  const windows = render({ ...base, check: 'available', available: offer({ downloaded: true }) });
  expect(windows).toContain('href="/native/apply-update"');
  expect(windows).not.toContain('>update.download<');
  const android = render({ ...base, platform: 'android-arm64', check: 'available', available: offer({ downloaded: true, mode: 'install' }) });
  expect(android).toContain('href="/native/install-update"');
});

it('only shows where to get the update on iPhone, never an in-app install', () => {
  const markup = render({ ...base, platform: 'ios-arm64', check: 'available', available: offer({ mode: 'link' }) });
  expect(markup).toContain('https://example.org/release');
  expect(markup).toContain('update.copyLink');
  expect(markup).not.toContain('update.download');
  expect(markup).not.toContain('/native/');
  // External addresses cannot be opened inside the app, so none may be a link.
  expect(markup).not.toMatch(/<a[^>]*href="https?:/);
});

it('shows download progress with a way to cancel', () => {
  const markup = render({ ...base, check: 'available', available: offer(),
    download: { state: 'running', downloaded: 15_000_000, total: 30_000_000, error: null } });
  expect(markup).toContain('role="progressbar"');
  expect(markup).toContain('update.cancel');
  expect(markup).not.toContain('>update.download<');
});

it('explains a failed check or download in plain words', () => {
  expect(render({ ...base, check: 'error', error: 'update_signature' })).toContain('update.error.update_signature');
  expect(render({ ...base, check: 'available', available: offer(),
    download: { state: 'failed', downloaded: 0, total: 0, error: 'update_unreachable' } })).toContain('update.error.update_unreachable');
});

it('says so when this device cannot update itself', () => {
  const markup = render({ ...base, supported: false, platform: null });
  expect(markup).toContain('update.unsupported');
  expect(markup).not.toContain('name="source"');
});
