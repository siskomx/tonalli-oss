const prefix='tonalli-chat:';
export function connectionCode(profile){
  const bytes=new TextEncoder().encode(JSON.stringify(profile));if(bytes.length>20000)throw Error('Connection details are too large');
  return prefix+btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replaceAll('=','');
}
export function parseConnectionCode(text){
  if(typeof text!=='string'||text.length>30000)throw Error('Invalid connection details');
  let value;
  if(text.startsWith(prefix)){
    const encoded=text.slice(prefix.length);if(!/^[A-Za-z0-9_-]+$/.test(encoded))throw Error('Invalid connection details');
    const bytes=Uint8Array.from(atob(encoded.replaceAll('-','+').replaceAll('_','/')),c=>c.charCodeAt(0));
    value=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes));
  }else value=JSON.parse(text);
  if(!value||typeof value.url!=='string'||typeof value.user!=='string'||value.user.length>64)throw Error('Invalid connection details');
  const certificate=value.certificate??'',password=value.password??'';
  if(typeof certificate!=='string'||certificate.length>16384||typeof password!=='string'||password.length>256)throw Error('Invalid connection details');
  const url=new URL(value.url);if(url.protocol!=='https:'||url.username||url.password||url.pathname!=='/'||url.hash||url.search)throw Error('Invalid connection address');
  return {url:url.origin,user:value.user,password,certificate};
}
