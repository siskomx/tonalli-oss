const codes = new Set(['notes_locked', 'unlock_failed', 'already_unlocked', 'notes_not_initialized', 'vault_exists',
  'recovery_required', 'note_not_found', 'invalid_request', 'input_too_large', 'notes_unavailable', 'rotation_uncertain']);

export class NotesError extends Error {
  constructor(status, code) { super(code); this.status = status; this.code = code; }
}
const validId = (id) => Number.isSafeInteger(id) && id > 0;
const validNote = (note) => note && validId(note.id) && typeof note.title === 'string' && typeof note.body === 'string';
const invalid = () => { throw new NotesError(0, 'invalid_response'); };

export function createNotesClient(csrf, fetcher = globalThis.fetch) {
  async function request(path, { method = 'GET', body, signal, keepalive = false } = {}) {
    let response;
    try {
      response = await fetcher(`/api/notes${path}`, { method, signal, keepalive, credentials: 'same-origin', cache: 'no-store',
        headers: { Accept: 'application/json', 'X-Tonalli-Csrf': csrf, ...(body ? { 'Content-Type': 'application/json' } : {}) },
        ...(body ? { body: JSON.stringify(body) } : {}) });
    } catch (error) {
      if (error.name === 'AbortError') throw error;
      throw new NotesError(0, 'notes_unavailable');
    }
    if (!response.ok) {
      let code;
      try { code = (await response.json()).error; } catch {}
      throw new NotesError(response.status, codes.has(code) ? code : 'notes_unavailable');
    }
    if (response.status === 204) return null;
    try { return await response.json(); } catch { invalid(); }
  }
  async function listing(path, signal) {
    const value = await request(path, { signal });
    if (!value || !Array.isArray(value.notes) || value.notes.some((note) => !validNote(note))) invalid();
    return value.notes;
  }
  return {
    async exportRecovery(passphrase, signal) {
      const value = await request('/recovery', { method: 'POST', body: { passphrase }, signal });
      if (typeof value?.recovery_key !== 'string' || !/^[0-9a-f]{64}$/i.test(value.recovery_key)) invalid();
      return value.recovery_key;
    },
    async changePassphrase(kind, credential, next, signal) {
      if (!['passphrase', 'recovery_key'].includes(kind)) throw new NotesError(400, 'invalid_request');
      if (await request('/passphrase', { method: 'POST', body: { [kind]: credential, new_passphrase: next }, signal }) !== null) invalid();
    },
    async update(id, title, body, signal) {
      if (!validId(id)) throw new NotesError(400, 'invalid_request');
      if (await request(`/${id}`, { method: 'PUT', body: { title, body }, signal }) !== null) invalid();
    },
    async remove(id, signal) {
      if (!validId(id)) throw new NotesError(400, 'invalid_request');
      if (await request(`/${id}`, { method: 'DELETE', signal }) !== null) invalid();
    },
    async create(passphrase, signal) {
      const value = await request('/create', { method: 'POST', body: { passphrase }, signal });
      if (typeof value?.recovery_key !== 'string' || !/^[0-9a-f]{64}$/i.test(value.recovery_key)) invalid();
      return value.recovery_key;
    },
    async unlock(kind, credential, signal) {
      if (!['passphrase', 'recovery_key'].includes(kind)) throw new NotesError(400, 'invalid_request');
      const value = await request('/unlock', { method: 'POST', body: { [kind]: credential }, signal });
      if (value?.unlocked !== true) invalid();
    },
    async lock(options = {}) {
      const value = await request('/lock', { method: 'POST', ...options });
      if (value !== null) invalid();
    },
    list: (signal) => listing('', signal),
    search: (query, signal) => listing(`/search?${new URLSearchParams({ q: query })}`, signal),
    async add(title, body, signal) {
      const value = await request('', { method: 'POST', body: { title, body }, signal });
      if (!value || !validId(value.id)) invalid();
      return value;
    },
    async get(id, signal) {
      if (!validId(id)) throw new NotesError(400, 'invalid_request');
      const value = await request(`/${id}`, { signal });
      if (!validNote(value) || value.id !== id) invalid();
      return value;
    },
  };
}
