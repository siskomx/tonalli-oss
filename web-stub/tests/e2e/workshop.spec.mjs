import assert from 'node:assert/strict';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {randomUUID,createHash} from 'node:crypto';
import path from 'node:path';
import {root,startArticleHost} from './article-host.mjs';
import {destination,appWords} from './consumer-gui-flow.mjs';
const output=path.join(root,'target/workshop-browser',randomUUID());await mkdir(output,{recursive:true});
const fixtureRoot=path.join(root,'web-stub/tests/fixtures/workshop'),manifest=JSON.parse(await readFile(path.join(fixtureRoot,'manifest.json'),'utf8'));
const fixtures=await Promise.all(manifest.files.map(async row=>{const bytes=await readFile(path.join(fixtureRoot,row.name));assert.equal(bytes.length,row.bytes);assert.equal(createHash('sha256').update(bytes).digest('hex'),row.sha256);return{...row,data:[...bytes]};}));
let host,progress='start';const errors=[],dialogs=[];
try{
 host=await startArticleHost({bootstrap:'request',headless:true});const{page,origin}=host;
 page.on('pageerror',error=>errors.push(error.message));page.on('dialog',async dialog=>{dialogs.push(dialog.type());await dialog.dismiss();});
 await page.goto(origin+'/workshop',{waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready);const t=await appWords(page);
 assert(await page.getByText(t['workshop.empty'],{exact:true}).isVisible());
 const files=await page.evaluate(async fixtures=>{
  const{csrf}=await(await fetch('/api/session')).json(),h={'X-Tonalli-Csrf':csrf,'Content-Type':'application/json'};
  const setup=await fetch('/api/admin/setup',{method:'POST',headers:h,body:JSON.stringify({passphrase:'Owned Workshop browser fixture'})});if(!setup.ok)throw Error('Owned Admin setup failed');const{token}=await setup.json();
  for(const row of fixtures){const response=await fetch(`/api/files/upload?${new URLSearchParams({name:row.name,folder:'Workshop'})}`,{method:'POST',headers:{'X-Tonalli-Csrf':csrf,'X-Tonalli-Admin':token,'Content-Type':'application/octet-stream'},body:Uint8Array.from(row.data)});if(!response.ok)throw Error('Owned model import failed');}
  await fetch('/api/admin/lock',{method:'POST',headers:h});if((await fetch('/api/ping',{headers:h})).status!==404)throw Error('Production host only');return(await(await fetch('/api/files',{headers:h})).json()).files;
 },fixtures);
 assert.equal(files.length,5);await page.reload({waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready);
 async function open(name,count){await destination(page,'workshop');await page.locator('#workshop-search').fill(name);await page.getByRole('link',{name,exact:true}).click();if(count){await page.waitForFunction(count=>window.__ready?.ready&&window.__ready.triangles===count,count);await page.waitForFunction(()=>document.querySelector('[data-testid="model-canvas"] canvas')?.width>100);}}
 async function settle(){await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));}
 async function shot(name){await settle();const bytes=await page.getByTestId('model-canvas').screenshot({path:path.join(output,name+'.png')});return createHash('sha256').update(bytes).digest('hex');}
 await open('Block-ascii.stl',12);progress='ascii';const initial=await shot('ascii');
 await page.getByRole('button',{name:t['workshop.left'],exact:true}).click();const rotated=await shot('rotated');assert.notEqual(rotated,initial,'Rotate must visibly change the model');
 await page.getByRole('button',{name:t['workshop.zoomIn'],exact:true}).click();const zoomed=await shot('zoomed');assert.notEqual(zoomed,rotated,'Zoom must visibly change the model');
 await page.getByRole('button',{name:t['workshop.reset'],exact:true}).click();const reset=await shot('reset');assert.equal(reset,initial,'Reset must restore the initial view');
 // Exercise pointer orbit/wheel as well as accessible buttons.
 const canvas=page.getByTestId('model-canvas').locator('canvas'),box=await canvas.boundingBox();assert(box);await page.mouse.move(box.x+box.width*.5,box.y+box.height*.5);await page.mouse.down();await page.mouse.move(box.x+box.width*.65,box.y+box.height*.6,{steps:8});await page.mouse.up();const dragged=await shot('dragged');assert.notEqual(dragged,reset);await page.mouse.wheel(0,-200);const wheeled=await shot('wheel');assert.notEqual(wheeled,dragged);
 await open('Block-binary.STL',12);await shot('binary');await open('Block.obj',12);await shot('obj');await open('Components.3mf',24);await shot('components');progress='three-formats';
 // Losing graphics resources must leave a usable error and retry path.
 await page.getByTestId('model-canvas').locator('canvas').evaluate(canvas=>canvas.getContext('webgl2').getExtension('WEBGL_lose_context').loseContext());
 await page.getByText(t['workshop.graphicsFailed'],{exact:true}).waitFor();await page.getByTestId('model-preview').getByRole('button',{name:t['files.refresh'],exact:true}).click();await page.waitForFunction(()=>window.__ready?.ready&&document.querySelector('[data-testid="model-canvas"] canvas')?.width>100);await page.getByRole('button',{name:t['workshop.left'],exact:true}).waitFor({state:'visible'});assert.equal(await page.getByRole('alert').count(),0);
 await open('Broken.stl');await page.getByText(t['workshop.unsupported'],{exact:true}).waitFor();progress='bad-file';
 await page.setViewportSize({width:500,height:850});await open('Components.3mf',24);assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));await shot('narrow');
 const after=await page.evaluate(async()=>{const{csrf}=await(await fetch('/api/session')).json();return(await(await fetch('/api/files',{headers:{'X-Tonalli-Csrf':csrf}})).json()).files;});assert.deepEqual(after,files);
 await page.reload({waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready&&window.__ready.triangles===24);progress='reload';
 assert.deepEqual(errors,[]);assert.deepEqual(dialogs,[]);assert.deepEqual(host.blocked,[]);
 const result={label:'desktop production browser integration; not native Windows or Android acceptance',stlAsciiAndBinary:true,objQuads:true,threeMfComponents:true,rotateZoomReset:true,pointerOrbitAndWheel:true,contextLossUsefulErrorAndRetry:true,corruptModelUsefulError:true,narrowViewport:true,reloadWorks:true,managedSourcesUnchanged:true,errors,dialogs,external:host.blocked,cli_sha256:host.cliSha256,ui_sha256:host.distSha256};
 await writeFile(path.join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({output,...result}));
}catch(error){console.log(JSON.stringify({output,progress,errors,dialogs}));if(host)await host.page.screenshot({path:path.join(output,'failure.png')}).catch(()=>{});throw error;}finally{await host?.close();}
