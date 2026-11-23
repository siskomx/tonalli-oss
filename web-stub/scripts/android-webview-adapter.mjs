import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';

const APP = 'org.tonalli.spike';
const run = promisify(execFile);
const problem = (code) => new Error(code);

export function parseMeasurementDataset(text, { dataset, expectedTitleCount = '19191219' }) {
  if (Buffer.byteLength(text) > 65536 || !/^[1-9]\d*$/.test(expectedTitleCount)) throw problem('invalid_measurement_dataset');
  const count = Number(expectedTitleCount);
  const value = JSON.parse(text);
  const pin = (item) => item && Number.isSafeInteger(item.bytes) && item.bytes > 0 && /^[a-f0-9]{64}$/.test(item.sha256);
  if (!Number.isSafeInteger(count) || count < 100000 || value.version !== 1 || value.id !== dataset
    || !pin(value.zim) || !pin(value.title_index)) throw problem('invalid_measurement_dataset');
  return { id: dataset, expectedTitleCount: count,
    zim: { bytes: value.zim.bytes, sha256: value.zim.sha256 },
    titleIndex: { bytes: value.title_index.bytes, sha256: value.title_index.sha256 } };
}

export function qualifyMeasurementTitle(value, expectedTitleCount) {
  const row = value?.rows?.[0];
  if (!Number.isSafeInteger(expectedTitleCount) || expectedTitleCount < 100000
    || value?.pack !== 'probe' || value.offset !== 99999 || value.total !== expectedTitleCount
    || !Array.isArray(value.rows) || value.rows.length !== 1 || typeof row?.title !== 'string'
    || typeof row.url !== 'string' || Buffer.byteLength(row.title) > 4096 || Buffer.byteLength(row.url) > 4096
    || !/^[AC]\/[^\u0000-\u001f\u007f]+$/u.test(row.url)) throw problem('invalid_measurement_title');
  return { pack: value.pack, index: value.offset, total: value.total, title: row.title, url: row.url };
}

export function validateAdbHost(snapshot, adb, port) {
  const proc = snapshot?.process;
  const rows = snapshot?.listeners;
  // Only the SDK's default local fork-server launch is supported. In particular,
  // -a changes tcp:0 forwards to wildcard listeners even with a local server socket.
  if (!proc || !Number.isSafeInteger(proc.ProcessId) || proc.ProcessId < 1
    || typeof proc.ExecutablePath !== 'string' || path.win32.resolve(proc.ExecutablePath).toLowerCase() !== path.win32.resolve(adb).toLowerCase()
    || !/^adb -L tcp:(?:localhost:)?5037 fork-server server --reply-fd \d+$/.test(proc.CommandLine)
    || !Array.isArray(rows)) throw problem('unsafe_adb_host');
  for (const required of [5037, ...(port === undefined ? [] : [port])]) {
    const listeners = rows.filter((row) => row.LocalPort === required);
    if (!listeners.length || listeners.some((row) => row.OwningProcess !== proc.ProcessId
      || !['127.0.0.1', '::1'].includes(row.LocalAddress))) throw problem('unsafe_adb_host');
  }
  return proc.ProcessId;
}
async function inspectAdbHost(adb, port) {
  if (process.platform !== 'win32' || Object.keys(process.env).some((key) =>
    /^(ADB_SERVER_SOCKET|ADB_SERVER_PORT|ANDROID_ADB_SERVER_ADDRESS|ANDROID_ADB_SERVER_PORT)$/i.test(key))) {
    throw problem('unsafe_adb_host');
  }
  const ports = port === undefined ? '5037' : `5037,${port}`;
  const script = `$ErrorActionPreference='Stop'; $rows=@(Get-NetTCPConnection -State Listen -LocalPort ${ports}); $owners=@($rows | Where-Object LocalPort -eq 5037 | Select-Object -ExpandProperty OwningProcess -Unique); if($owners.Count -ne 1){throw 'owner'}; $proc=Get-CimInstance Win32_Process -Filter ('ProcessId='+$owners[0]); @{process=($proc | Select-Object ProcessId,ExecutablePath,CommandLine);listeners=@($rows | Select-Object LocalPort,LocalAddress,OwningProcess)} | ConvertTo-Json -Depth 4 -Compress`;
  try {
    const { stdout } = await run('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script],
      { windowsHide: true, timeout: 15000, maxBuffer: 65536, encoding: 'utf8' });
    return validateAdbHost(JSON.parse(stdout), adb, port);
  } catch { throw problem('unsafe_adb_host'); }
}
function validateBenchmark({ profile, dataset, manifest }) {
  if (!/^m1-perf-[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(profile) || !/^[a-z][a-z0-9_-]{0,31}$/.test(dataset)
    || !/^[a-f0-9]{64}$/.test(manifest)) throw problem('invalid_benchmark_configuration');
}
async function execute(file, args) {
  try { return (await run(file, args, { windowsHide: true, timeout: 15000, maxBuffer: 1024 * 1024, encoding: 'utf8' })).stdout; }
  catch (error) {
    if (error.code === 1 && args[2] === 'shell' && args[3] === 'pidof') return '';
    throw problem('adb_command_failed');
  }
}
export function selectAppPage(pages, blank = false) {
  const allowed = pages.filter((page) => {
    if (blank && page.url() === 'about:blank') return true;
    try {
      const url = new URL(page.url());
      return url.protocol === 'http:' && url.hostname === '127.0.0.1' && Number(url.port) > 0
        && !url.username && !url.password && ['/library', '/article', '/map'].includes(url.pathname)
        && [...url.searchParams.keys()].every((key) => ['pack', 'url', 'source'].includes(key));
    } catch { return false; }
  });
  if (allowed.length !== 1 || pages.length !== 1) throw problem('app_webview_not_unique');
  return allowed[0];
}
export async function observeFreshEntryDocuments(session, record) {
  const { frameTree } = await session.send('Page.getFrameTree');
  let mainFrameId = frameTree.frame.id; // Kept in memory only, never copied into evidence.
  const event = (kind, url) => {
    if (url === 'about:blank') return;
    try { record({ kind, path: new URL(url).pathname }); }
    catch { record({ kind, path: '[invalid-document]' }); }
  };
  const document = ({ frame }) => {
    if (frame.parentId) return;
    mainFrameId = frame.id;
    event('commit', frame.url);
  };
  const sameDocument = ({ frameId, url }) => {
    if (frameId === mainFrameId) event('same_document', url);
  };
  session.on('Page.frameNavigated', document);
  session.on('Page.navigatedWithinDocument', sameDocument);
  const stop = () => {
    session.off('Page.frameNavigated', document);
    session.off('Page.navigatedWithinDocument', sameDocument);
  };
  try { await session.send('Page.enable'); }
  catch (error) { stop(); throw error; }
  return stop;
}

export function qualifyFreshEntry(events) {
  const responses = events.filter((event) => event.kind === 'response');
  const commits = events.filter((event) => event.kind === 'commit');
  const firstCommit = events.findIndex((event) => event.kind === 'commit');
  if (commits.some((event) => event.path !== '/library')
    || events.some((event, index) => event.kind === 'same_document' && (event.path !== '/library' || index <= firstCommit))
    || responses.some((event) => event.status >= 200 && event.status < 300 && event.path !== '/library')) {
    throw problem('fresh_entry_preloaded');
  }
  if (responses.some((event) => !(event.path === '/launch' && event.status === 302)
    && !(event.path === '/library' && event.status >= 200 && event.status < 300 && event.mime === 'text/html'))) {
    throw problem('fresh_entry_unexpected_response');
  }
  if (events.length > 32 || commits.length !== 1 || responses.filter((event) => event.path === '/library').length !== 1) {
    throw problem('fresh_entry_not_unique');
  }
}
export function parsePss(text) {
  const pid = Number(text.match(/MEMINFO in pid (\d+) \[org\.tonalli\.spike\]/)?.[1]);
  const pss = Number(text.match(/TOTAL PSS:\s+(\d+)/)?.[1] ?? text.match(/^\s*TOTAL\s+(\d+)\s/m)?.[1]);
  if (!Number.isSafeInteger(pid) || pid < 1 || !Number.isSafeInteger(pss) || pss < 1) throw problem('invalid_pss');
  return { pid, appProcessPssKiB: pss, rendererPss: 'not measured; manual attribution required' };
}
export function parseBenchmarkSession(text, { pid, dataset, manifest, ui, profile }) {
  let value;
  if (Buffer.byteLength(text) > 4096) throw problem('invalid_benchmark_session');
  try { value = JSON.parse(text); } catch { throw problem('invalid_benchmark_session'); }
  if (value.version !== 1 || value.state !== 'ready' || value.pid !== pid || value.dataset_id !== dataset
    || value.manifest_sha256 !== manifest || value.ui_id !== ui || value.source !== 'internal_mmap'
    || !/^m1-perf-[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(value.profile)
    || !/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(value.session_id)
    || (profile && value.profile !== profile) || !Number.isSafeInteger(value.process_started_elapsed_ms)
    || value.process_started_elapsed_ms < 0) throw problem('invalid_benchmark_session');
  return { pid, sessionId: value.session_id, profile: value.profile, processStartedElapsedMs: value.process_started_elapsed_ms,
    datasetId: dataset, manifestSha256: manifest, uiId: ui, source: 'internal_mmap' };
}
export class AndroidWebView {
  constructor({ serial, adb = path.join(process.env.LOCALAPPDATA || '', 'Android/Sdk/platform-tools/adb.exe'), execute: runner = execute, inspectHost = (port) => inspectAdbHost(adb, port) }) {
    if (!serial || !/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,199}$/.test(serial)) throw problem('invalid_serial');
    this.serial = serial; this.adb = adb; this.execute = runner; this.port = null; this.pid = null;
    this.inspectHost = inspectHost;
  }
  command(...args) { return this.execute(this.adb, ['-s', this.serial, ...args]); }
  async ownedPid() {
    if ((await this.command('get-state')).trim() !== 'device') throw problem('device_not_ready');
    const value = (await this.command('shell', 'pidof', '-s', APP)).trim();
    if (!/^[1-9]\d{0,8}$/.test(value)) throw problem('app_process_not_unique');
    const name = (await this.command('shell', 'run-as', APP, 'cat', `/proc/${value}/cmdline`)).replace(/\0+$/, '');
    if (name !== APP) throw problem('app_process_mismatch');
    return Number(value);
  }
  async forward() {
    this.pid = await this.ownedPid();
    this.socket = `localabstract:webview_devtools_remote_${this.pid}`;
    const sockets = await this.command('shell', 'cat', '/proc/net/unix');
    if (!sockets.split(/\r?\n/).some((line) => line.trim().split(/\s+/).at(-1) === `@webview_devtools_remote_${this.pid}`)) {
      throw problem('owned_webview_debug_socket_unavailable');
    }
    const serverPid = await this.inspectHost();
    const port = (await this.command('forward', '--no-rebind', 'tcp:0', this.socket)).trim();
    if (!/^\d{1,5}$/.test(port) || Number(port) < 1 || Number(port) > 65535) throw problem('invalid_forward_port');
    this.port = Number(port);
    try {
      await this.verifyForward();
      if (await this.inspectHost(this.port) !== serverPid) throw problem('unsafe_adb_host');
    } catch (error) {
      await this.close();
      throw error;
    }
    return `http://127.0.0.1:${this.port}`;
  }
  async verifyForward() {
    const list = await this.command('forward', '--list');
    if (!list.split(/\r?\n/).some((line) => {
      const fields = line.trim().split(/\s+/);
      return fields[0] === this.serial && fields[1] === `tcp:${this.port}` && fields[2] === this.socket;
    })) throw problem('forward_ownership_unconfirmed');
  }
  async close() {
    if (this.port === null) return;
    await this.verifyForward();
    await this.command('forward', '--remove', `tcp:${this.port}`);
    this.port = null;
  }
  async memory() {
    if (await this.ownedPid() !== this.pid) throw problem('app_process_changed');
    const value = parsePss(await this.command('shell', 'dumpsys', 'meminfo', APP));
    if (value.pid !== this.pid) throw problem('app_process_changed');
    return value;
  }
  async installedApkHash() {
    const location = (await this.command('shell', 'pm', 'path', APP)).trim();
    if (!/^package:\/data\/app\/[A-Za-z0-9_./~=+-]+\/base\.apk$/.test(location)) throw problem('unsupported_apk_layout');
    const result = await this.command('shell', 'sha256sum', location.slice('package:'.length));
    const hash = result.trim().split(/\s+/)[0];
    if (!/^[a-f0-9]{64}$/.test(hash)) throw problem('installed_apk_hash_unavailable');
    return hash;
  }
  async assertUi(id) {
    if (!/^[a-f0-9]{64}$/.test(id)) throw problem('invalid_ui_id');
    await this.command('shell', 'run-as', APP, 'test', '-d', `files/web-ui/ui-${id}`);
  }
  async benchmarkSession(configuration) {
    return parseBenchmarkSession(await this.command('exec-out', 'run-as', APP, 'cat', 'files/benchmarks/webview-session.json'),
      { ...configuration, pid: this.pid });
  }
  async benchmarkAction(action, { profile, dataset, manifest }) {
    validateBenchmark({ profile, dataset, manifest });
    if (!['WEBVIEW_BENCHMARK', 'WEBVIEW_BENCHMARK_LOAD'].includes(action)) throw problem('invalid_benchmark_action');
    await this.command('shell', 'am', 'start', '-W', '-n', `${APP}/.MainActivity`, '-a', `${APP}.${action}`,
      '--es', 'profile', profile, '--es', 'dataset_id', dataset, '--es', 'manifest_sha256', manifest);
  }
  async startFresh(configuration) {
    validateBenchmark(configuration);
    if ((await this.command('get-state')).trim() !== 'device') throw problem('device_not_ready');
    if (!(await this.command('shell', 'dumpsys', 'package', APP)).includes('DEBUGGABLE')) throw problem('app_not_debuggable');
    await this.command('shell', 'am', 'force-stop', APP);
    if ((await this.command('shell', 'pidof', '-s', APP)).trim()) throw problem('app_stop_unconfirmed');
    await this.benchmarkAction('WEBVIEW_BENCHMARK', configuration);
  }
}
