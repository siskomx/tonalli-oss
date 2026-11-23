import assert from 'node:assert/strict';
import {mkdir,writeFile} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import path from 'node:path';
import {root,startArticleHost} from './article-host.mjs';
import {destination,appWords} from './consumer-gui-flow.mjs';

const output=path.join(root,'target/cookbook-browser',randomUUID());await mkdir(output,{recursive:true});
let host;const errors=[];let progress='start';
try{
 host=await startArticleHost({bootstrap:'request',headless:true});const{page,origin}=host;page.on('pageerror',error=>errors.push(error.message));
 await page.goto(origin+'/cookbook',{waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready);const t=await appWords(page);
 assert(await page.getByText(t['recipe.empty'],{exact:true}).isVisible());
 await page.getByRole('button',{name:t['recipe.add'],exact:true}).click();const admin=page.getByTestId('admin-settings');
 await admin.getByLabel(t['admin.password'],{exact:true}).fill('Owned Cookbook fixture');await admin.getByLabel(t['admin.confirm'],{exact:true}).fill('Owned Cookbook fixture');
 await admin.getByRole('button',{name:t['admin.create'],exact:true}).click();
 const editor=page.getByTestId('recipe-editor'),source='>> title: Lentils\n\nMix @red lentils{200%g} and @water{1%l}.\n\nSimmer for ~{20%minutes}.\n\nServe. <script>window.recipeUnsafe=1</script>';
 await editor.getByLabel(t['recipe.name'],{exact:true}).fill('Lentils');await editor.getByLabel(t['recipe.source'],{exact:true}).fill(source);
 await editor.getByRole('button',{name:t['recipe.save'],exact:true}).click();
 const viewer=page.getByTestId('recipe-viewer'),ingredients=page.getByTestId('recipe-ingredients').getByRole('checkbox'),steps=page.getByTestId('recipe-steps').getByRole('checkbox');
 await ingredients.first().waitFor();assert.equal(await ingredients.count(),2);assert.equal(await steps.count(),3);assert.equal(await page.evaluate(()=>window.recipeUnsafe),undefined);
 const id=new URL(page.url()).searchParams.get('file');assert.match(id,/^[a-f0-9]{32}$/);progress='created';
 async function tick(locator){const response=page.waitForResponse(reply=>new URL(reply.url()).pathname==='/api/reading'&&reply.request().method()==='POST');await locator.check();assert.equal((await response).status(),200);}
 await tick(ingredients.first());await tick(steps.nth(1));
 await page.reload({waitUntil:'domcontentloaded'});await ingredients.first().waitFor();assert(await ingredients.first().isChecked());assert(await steps.nth(1).isChecked());
 await destination(page,'library');await destination(page,'cookbook');await page.getByRole('link',{name:'Lentils.cook',exact:true}).click();await ingredients.first().waitFor();assert(await ingredients.first().isChecked());progress='restored';
 await viewer.getByRole('button',{name:t['recipe.edit'],exact:true}).click();await admin.getByLabel(t['admin.password'],{exact:true}).fill('Owned Cookbook fixture');await admin.getByRole('button',{name:t['admin.unlock'],exact:true}).click();
 await editor.getByLabel(t['recipe.source'],{exact:true}).fill(source+'\n\nCool before storing.');await editor.getByRole('button',{name:t['recipe.save'],exact:true}).click();
 await page.waitForFunction(()=>document.querySelectorAll('[data-testid="recipe-steps"] input').length===4);assert(!await ingredients.first().isChecked());assert(!await steps.nth(1).isChecked());progress='edited';
 // A concurrent writer must retain its complete content when the old editor saves.
 await viewer.getByRole('button',{name:t['recipe.edit'],exact:true}).click();await admin.getByLabel(t['admin.password'],{exact:true}).fill('Owned Cookbook fixture');
 const loginReply=page.waitForResponse(response=>new URL(response.url()).pathname==='/api/admin/login');await admin.getByRole('button',{name:t['admin.unlock'],exact:true}).click();const{token}=await(await loginReply).json();
 await editor.getByLabel(t['recipe.source'],{exact:true}).waitFor({state:'visible'});
 const concurrent=await page.evaluate(async({id,token})=>{const{csrf}=await(await fetch('/api/session')).json(),h={'X-Tonalli-Csrf':csrf,'Content-Type':'application/json'};
 const value=await(await fetch(`/api/files/${id}/text`,{headers:h})).json();const text=value.text+'\n\nKeep cool.';
 const response=await fetch(`/api/files/${id}/text`,{method:'PUT',headers:{...h,'X-Tonalli-Admin':token},body:JSON.stringify({version:value.file.version,text})});if(!response.ok)throw Error('Owned concurrent edit failed');return text;},{id,token});
 await editor.getByRole('button',{name:t['recipe.save'],exact:true}).click();await editor.getByText(t['recipe.changed'],{exact:true}).waitFor();
 const retained=await page.evaluate(async id=>{const{csrf}=await(await fetch('/api/session')).json();return(await(await fetch(`/api/files/${id}/text`,{headers:{'X-Tonalli-Csrf':csrf}})).json()).text;},id);assert.equal(retained,concurrent);
 await editor.getByRole('button',{name:t['files.cancel'],exact:true}).click();
 await viewer.getByRole('button',{name:t['recipe.reset'],exact:true}).click();await page.reload({waitUntil:'domcontentloaded'});await ingredients.first().waitFor();assert(!await ingredients.first().isChecked());
 const safety=await page.evaluate(async()=>{const{csrf}=await(await fetch('/api/session')).json();const headers={'X-Tonalli-Csrf':csrf,'Content-Type':'application/octet-stream'};
 return{locked:(await fetch('/api/files/upload?name=unlocked.cook',{method:'POST',headers,body:'Serve.'})).status,testRoutes:(await fetch('/api/ping',{headers})).status};});assert.deepEqual(safety,{locked:423,testRoutes:404});
 await destination(page,'files');await admin.getByLabel(t['admin.password'],{exact:true}).fill('Owned Cookbook fixture');await admin.getByRole('button',{name:t['admin.unlock'],exact:true}).click();
 await page.locator('input[type="file"]:not([webkitdirectory])').first().setInputFiles(path.join(root,'web-stub/tests/fixtures/recipe-check.cook'));
 await page.getByTestId('managed-files').getByRole('button',{name:t['files.import'],exact:true}).click();await page.getByTestId('managed-file').filter({hasText:'recipe-check.cook'}).waitFor();
 await destination(page,'cookbook');await page.getByRole('link',{name:'recipe-check.cook',exact:true}).click();await ingredients.first().waitFor();assert.equal(await ingredients.count(),2);assert.equal(await steps.count(),3);progress='imported';
 await page.setViewportSize({width:500,height:850});assert(await ingredients.first().isVisible());assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
 await page.screenshot({path:path.join(output,'recipe.png')});assert.deepEqual(errors,[]);assert.deepEqual(host.blocked,[]);
 const result={label:'desktop browser integration, not native Windows or Android acceptance',create:true,edit:true,fileImport:true,concurrentEditRetained:true,privateChecklistsRestore:true,changedContentStartsNewChecklist:true,scriptsInert:true,narrowViewport:true,safety,errors,external:host.blocked,cli_sha256:host.cliSha256,ui_sha256:host.distSha256};
 await writeFile(path.join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({output,...result}));
}catch(error){console.log(JSON.stringify({output,progress}));throw error;}finally{await host?.close();}
