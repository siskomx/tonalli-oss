// Adapted from Project NOMAD admin/inertia/lib/api.ts streamChatMessage at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: local Core endpoints/CSRF, role-preserving conversation, bounded strict SSE,
// existing upstream think splitter, explicit request-owned cancel; no Ollama/admin/session persistence.
import { ThinkTagSplitter } from './think_stream.js';
import { chatStreamErrorMessage } from './chat_stream.js';

const headers = (csrf) => ({ 'Content-Type': 'application/json', 'X-Tonalli-CSRF': csrf });
const failure = (response) => new Error(response.status === 401 || response.status === 403 ? 'chat.session'
  : response.status === 503 ? 'chat.unavailable' : 'chat.failed');

export async function loadModel(csrf, signal, fetcher = globalThis.fetch) {
  const response = await fetcher('/api/ai/model', { credentials: 'same-origin', cache: 'no-store', headers: headers(csrf), signal });
  if (!response.ok) throw failure(response);
  const model = await response.json().catch(() => null);
  if (!model || model.kind !== 'generation' || typeof model.model !== 'string' || !model.model || model.model.length > 256
    || !['file_bytes', 'training_context_tokens', 'embedding_dimensions', 'estimated_reserved_bytes']
      .every((key) => Number.isSafeInteger(model[key]) && model[key] > 0)) throw new Error('chat.failed');
  return model;
}

export async function cancelChat(csrf, requestId, fetcher = globalThis.fetch) {
  const response = await fetcher('/api/ai/cancel', { method: 'POST', credentials: 'same-origin', cache: 'no-store',
    headers: headers(csrf), body: JSON.stringify({ request_id: requestId }), keepalive: true });
  if (!response.ok) throw failure(response);
}

function modelStatus(value) {
  const id = text => typeof text === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(text);
  if (!value || !['off', 'loading', 'stopping', 'ready'].includes(value.state)
    || !(value.model === null || typeof value.model === 'string' && /^[A-Za-z0-9_.-]{1,128}$/.test(value.model))
    || !(value.error === null || ['model_missing', 'model_refused', 'model_failed'].includes(value.error))
    || typeof value.downloadAvailable !== 'boolean' || !Array.isArray(value.models) || value.models.length > 256
    || value.models.some(model => !model || !id(model.id) || typeof model.title !== 'string' || model.title.length > 512
      || typeof model.version !== 'string' || model.version.length > 128 || !Number.isSafeInteger(model.bytes) || model.bytes <= 0
      || typeof model.installed !== 'boolean' || Object.keys(model).some(key => !['id', 'title', 'version', 'bytes', 'installed'].includes(key)))
    || new Set(value.models.map(model => model.id)).size !== value.models.length) throw new Error('chat.failed');
  return value;
}

export async function loadModels(csrf, signal, fetcher = globalThis.fetch) {
  const response = await fetcher('/api/ai/models', { credentials: 'same-origin', cache: 'no-store', headers: headers(csrf), signal });
  if (!response.ok) throw failure(response);
  return modelStatus(await response.json());
}

export async function changeModel(csrf, action, model, signal, fetcher = globalThis.fetch) {
  if (!['load', 'off'].includes(action) || action === 'load' && (typeof model !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(model))) throw new Error('chat.failed');
  const response = await fetcher(`/api/ai/models/${action}`, { method: 'POST', credentials: 'same-origin', cache: 'no-store',
    headers: headers(csrf), body: JSON.stringify(action === 'load' ? { model } : {}), signal });
  if (!response.ok) throw failure(response);
  return modelStatus(await response.json());
}

export function checkedGrounding(value, pack) {
  if (!value || value.pack !== pack || !Array.isArray(value.sources) || value.sources.length > 3
    || value.sources.some((source, i) => source.number !== i + 1 || typeof source.title !== 'string' || source.title.length > 512
      || typeof source.url !== 'string' || source.url.length > 4096 || !/^[CA]\//.test(source.url)
      || /[\\\x00-\x1f\x7f]/.test(source.url) || source.url.split('/').some(part => part === '.' || part === '..')
      || typeof source.excerpt !== 'string' || source.excerpt.length > 1024)
    || !Array.isArray(value.citations) || value.citations.length > 3
    || new Set(value.citations).size !== value.citations.length
    || value.citations.some(n => !Number.isInteger(n) || n < 1 || n > value.sources.length)
    || !value.flags || !['uncited', 'invalid_citations', 'incomplete', 'truncated'].every(key => typeof value.flags[key] === 'boolean')
    || (value.no_sources !== undefined && typeof value.no_sources !== 'boolean')) throw new Error('chat.failed');
  return value;
}

export async function streamChat({ requestId, model, question, messages = [{ role: 'user', content: question }], pack, csrf, signal, onChunk, onDoneReason = () => {}, onGrounding = () => {}, fetcher = globalThis.fetch }) {
  if (!Array.isArray(messages) || !messages.length || messages.length > 33 || messages.length % 2 !== 1
    || messages.some((message, index) => message?.role !== (index % 2 === 0 ? 'user' : 'assistant')
      || typeof message.content !== 'string' || !message.content.trim() || message.content.includes('\0'))
    || messages.reduce((bytes, message) => bytes + new TextEncoder().encode(message.content).length, 0) > 65536 - '\n/no_think'.length) throw new Error('chat.contextFull');
  if (pack !== undefined && (!/^[A-Za-z0-9_-]{1,128}$/.test(pack) || new TextEncoder().encode(messages.at(-1).content).length > 4096)) throw new Error('chat.failed');
  const response = await fetcher('/api/ai/chat', { method: 'POST', credentials: 'same-origin', cache: 'no-store',
    headers: headers(csrf), body: JSON.stringify({ request_id: requestId, model, messages: messages.map(({ role, content }) => ({ role, content })), max_tokens: 128, ...(pack === undefined ? {} : { pack }) }), signal });
  if (!response.ok || !response.body) {
    if (response.status === 503) {
      const body = await response.json().catch(() => null);
      if (body?.error === 'ai_index_unavailable') throw new Error('chat.prepareFirst');
    }
    throw failure(response);
  }
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  const splitter = new ThinkTagSplitter();
  let buffer = '', received = 0, terminal = false;
  const consume = (line) => {
    if (!line.startsWith('data: ')) return;
    let data;
    try { data = JSON.parse(line.slice(6)); } catch { throw new Error('chat.failed'); }
    if (!data || typeof data !== 'object' || chatStreamErrorMessage(data) || terminal
      || typeof data.done !== 'boolean' || typeof data.message?.content !== 'string') throw new Error('chat.failed');
    const split = splitter.push(data.message.content);
    if (data.done) {
      if (!['stop', 'length', 'cancelled'].includes(data.done_reason)) throw new Error('chat.failed');
      if (pack !== undefined) onGrounding(checkedGrounding(data.grounding, pack));
      else if (data.grounding !== undefined) throw new Error('chat.failed');
      terminal = true;
      const tail = splitter.flush(); split.content += tail.content; split.thinking += tail.thinking;
      onDoneReason(data.done_reason);
    }
    onChunk(split.content, split.thinking, data.done);
  };
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      received += value.byteLength;
      if (received > 1024 * 1024) throw new Error('chat.failed');
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';
      for (const line of lines) consume(line.replace(/\r$/, ''));
    }
    buffer += decoder.decode();
    if (buffer.trim()) consume(buffer.replace(/\r$/, ''));
    if (!terminal) throw new Error('chat.failed');
  } finally {
    // A protocol failure must also release the server stream, not only its reader lock.
    if (!terminal) await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
