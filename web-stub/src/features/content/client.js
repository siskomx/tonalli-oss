const id = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const bytes = value => Number.isSafeInteger(value) && value >= 0;
const text = (value, max = 512) => typeof value === 'string' && value.trim().length > 0 && value.length <= max && !/[\x00-\x1f\x7f]/.test(value);
const nullable = (value, check) => value === null || check(value);
const fields = (value, keys) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
const states = new Set(['queued', 'downloading', 'verifying', 'installing', 'pausing', 'paused', 'cancelling', 'completed', 'cancelled', 'failed', 'waiting_wifi']);
const codes = new Set(['failed', 'invalid_request', 'invalid_response', 'session', 'request_unconfirmed', 'content_unavailable', 'download_unavailable',
  'catalog_changed', 'busy', 'not_found', 'in_use', 'no_space', 'waiting_wifi', 'unsupported_kind', 'incomplete_bundle', 'source_unavailable',
  'publication_uncertain', 'checkpoint_failed', 'io_error', 'checksum', 'network_error']);

export class ContentError extends Error {
  constructor(code) { super(`content.error.${code}`); this.code = code; }
}
const invalid = () => { throw new ContentError('invalid_response'); };
const identity = (entryId, revision) => {
  if (!id(entryId) || !Number.isSafeInteger(revision) || revision < 1) throw new ContentError('invalid_request');
  return { entryId, catalogRevision: revision };
};
const baseEntry = value => id(value.id) && text(value.title) && ['zim', 'map', 'model'].includes(value.kind)
  && text(value.language, 35) && text(value.version, 128) && bytes(value.bytes) && value.bytes > 0;
const unique = items => items.every(item => item && typeof item === 'object') && new Set(items.map(item => item.id)).size === items.length;
const jobValid = value => fields(value, ['id', 'entryId', 'state', 'downloadedBytes', 'totalBytes', 'verifiedBytes', 'errorCode'])
  && id(value.id) && id(value.entryId) && states.has(value.state) && bytes(value.downloadedBytes) && bytes(value.totalBytes)
  && value.totalBytes > 0 && bytes(value.verifiedBytes) && value.downloadedBytes <= value.totalBytes && value.verifiedBytes <= value.totalBytes
  && nullable(value.errorCode, code => typeof code === 'string' && /^[a-z0-9_]{1,64}$/.test(code));

async function request(url, csrf, signal, fetcher, method = 'GET', body) {
  let response;
  try {
    response = await fetcher(url, { method, signal, credentials: 'same-origin', cache: 'no-store',
      headers: { Accept: 'application/json', 'X-Tonalli-Csrf': csrf, ...(body ? { 'Content-Type': 'application/json' } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}) });
  } catch (error) {
    if (error?.name === 'AbortError') throw error;
    throw new ContentError(method === 'GET' ? 'failed' : 'request_unconfirmed');
  }
  if (!response.ok) {
    if ([401, 403].includes(response.status)) throw new ContentError('session');
    const result = await response.json().catch(() => null);
    throw new ContentError(codes.has(result?.error) ? result.error : 'failed');
  }
  try { return await response.json(); } catch { return invalid(); }
}

export async function loadCatalog(csrf, signal, fetcher = fetch) {
  const data = await request('/api/content/catalog', csrf, signal, fetcher);
  if (!fields(data, ['revision', 'entries', 'installed', 'downloadAvailable']) || !Number.isSafeInteger(data.revision) || data.revision < 1
    || typeof data.downloadAvailable !== 'boolean' || !Array.isArray(data.entries) || !Array.isArray(data.installed)
    || data.entries.length > 256 || data.installed.length > 256
    || !unique(data.entries) || !unique(data.installed)
    || data.entries.some(item => !fields(item, ['id', 'title', 'kind', 'language', 'version', 'bytes', 'installedVersion', 'installedReadable'])
      || !baseEntry(item) || !nullable(item.installedVersion, value => text(value, 128))
      || (item.installedVersion === null ? item.installedReadable !== null : typeof item.installedReadable !== 'boolean'))
    || data.installed.some(item => !fields(item, ['id', 'title', 'kind', 'language', 'version', 'bytes', 'readable']) || !baseEntry(item) || typeof item.readable !== 'boolean')) invalid();
  return data;
}

export async function loadJobs(csrf, signal, fetcher = fetch) {
  const data = await request('/api/content/jobs', csrf, signal, fetcher);
  if (!fields(data, ['jobs']) || !Array.isArray(data.jobs) || data.jobs.length > 64 || !unique(data.jobs) || data.jobs.some(item => !jobValid(item))) invalid();
  return data;
}

export async function preflight(entryId, revision, csrf, signal, fetcher = fetch) {
  const data = await request('/api/content/preflight', csrf, signal, fetcher, 'POST', identity(entryId, revision));
  if (!fields(data, ['entryId', 'availableBytes', 'downloadBytes', 'stagingBytes', 'requiredBytes', 'allowed', 'reason']) || data.entryId !== entryId
    || !['availableBytes', 'downloadBytes', 'stagingBytes', 'requiredBytes'].every(key => bytes(data[key])) || typeof data.allowed !== 'boolean'
    || !nullable(data.reason, value => typeof value === 'string' && /^[a-z0-9_]{1,64}$/.test(value))) invalid();
  return data;
}

function jobResult(data, entryId, jobId) {
  if (!fields(data, ['job']) || !jobValid(data.job) || (entryId && data.job.entryId !== entryId) || (jobId && data.job.id !== jobId)) invalid();
  return data;
}
export async function startDownload(entryId, revision, csrf, signal, fetcher = fetch) {
  return jobResult(await request('/api/content/jobs', csrf, signal, fetcher, 'POST', identity(entryId, revision)), entryId);
}
export async function controlJob(jobId, action, csrf, signal, fetcher = fetch) {
  if (!id(jobId) || !['pause', 'resume', 'cancel', 'retry'].includes(action)) throw new ContentError('invalid_request');
  return jobResult(await request(`/api/content/jobs/${jobId}/${action}`, csrf, signal, fetcher, 'POST'), null, jobId);
}
export async function removeInstalled(entryId, csrf, signal, fetcher = fetch) {
  if (!id(entryId)) throw new ContentError('invalid_request');
  const data = await request(`/api/content/installed/${entryId}`, csrf, signal, fetcher, 'DELETE');
  if (!fields(data, ['removed', 'reason']) || typeof data.removed !== 'boolean' || ![null, 'in_use'].includes(data.reason)) invalid();
  return data;
}
