import assert from 'node:assert/strict';
import test from 'node:test';
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { AndroidWebView, parsePss, parseBenchmarkSession, selectAppPage } from '../scripts/android-webview-adapter.mjs';
import * as adapter from '../scripts/android-webview-adapter.mjs';

test('owned WebView forwarding is explicit loopback and removes only its verified endpoint', async () => {
  const calls = [];
  const inspections = [];
  const adb = new AndroidWebView({ serial: 'fixture-device', adb: 'fixture-adb', inspectHost: async (port) => { inspections.push(port); return 77; }, execute: async (_, args) => {
    calls.push(args);
    assert.deepEqual(args.slice(0, 2), ['-s', 'fixture-device']);
    const command = args.slice(2).join(' ');
    if (command === 'get-state') return 'device\n';
    if (command === 'shell pidof -s org.tonalli.spike') return '123\n';
    if (command === 'shell run-as org.tonalli.spike cat /proc/123/cmdline') return 'org.tonalli.spike\0';
    if (command === 'shell cat /proc/net/unix') return '000: 0 0 0 0 0 0 @webview_devtools_remote_999\n000: 0 0 0 0 0 0 @webview_devtools_remote_123\n';
    if (command === 'forward --no-rebind tcp:0 localabstract:webview_devtools_remote_123') return '42123\n';
    if (command === 'forward --list') return 'other-device tcp:9999 localabstract:unrelated\nfixture-device tcp:42123 localabstract:webview_devtools_remote_123\n';
    if (command === 'forward --remove tcp:42123') return '';
    throw Error('unexpected fixture command');
  } });
  assert.equal(await adb.forward(), 'http://127.0.0.1:42123');
  assert.deepEqual(inspections, [undefined, 42123]);
  await adb.close();
  assert.equal(calls.filter((args) => args.includes('--remove')).length, 1);
  assert.ok(calls.every((args) => !args.includes('--remove-all')));
});

test('unsafe or unverifiable host listeners fail closed', () => {
  const snapshot = { process: { ProcessId: 77, ExecutablePath: 'C:\\sdk\\adb.exe', CommandLine: 'adb -L tcp:5037 fork-server server --reply-fd 732' }, listeners: [{ LocalPort: 5037, LocalAddress: '127.0.0.1', OwningProcess: 77 }, { LocalPort: 42123, LocalAddress: '127.0.0.1', OwningProcess: 77 }] };
  assert.equal(adapter.validateAdbHost(snapshot, 'C:\\sdk\\adb.exe', 42123), 77);
  for (const change of [
    { process: { ...snapshot.process, CommandLine: 'adb -a -L tcp:5037 fork-server server --reply-fd 732' } },
    { listeners: snapshot.listeners.map((row) => ({ ...row, LocalAddress: '0.0.0.0' })) },
    { listeners: [snapshot.listeners[0]] },
    { process: { ...snapshot.process, ExecutablePath: 'C:\\other\\adb.exe' } },
  ]) assert.throws(() => adapter.validateAdbHost({ ...snapshot, ...change }, 'C:\\sdk\\adb.exe', 42123), /unsafe_adb_host/);
});

test('host refusal precedes forwarding and failed listener verification removes only the owned forward', async () => {
  for (const failAfterCreation of [false, true]) {
    const calls = [];
    const adb = new AndroidWebView({ serial: 'fixture', inspectHost: async (port) => {
      if (!failAfterCreation || port !== undefined) throw Error('unsafe_adb_host');
      return 77;
    }, execute: async (_, args) => {
      const command = args.slice(2).join(' '); calls.push(command);
      if (command === 'get-state') return 'device';
      if (command.includes('pidof')) return '123';
      if (command.includes('/cmdline')) return 'org.tonalli.spike\0';
      if (command === 'shell cat /proc/net/unix') return '@webview_devtools_remote_123';
      if (command === 'forward --no-rebind tcp:0 localabstract:webview_devtools_remote_123') return '42123';
      if (command === 'forward --list') return 'fixture tcp:42123 localabstract:webview_devtools_remote_123';
      if (command === 'forward --remove tcp:42123') return '';
      throw Error('unexpected command');
    } });
    await assert.rejects(adb.forward(), /unsafe_adb_host/);
    assert.equal(calls.some((call) => call.includes('--no-rebind')), failAfterCreation);
    assert.equal(calls.filter((call) => call === 'forward --remove tcp:42123').length, failAfterCreation ? 1 : 0);
    assert.equal(adb.port, null);
  }
});

test('wrong process, ambiguous pages and notes/token pages fail without reading page content', async () => {
  const adb = new AndroidWebView({ serial: 'fixture', execute: async (_, args) => {
    const command = args.slice(2).join(' ');
    if (command === 'get-state') return 'device';
    if (command.includes('pidof')) return '42';
    return 'another.package\0';
  } });
  await assert.rejects(adb.forward(), /app_process_mismatch/);
  const page = (url) => ({ url: () => url });
  assert.equal(selectAppPage([page('http://127.0.0.1:8888/library?pack=probe')]).url().includes('/library'), true);
  for (const pages of [[page('http://127.0.0.1:8888/notes')], [page('http://evil.invalid/library')],
    [page('http://127.0.0.1:8888/launch?token=DO_NOT_ECHO')], [page('about:blank'), page('about:blank')]]) {
    assert.throws(() => selectAppPage(pages, true), /app_webview_not_unique/);
  }
  assert.throws(() => new AndroidWebView({ serial: '--unsafe' }), /invalid_serial/);
});

test('memory receipts contain measured app-process PSS only and no raw dump', () => {
  const parsed = parsePss('** MEMINFO in pid 123 [org.tonalli.spike] **\nTOTAL PSS: 45678 TOTAL RSS: 77777\nsecret unrelated text');
  assert.deepEqual(parsed, { pid: 123, appProcessPssKiB: 45678, rendererPss: 'not measured; manual attribution required' });
  assert.equal(JSON.stringify(parsed).includes('secret'), false);
  assert.throws(() => parsePss('** MEMINFO in pid 1 [other.package] **\nTOTAL PSS: 1'), /invalid_pss/);
});

test('a root HTML preload cannot qualify as a fresh first library document', () => {
  const library = [{ kind: 'response', path: '/library', status: 200, mime: 'text/html' },
    { kind: 'commit', path: '/library' }];
  const prewarmed = [{ kind: 'response', path: '/', status: 200, mime: 'text/html' },
    { kind: 'commit', path: '/' }, ...library];
  assert.throws(() => adapter.qualifyFreshEntry(prewarmed), /fresh_entry_preloaded/);
  assert.throws(() => adapter.qualifyFreshEntry([{ kind: 'response', path: '/article', status: 200, mime: 'text/html' }, ...library]), /fresh_entry_preloaded/);
  assert.throws(() => adapter.qualifyFreshEntry([...library, ...library]), /fresh_entry_not_unique/);
  assert.doesNotThrow(() => adapter.qualifyFreshEntry([{ kind: 'response', path: '/launch', status: 302, mime: '' }, ...library]));
});

test('only a current native receipt can bind the prepared internal dataset; extra private fields are discarded', () => {
  const expected = { pid: 123, dataset: 'full', manifest: 'a'.repeat(64), ui: 'b'.repeat(64), profile: 'm1-perf-fixture' };
  const fixture = { version: 1, state: 'ready', pid: 123, dataset_id: 'full', manifest_sha256: expected.manifest,
    session_id: '12345678-1234-1234-1234-123456789abc',
    ui_id: expected.ui, source: 'internal_mmap', profile: expected.profile, process_started_elapsed_ms: 99,
    accidental_private_field: 'DO_NOT_RETAIN' };
  const parsed = parseBenchmarkSession(JSON.stringify(fixture), expected);
  assert.equal(parsed.source, 'internal_mmap');
  assert.ok(!JSON.stringify(parsed).includes('DO_NOT_RETAIN'));
  for (const change of [{ pid: 124 }, { state: 'closed' }, { source: 'external_pread' },
    { profile: 'm1-perf-another' }, { manifest_sha256: 'c'.repeat(64) }, { ui_id: 'd'.repeat(64) }]) {
    assert.throws(() => parseBenchmarkSession(JSON.stringify({ ...fixture, ...change }), expected), /invalid_benchmark_session/);
  }
});

test('invalid benchmark configuration cannot force-stop an app', async () => {
  const calls = [];
  const adb = new AndroidWebView({ serial: 'fixture', execute: async (_, args) => {
    calls.push(args);
    return args.includes('get-state') ? 'device' : 'DEBUGGABLE';
  } });
  await assert.rejects(adb.startFresh({ profile: 'unsafe/profile', dataset: 'fixture', manifest: 'a'.repeat(64) }), /invalid_benchmark_configuration/);
  assert.ok(!calls.some((args) => args.includes('force-stop')));
});

test('fresh mode stops only the fixed app and requires process absence before its debug action', async () => {
  const calls = [];
  const adb = new AndroidWebView({ serial: 'fixture', execute: async (_, args) => {
    calls.push(args);
    if (args.includes('get-state')) return 'device';
    if (args.includes('package')) return 'DEBUGGABLE';
    return '';
  } });
  await adb.startFresh({ profile: 'm1-perf-fixture', dataset: 'full', manifest: 'a'.repeat(64) });
  const stop = calls.findIndex((args) => args.includes('force-stop'));
  const pid = calls.findIndex((args) => args.includes('pidof'));
  const start = calls.findIndex((args) => args.includes('start'));
  assert.ok(stop >= 0 && pid > stop && start > pid);
  assert.equal(calls[stop].at(-1), 'org.tonalli.spike');
  assert.ok(calls.every((args) => args[0] === '-s' && args[1] === 'fixture'));
});

test('receipt reservation refuses overwrite and missing inputs fail before any device access', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'tonalli-adapter-output-test-'));
  try {
    const output = path.join(directory, 'receipt.json');
    const args = [fileURLToPath(new URL('../scripts/measure-android-webview.mjs', import.meta.url)),
      '--serial', 'fixture-device', '--adb', path.join(directory, 'nonexistent-adb.exe'), '--output', output,
      '--apk-sha256', 'a'.repeat(64), '--ui-id', 'b'.repeat(64), '--dataset-id', 'full',
      '--dataset-manifest', path.join(directory, 'missing-manifest.json'), '--check'];
    await writeFile(output, 'retained receipt', { flag: 'wx' });
    await assert.rejects(promisify(execFile)(process.execPath, args));
    assert.equal(await readFile(output, 'utf8'), 'retained receipt');
    await rm(output);
    await assert.rejects(promisify(execFile)(process.execPath, args));
    const receipt = JSON.parse(await readFile(output));
    assert.equal(receipt.status, 'setup_failed');
    assert.deepEqual(receipt.attempts, []);
    assert.ok(!JSON.stringify(receipt).includes('missing-manifest'));
  } finally {
    assert.equal(path.dirname(path.resolve(directory)), path.resolve(tmpdir()));
    assert.ok(path.basename(directory).startsWith('tonalli-adapter-output-test-'));
    await rm(directory, { recursive: true, force: true });
  }
});
