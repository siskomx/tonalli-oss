import assert from 'node:assert/strict';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import path from 'node:path';
import {root,startArticleHost} from './article-host.mjs';
import {destination} from './consumer-gui-flow.mjs';

const output=path.join(root,'target/workshop-lifetime',randomUUID());
await mkdir(output,{recursive:true});
const host=await startArticleHost({bootstrap:'request',headless:true});
try{
 const {page,origin}=host;
 await page.addInitScript(()=>{
  const Native=window.ResizeObserver;
  window.ResizeObserver=class extends Native{
   constructor(callback){super(callback);this.callback=callback;}
   observe(element,options){
    if(element.dataset.testid==='model-canvas')window.testLateModelResize=()=>this.callback([],this);
    super.observe(element,options);
   }
  };
 });
 await page.goto(origin+'/workshop');await page.waitForFunction(()=>window.__ready?.ready);
 const bytes=await readFile(path.join(root,'web-stub/tests/fixtures/workshop/Block.obj'));
 await page.evaluate(async bytes=>{
  const{csrf}=await(await fetch('/api/session')).json(),headers={'X-Tonalli-Csrf':csrf,'Content-Type':'application/json'};
  const setup=await fetch('/api/admin/setup',{method:'POST',headers,body:JSON.stringify({passphrase:'Owned resize regression'})});
  if(!setup.ok)throw Error('Owned Admin setup failed');const{token}=await setup.json();
  const response=await fetch('/api/files/upload?name=Block.obj',{method:'POST',headers:{'X-Tonalli-Csrf':csrf,'X-Tonalli-Admin':token,'Content-Type':'application/octet-stream'},body:Uint8Array.from(bytes)});
  if(!response.ok)throw Error('Owned model import failed');await fetch('/api/admin/lock',{method:'POST',headers});
 },[...bytes]);
 await page.reload();await page.getByRole('link',{name:'Block.obj',exact:true}).click();
 await page.waitForFunction(()=>window.__ready?.triangles===12&&typeof window.testLateModelResize==='function');
 await destination(page,'library');
 assert.equal(await page.getByTestId('model-canvas').count(),0);
 await page.evaluate(()=>window.testLateModelResize());
 const receipt={label:'desktop production browser integration',late_resize_after_unmount:true,cli_sha256:host.cliSha256,ui_sha256:host.distSha256};
 await writeFile(path.join(output,'result.json'),JSON.stringify(receipt,null,2));console.log(JSON.stringify({output,...receipt}));
}finally{await host.close();}
