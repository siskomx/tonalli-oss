const packId = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const jobId = value => typeof value === 'string' && /^[a-f0-9]{32}$/.test(value);
const epoch = value => typeof value === 'string' && /^[1-9][0-9]{0,19}$/.test(value)
  && (value.length < 20 || value <= '18446744073709551615');
const count = value => Number.isSafeInteger(value) && value >= 0;
const code = value => typeof value === 'string' && /^[a-z][a-z0-9_]{0,63}$/.test(value);
const states = new Set(['unprepared', 'queued', 'preparing', 'pausing', 'paused', 'cancelling', 'cancelled', 'ready', 'failed', 'stale_source']);
const phases = new Set(['checking', 'indexing', 'validating', 'activating']);
const keys = ['pack', 'jobId', 'epoch', 'state', 'phase', 'processedArticles', 'committedArticles', 'reason'];

export class PreparationError extends Error {
  constructor(code) { super(code); this.code = code; }
}
const fail = code => { throw new PreparationError(code); };
function snapshot(value, pack) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).length !== keys.length || !keys.every(key => Object.hasOwn(value, key))
    || value.pack !== pack || !states.has(value.state)
    || !(value.phase === null || phases.has(value.phase))
    || !(value.reason === null || code(value.reason))
    || !(value.jobId === null && value.epoch === null || jobId(value.jobId) && epoch(value.epoch))
    || !(value.processedArticles === null && value.committedArticles === null
      || count(value.processedArticles) && count(value.committedArticles) && value.committedArticles <= value.processedArticles)) {
    fail('invalid_preparation_response');
  }
  return value;
}
async function request(path, pack, csrf, signal, fetcher, body) {
  if (!packId(pack)) fail('invalid_preparation_request');
  let response;
  try {
    response = await fetcher(path, { method: body ? 'POST' : 'GET', signal,
      credentials: 'same-origin', cache: 'no-store',
      headers: { Accept: 'application/json', 'X-Tonalli-Csrf': csrf, ...(body ? { 'Content-Type': 'application/json' } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}) });
  } catch (error) {
    if (error?.name === 'AbortError') throw error;
    fail(body ? 'request_unconfirmed' : 'failed');
  }
  let data;
  try { data = await response.json(); } catch (error) {
    if (error?.name === 'AbortError') throw error;
    fail(response.ok ? 'invalid_preparation_response' : 'failed');
  }
  if (!response.ok) fail([401, 403].includes(response.status) ? 'session' : code(data?.error) ? data.error : 'failed');
  return snapshot(data, pack);
}
export async function loadPreparation(pack, csrf, signal, fetcher = fetch) {
  return request(`/api/zim/index-status?${new URLSearchParams({ pack })}`, pack, csrf, signal, fetcher);
}
export async function startPreparation(pack, csrf, signal, fetcher = fetch) {
  return request('/api/zim/index-start', pack, csrf, signal, fetcher, { pack });
}
export async function controlPreparation(pack, id, revision, action, csrf, signal, fetcher = fetch) {
  if (!jobId(id) || !epoch(revision) || !['pause', 'resume', 'cancel'].includes(action)) fail('invalid_preparation_request');
  return request('/api/zim/index-control', pack, csrf, signal, fetcher, { jobId: id, epoch: revision, action });
}
