// Standalone adapter for Project NOMAD's private session/message operations.
import { checkedGrounding } from './client.js';

const idValid = value => typeof value === 'string' && /^[a-f0-9]{32}$/.test(value);
const integer = value => Number.isSafeInteger(value) && value >= 0;
const identifier = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(value);
const packValid = value => value === null || typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const failed = () => new Error('chat.historyFailed');
function summary(value) {
  if (!value || !idValid(value.id) || !integer(value.revision) || !value.revision
    || typeof value.title !== 'string' || !value.title.trim() || new TextEncoder().encode(value.title).length > 512
    || /[\x00-\x1f\x7f]/.test(value.title) || !identifier(value.model) || !packValid(value.pack) || !integer(value.updated)) throw failed();
  return value;
}
function record(value) {
  summary(value);
  if (!Array.isArray(value.messages) || !value.messages.length || value.messages.length > 128) throw failed();
  let bytes = 0, contextTurns = 0;
  for (const message of value.messages) {
    if (!message || !['user', 'assistant'].includes(message.role) || typeof message.content !== 'string'
      || message.content.includes('\0') || typeof message.context !== 'boolean' || typeof message.truncated !== 'boolean'
      || !integer(message.timestamp)) throw failed();
    bytes += new TextEncoder().encode(message.content).length;
    if (bytes > 256 * 1024) throw failed();
    if (message.context) {
      if (message.role !== (contextTurns % 2 === 0 ? 'user' : 'assistant') || !message.content.trim()) throw failed();
      contextTurns++;
    }
    if (message.grounding !== undefined) {
      if (message.role !== 'assistant' || value.pack === null) throw failed();
      checkedGrounding(message.grounding, value.pack);
    }
  }
  if (contextTurns % 2 !== 0) throw failed();
  return value;
}
async function request(csrf, method, id, body, signal) {
  if (id !== undefined && !idValid(id)) throw failed();
  const response = await fetch(`/api/ai/history${id === undefined ? '' : `/${id}`}`, {
    method, credentials: 'same-origin', cache: 'no-store', signal,
    headers: { 'Content-Type': 'application/json', 'X-Tonalli-CSRF': csrf },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  if (!response.ok) throw new Error(response.status === 401 || response.status === 403 ? 'chat.session'
    : response.status === 409 ? 'chat.historyChanged' : response.status === 507 ? 'chat.historyFull' : 'chat.historyFailed');
  return response.json();
}
export async function listConversations(csrf, signal) {
  const value = await request(csrf, 'GET', undefined, undefined, signal);
  if (!Array.isArray(value?.sessions) || value.sessions.length > 64) throw failed();
  const sessions = value.sessions.map(summary);
  if (new Set(sessions.map(session => session.id)).size !== sessions.length) throw failed();
  return sessions;
}
export async function openConversation(csrf, id, signal) {
  const value = record(await request(csrf, 'GET', id, undefined, signal));
  if (value.id !== id) throw failed();
  return value;
}
export async function saveConversation(csrf, id, input, signal) {
  const value = record(await request(csrf, 'PUT', id, input, signal));
  if (value.id !== id || value.revision !== input.revision + 1) throw failed();
  return value;
}
export async function removeConversation(csrf, session, signal) {
  const value = await request(csrf, 'DELETE', session.id, { revision: session.revision, confirmed: true }, signal);
  if (value?.removed !== true) throw failed();
}

export function visibleMessages(messages) {
  return messages.map(message => ({ role: message.role, content: message.content, context: message.context === true,
    timestamp: message.timestamp.getTime(), truncated: message.truncated === true,
    ...(message.grounding ? { grounding: message.grounding } : {}) }));
}
