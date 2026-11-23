import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, test } from 'vitest';
import ContentSelector from '../../src/features/content/ContentSelector.jsx';
import ActiveDownloads from '../../src/features/content/ActiveDownloads.jsx';
import StorageProjectionBar from '../../src/features/content/StorageProjectionBar.jsx';
import { createTranslator } from '../../src/locales/index.js';

const t = createTranslator('en');
const entry = { id: 'medicine', title: 'Medicine', language: 'en', kind: 'zim', version: '2026-10', bytes: 1000,
  installedVersion: null, installedReadable: null };

test('a disconnected nearby copy can still be cancelled while resume waits for reconnection', () => {
  const html = renderToStaticMarkup(<ActiveDownloads jobs={[{ id: 'nearby-copy', entryId: 'medicine', state: 'paused',
    downloadedBytes: 1, totalBytes: 100, verifiedBytes: 0 }]} titles={{ medicine: 'Medicine' }} onAction={() => {}}
    disabledActions={['resume', 'retry']} t={t} language="en" />);
  expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Resume<\/button>/);
  expect(html).toMatch(/<button(?![^>]*disabled)[^>]*>Cancel<\/button>/);
});

test('catalog selection is keyboard accessible and an unreadable installed version is not marked ready', () => {
  const html = renderToStaticMarkup(<ContentSelector entries={[{ ...entry, installedVersion: '2026-10', installedReadable: false }]}
    selectedId="medicine" onSelect={() => {}} t={t} language="en" />);
  expect(html).toContain('aria-pressed="true"');
  expect(html).toContain('<button');
  expect(html).toContain('Unavailable');
  expect(html).not.toContain('Ready to use');
  expect(html).not.toMatch(/\b(?:href|src)=|url\(/);
});

test('downloaded bytes at100percent render verification, not completed installation', () => {
  const job = { id: 'j1', entryId: 'medicine', state: 'verifying', downloadedBytes: 1000, totalBytes: 1000, verifiedBytes: 200, errorCode: null };
  const html = renderToStaticMarkup(<ActiveDownloads jobs={[job]} titles={{ medicine: 'Medicine' }} onAction={() => {}} t={t} language="en" />);
  expect(html).toContain('Verifying');
  expect(html).toContain('aria-valuenow="200"');
  expect(html).not.toContain('Completed');
  const pending = renderToStaticMarkup(<ActiveDownloads jobs={[{ ...job, state: 'cancelling' }]} titles={{}} onAction={() => {}} t={t} language="en" />);
  expect(pending).toContain('Cancelling');
  expect(pending).not.toContain('Cancelled');
  expect(pending).not.toContain('<button');
});

test('storage projection displays authoritative additional budget without invented disk usage', () => {
  const html = renderToStaticMarkup(<StorageProjectionBar budget={{ availableBytes: 5000, requiredBytes: 1500, downloadBytes: 500,
    stagingBytes: 0, allowed: false, reason: 'waiting_wifi' }} t={t} language="en" />);
  expect(html).toContain('Available');
  expect(html).toContain(t('content.required'));
  expect(html).not.toContain('Current usage');
  expect(html).not.toContain('Ready to download');
});

test('a job with a valid ID absent from current metadata uses its literal ID, not an inherited object property', () => {
  const html = renderToStaticMarkup(<ActiveDownloads jobs={[{ id: 'j1', entryId: 'constructor', state: 'paused', downloadedBytes: 1,
    totalBytes: 100, verifiedBytes: 0, errorCode: null }]} titles={{}} onAction={() => {}} t={t} language="en" />);
  expect(html).toContain('constructor');
});
