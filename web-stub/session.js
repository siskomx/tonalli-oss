export async function loadSession(fetcher = globalThis.fetch) {
  return (await loadSessionInfo(fetcher)).csrf;
}

export async function loadSessionInfo(fetcher = globalThis.fetch) {
  const response = await fetcher('/api/session', {
    credentials: 'same-origin',
    cache: 'no-store',
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) throw new Error('Local session unavailable');
  const { csrf, private_notes, private_mesh, private_ai, private_ai_management, private_content, private_index_preparation, shared_media,lan_room,private_tv,private_remote,remote_library,chat_broker,chat_user } = await response.json();
  if (typeof csrf !== 'string' || csrf.length === 0) {
    throw new Error('Invalid local session');
  }
  return { csrf, privateNotes: private_notes === true, privateAi: private_ai === true, privateAiManagement: private_ai_management === true, privateContent: private_content === true,
    privateIndexPreparation: private_index_preparation === true,sharedMedia:shared_media===true,lanRoom:lan_room===true,privateTv:private_tv===true,
    privateRemote:private_remote===true,remoteLibrary:remote_library===true,chatBroker:chat_broker===true,chatUser:chat_user,privateMesh:private_mesh===true };
}
