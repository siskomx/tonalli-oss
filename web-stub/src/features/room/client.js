export function mergeRoom(previous,incoming){
  if(!incoming||!Array.isArray(incoming.messages)||incoming.messages.length>100||!Array.isArray(incoming.members)||incoming.members.length>129||!Number.isSafeInteger(incoming.latest)||incoming.latest<0)throw Error('room_unavailable');
  if(!incoming.active||!incoming.joined)return{...incoming,messages:[],members:[],latest:0};
  const same=previous?.epoch===incoming.epoch&&previous?.self_id===incoming.self_id&&previous.joined;
  if(same&&previous.latest>incoming.latest)return previous;
  const messages=new Map((same?previous.messages:[]).map(message=>[message.id,message]));
  for(const message of incoming.messages){if(!Number.isSafeInteger(message.id)||message.id<1||message.id>incoming.latest||typeof message.text!=='string'||Array.from(message.text).length>1000||!message.sender||typeof message.sender.label!=='string')throw Error('room_unavailable');messages.set(message.id,message);}
  return{...incoming,messages:[...messages.values()].sort((a,b)=>a.id-b.id).slice(-100)};
}
