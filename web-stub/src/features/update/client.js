const headers = (csrf) => ({ 'Content-Type': 'application/json', 'X-Tonalli-CSRF': csrf });
const ERRORS = ['update_unavailable', 'update_signature', 'update_unreachable', 'update_no_space', 'update_invalid'];
const text = (value, max) => typeof value === 'string' && value.length <= max;

function checkedStatus(value) {
  const download = value?.download;
  const offer = value?.available;
  if (!value || typeof value.supported !== 'boolean' || !text(value.current, 64) || typeof value.enabled !== 'boolean'
    || !text(value.source, 2048) || !text(value.public_key, 64)
    || !['never', 'up_to_date', 'available', 'error'].includes(value.check)
    || !(value.error === null || ERRORS.includes(value.error))
    || !(value.platform === null || text(value.platform, 32))
    || !download || !['idle', 'running', 'done', 'failed'].includes(download.state)
    || !Number.isSafeInteger(download.downloaded) || !Number.isSafeInteger(download.total)
    || !(download.error === null || ERRORS.includes(download.error))
    || !(offer === null || offer && text(offer.version, 64) && text(offer.published, 32) && text(offer.notes, 16384)
      && text(offer.page, 2048) && Number.isSafeInteger(offer.bytes) && typeof offer.downloaded === 'boolean'
      && ['apply', 'install', 'link'].includes(offer.mode))) throw new Error('update.failed');
  return value;
}

async function send(csrf, path, method, body, fetcher) {
  const response = await fetcher(`/api/update${path}`, { method, credentials: 'same-origin', cache: 'no-store',
    headers: headers(csrf), ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
  if (response.status === 401 || response.status === 403) throw new Error('chat.session');
  if (!response.ok) {
    const reply = await response.json().catch(() => ({}));
    throw new Error(response.status === 400 ? 'update.invalidSettings' : reply.error === 'updates_off' ? 'update.off' : 'update.failed');
  }
  return checkedStatus(await response.json());
}

export const loadUpdate = (csrf, fetcher = globalThis.fetch) => send(csrf, '', 'GET', undefined, fetcher);
export const saveUpdateSettings = (csrf, settings, fetcher = globalThis.fetch) => send(csrf, '/settings', 'PUT',
  { enabled: !!settings.enabled, source: settings.source, public_key: settings.public_key }, fetcher);
export const checkForUpdate = (csrf, fetcher = globalThis.fetch) => send(csrf, '/check', 'POST', {}, fetcher);
export const downloadUpdate = (csrf, fetcher = globalThis.fetch) => send(csrf, '/download', 'POST', {}, fetcher);
export const cancelUpdate = (csrf, fetcher = globalThis.fetch) => send(csrf, '/cancel', 'POST', {}, fetcher);
