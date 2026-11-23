import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import path from 'node:path';
export async function openHistory(directory, executable) {
  const key = (await readFile(path.join(directory,'history.key'),'utf8')).trim();
  if (!/^[a-f0-9]{64}$/i.test(key)) throw Error('Invalid history key');
  const child = spawn(executable,[],{stdio:['pipe','pipe','inherit'],windowsHide:true});
  let next = 0, closed = false;
  const pending = new Map();
  const lines = createInterface({input:child.stdout});
  const fail = () => { closed=true; for (const p of pending.values()) p.reject(Error('Encrypted history unavailable')); pending.clear(); };
  child.on('error',fail); child.on('exit',fail);
  const ready = new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{reject(Error('History startup timed out'));child.kill();},10000);
    child.once('error',reject); child.once('exit',()=>reject(Error('History stopped')));
    lines.once('line',line=>{clearTimeout(timer); try {if(JSON.parse(line).ready===true) resolve();else reject(Error('Invalid history startup'));} catch {reject(Error('Invalid history startup'));}});
  });
  child.stdin.write(JSON.stringify({path:path.join(directory,'history.db'),key})+'\n');
  await ready;
  lines.on('line',line=>{
    if (line.length > 1048576) {child.kill();return;}
    try {const response=JSON.parse(line),p=pending.get(response.id);if(p){pending.delete(response.id);response.error?p.reject(Error(response.error)):p.resolve(response.result);}} catch {child.kill();}
  });
  return {
    request(value) {
      if (closed || pending.size >= 64 || child.stdin.writableLength > 262144) return Promise.reject(Error('History busy or unavailable'));
      return new Promise((resolve,reject)=>{
        const id=++next;
        const timer=setTimeout(()=>{child.kill();fail();},10000);
        pending.set(id,{resolve:v=>{clearTimeout(timer);resolve(v);},reject:e=>{clearTimeout(timer);reject(e);}});
        child.stdin.write(JSON.stringify({...value,id})+'\n',error=>{if(error){child.kill();fail();}});
      });
    },
    close() { child.stdin.end(); return new Promise(resolve=>{if(closed)return resolve();child.once('exit',resolve);setTimeout(()=>child.kill(),3000).unref();}); },
  };
}
