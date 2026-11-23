import {it,expect,vi} from 'vitest';
import {readingRequest} from '../../src/features/library/reading-client.js';

it('orders progress writes and subsequent reads, and releases failed requests',async()=>{
  let release;
  const blocked=new Promise(resolve=>{release=resolve;});
  const calls=[];
  vi.stubGlobal('fetch',vi.fn(async(url,options)=>{
    calls.push(options.method);
    if(calls.length===1){await blocked;throw Error('offline');}
    return{ok:true,json:async()=>({entries:[]})};
  }));
  try{
    const first=readingRequest('owned','POST',{position:1}).catch(error=>error.message);
    const second=readingRequest('owned','POST',{position:2});
    const third=readingRequest('owned');
    await new Promise(resolve=>setTimeout(resolve,0));
    expect(calls).toEqual(['POST']);
    release();expect(await first).toBe('offline');await second;await third;
    expect(calls).toEqual(['POST','POST','GET']);
  }finally{release();vi.unstubAllGlobals();}
});
