const packId = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const canonical = value => typeof value === 'string' && value.length > 0 && value.length <= 4096
  && !/^[a-z][a-z0-9+.-]*:/i.test(value) && !value.startsWith('/') && !/[\\\x00-\x1f\x7f]/.test(value)
  && !value.split('/').some(part => part === '.' || part === '..');
const rowValid = row => typeof row.title === 'string' && row.title.length <= 4096 && canonical(row.url);
async function json(url, csrf, signal, fetcher) {
  const response = await fetcher(url, { signal, credentials: 'same-origin', cache: 'no-store',
    headers: { Accept: 'application/json', 'X-Tonalli-Csrf': csrf } });
  if (!response.ok) throw new Error(`library_${response.status}`);
  return response.json();
}
export async function loadPacks(csrf, signal, fetcher = fetch) {
  const body = await json('/api/packs', csrf, signal, fetcher);
  if (!Array.isArray(body.packs) || body.packs.some(item => !packId(item.id) || typeof item.title !== 'string'
    || item.title.length > 4096 || !Number.isSafeInteger(item.total) || item.total < 0
    || Object.keys(item).some(key => !['id', 'title', 'total'].includes(key)))
    || new Set(body.packs.map(item => item.id)).size !== body.packs.length) throw new Error('invalid_packs');
  return body.packs;
}
export async function loadSuggestions(pack, query, csrf, signal, fetcher = fetch) {
  if (!packId(pack) || new TextEncoder().encode(query).length > 4096) throw new Error('invalid_suggestions');
  const params = new URLSearchParams({ pack, q: query, limit: '20' });
  const body = await json(`/api/zim/suggest?${params}`, csrf, signal, fetcher);
  if (body.pack !== pack || !Array.isArray(body.hits) || body.hits.length > 20 || body.hits.some(row => !rowValid(row))) throw new Error('invalid_suggestions');
  return body;
}
export async function loadBodySearch(pack, query, csrf, signal, fetcher = fetch) {
  if (!packId(pack) || typeof query !== 'string' || !query.trim() || /[\x00-\x1f\x7f]/.test(query)
    || new TextEncoder().encode(query).length > 4096) throw new Error('invalid_body_search');
  const params = new URLSearchParams({ pack, q: query, limit: '20' });
  const response = await fetcher(`/api/zim/search?${params}`, { signal, credentials: 'same-origin', cache: 'no-store',
    headers: { Accept: 'application/json', 'X-Tonalli-Csrf': csrf } });
  if (!response.ok) {
    let code;
    if (response.status === 503) { try { code = (await response.json()).error; } catch { /* Stable local error below. */ } }
    throw new Error(['index_unavailable', 'media_removed'].includes(code) ? code : `library_${response.status}`);
  }
  const body = await response.json();
  if (!body || body.pack !== pack || Object.keys(body).length !== 2 || !Object.hasOwn(body, 'hits')
    || !Array.isArray(body.hits) || body.hits.length > 20
    || body.hits.some(row => !row || !rowValid(row) || !Number.isFinite(row.score)
      || Object.keys(row).some(key => !['title', 'url', 'score'].includes(key)))
    || new Set(body.hits.map(row => row.url)).size !== body.hits.length) throw new Error('invalid_body_search');
  return body;
}
export function titleWindow(total, requested) {
  if (!Number.isSafeInteger(total) || total < 0) throw new Error('invalid_title_total');
  const maximum = total === 0 ? 0 : Math.floor((total - 1) / 10000) * 10000;
  const start = Math.min(maximum, Math.floor(Math.max(0, Number.isSafeInteger(requested) ? requested : 0) / 10000) * 10000);
  const count = Math.min(10000, total - start);
  return { start, count, previous: start > 0 ? start - 10000 : null, next: start + count < total ? start + count : null };
}
export async function loadTitlePage(pack, offset, csrf, signal, fetcher = fetch) {
  if ((pack !== undefined && !packId(pack)) || !Number.isSafeInteger(offset) || offset < 0) throw new Error('invalid_title_page');
  const query = new URLSearchParams({ offset: String(offset), limit: '200' });
  if (pack) query.set('pack', pack);
  const page = await json(`/api/zim/titles?${query}`, csrf, signal, fetcher);
  if (!Number.isSafeInteger(page.total) || page.total < 0 || page.offset !== offset || typeof page.pack !== 'string'
    || (pack !== undefined && page.pack !== pack) || !packId(page.pack)
    || !Array.isArray(page.rows) || page.rows.length > Math.min(200, Math.max(0, page.total - offset)) || page.rows.some(row => !rowValid(row))) {
    throw new Error('invalid_title_page');
  }
  return page;
}
