import assert from 'node:assert/strict';
import test from 'node:test';
import * as client from '../src/features/chat/client.js';
import { loadSessionInfo } from '../session.js';

const status = { state: 'off', model: null, error: null, downloadAvailable: true,
  models: [{ id: 'local-model', title: 'Local model', bytes: 1024, version: '1', installed: true }] };

test('model-management capability never implies an attached model', async () => {
  for (const value of [true, false, null, undefined, 'true', 1]) {
    const session = await loadSessionInfo(async () => Response.json({ csrf: 'csrf', private_ai_management: value }));
    assert.equal(session.privateAiManagement, value === true);
    assert.equal(session.privateAi, false);
  }
});

test('model controls use private IDs only and validate actual pending states', async () => {
  const calls = [];
  const fetcher = async (url, options) => { calls.push({ url, options }); return Response.json(status); };
  assert.deepEqual(await client.loadModels('csrf', undefined, fetcher), status);
  assert.equal(calls[0].url, '/api/ai/models');
  assert.equal(calls[0].options.credentials, 'same-origin');
  await client.changeModel('csrf', 'load', 'local-model', undefined, fetcher);
  assert.deepEqual(JSON.parse(calls[1].options.body), { model: 'local-model' });
  await client.changeModel('csrf', 'off', undefined, undefined, fetcher);
  assert.equal(calls[2].url, '/api/ai/models/off');
  assert.deepEqual(JSON.parse(calls[2].options.body), {});
  for (const bad of [{ ...status, state: 'invented' }, { ...status, models: [{ ...status.models[0], path: '/private/model.gguf' }] },
    { ...status, models: [{ ...status.models[0], id: '../model' }] }]) {
    await assert.rejects(client.loadModels('csrf', undefined, async () => Response.json(bad)), /chat.failed/);
  }
});
