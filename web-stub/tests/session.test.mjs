import assert from 'node:assert/strict';
import test from 'node:test';
import * as session from '../session.js';
const { loadSession } = session;

test('TV capability requires explicit true and grants no other private access',async()=>{
  for(const private_tv of [true,false,undefined,null,'true',1,{},[]]){
    const info=await session.loadSessionInfo(async()=>Response.json({csrf:'csrf',private_tv}));
    assert.equal(info.privateTv,private_tv===true);assert.equal(info.privateNotes,false);assert.equal(info.privateContent,false);assert.equal(info.privateAi,false);
  }
});

test('session bootstrap requests the authenticated same-origin endpoint', async () => {
  let requested;
  const csrf = await loadSession(async (url, options) => {
    requested = { url, options };
    return new Response(JSON.stringify({ csrf: 'test-session-csrf' }));
  });
  assert.equal(csrf, 'test-session-csrf');
  assert.equal(requested.url, '/api/session');
  assert.equal(requested.options.credentials, 'same-origin');
  assert.equal(requested.options.cache, 'no-store');
  assert.deepEqual(requested.options.headers, { Accept: 'application/json' });
});

test('session bootstrap rejects revoked credentials and malformed responses', async () => {
  await assert.rejects(loadSession(async () => new Response('', { status: 401 })));
  for (const body of [{}, { csrf: '' }, { csrf: 123 }]) {
    await assert.rejects(loadSession(async () => new Response(JSON.stringify(body))));
  }
});

test('session info enables private notes only for an explicit boolean capability', async () => {
  for (const private_notes of [true, false, undefined, null, 'true', 1, {}, []]) {
    const info = await session.loadSessionInfo(async () => new Response(JSON.stringify({ csrf: 'session-csrf', private_notes })));
    assert.deepEqual(info, { csrf: 'session-csrf', privateNotes: private_notes === true, privateAi: false, privateAiManagement:false, privateContent: false, privateIndexPreparation: false,sharedMedia:false,lanRoom:false,privateTv:false,privateRemote:false,remoteLibrary:false,chatBroker:false,chatUser:undefined,privateMesh:false });
  }
});

test('LAN room capability requires explicit true and confers no private access',async()=>{
  for(const lan_room of [true,false,undefined,null,'true',1,{},[]]){const info=await session.loadSessionInfo(async()=>Response.json({csrf:'csrf',lan_room}));assert.equal(info.lanRoom,lan_room===true);assert.equal(info.privateNotes,false);assert.equal(info.privateContent,false);assert.equal(info.privateAi,false);}
});

test('shared file browsing needs its own explicit capability and grants no private access',async()=>{
  for(const shared_media of [true,false,undefined,null,'true',1,{},[]]){
    const info=await session.loadSessionInfo(async()=>Response.json({csrf:'csrf',shared_media}));
    assert.equal(info.sharedMedia,shared_media===true);assert.equal(info.privateNotes,false);assert.equal(info.privateContent,false);
  }
});

test('content management is private only for an explicit capability, independent of notes and AI', async () => {
  for (const private_content of [true, false, undefined, null, 'true', 1, {}, []]) {
    const info = await session.loadSessionInfo(async () => Response.json({ csrf: 'session-csrf', private_notes: true, private_ai: true, private_content }));
    assert.equal(info.privateContent, private_content === true);
  }
});

test('private AI capability is enabled only by explicit true, never inherited from private notes', async () => {
  for (const private_ai of [true, false, undefined, null, 'true', 1, {}, []]) {
    const info = await session.loadSessionInfo(async () => Response.json({ csrf: 'session-csrf', private_notes: true, private_ai }));
    assert.equal(info.privateAi, private_ai === true);
  }
});

test('session info preserves bootstrap credential validation', async () => {
  await assert.rejects(() => session.loadSessionInfo(async () => new Response('', { status: 401 })));
  for (const body of [{ private_notes: true }, { csrf: '', private_notes: true }, { csrf: 123, private_notes: true }]) {
    await assert.rejects(() => session.loadSessionInfo(async () => new Response(JSON.stringify(body))));
  }
});

test('article preparation requires its own explicit private capability', async () => {
  for (const private_index_preparation of [true, false, undefined, null, 'true', 1, {}, []]) {
    const info = await session.loadSessionInfo(async () => Response.json({ csrf: 'csrf', private_content: true, private_index_preparation }));
    assert.equal(info.privateIndexPreparation, private_index_preparation === true);
  }
});
