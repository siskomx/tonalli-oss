import assert from 'node:assert/strict';
import {mkdir,writeFile} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import path from 'node:path';
import {root,startArticleHost} from './article-host.mjs';
import {appWords} from './consumer-gui-flow.mjs';

const output=path.join(root,'target/article-readable',randomUUID());await mkdir(output,{recursive:true});
let host;
try{
  host=await startArticleHost({bootstrap:'request',headless:true});const{page,origin}=host;const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto(`${origin}/article?${new URLSearchParams({pack:'benign',url:'C/en/index.html'})}`,{waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready);
  const t=await appWords(page),frame=page.frameLocator('#article-frame');
  const before=await frame.locator('body').evaluate(body=>parseFloat(getComputedStyle(body).fontSize)),headingBefore=await page.locator('#article-title').evaluate(node=>getComputedStyle(node).fontSize);
  await page.getByLabel(t['article.textSize'],{exact:true}).selectOption('150');
  const after=await frame.locator('body').evaluate(body=>parseFloat(getComputedStyle(body).fontSize));assert(after>=before*1.49);
  assert.equal(await page.locator('#article-title').evaluate(node=>getComputedStyle(node).fontSize),headingBefore);
  assert.equal(await page.locator('#article-frame').getAttribute('sandbox'),'allow-same-origin');
  await page.setViewportSize({width:500,height:850});
  const narrow=await page.locator('#article-frame').boundingBox();assert(narrow.width<500&&Math.abs(narrow.height-850*.75)<3);
  const overflow=await frame.locator('body').evaluate(body=>body.scrollWidth>body.ownerDocument.documentElement.clientWidth+2);assert.equal(overflow,false);
  await page.screenshot({path:path.join(output,'article-text-150.png')});
  assert.deepEqual(errors,[]);assert.deepEqual(host.blocked,[]);
  const result={label:'desktop browser integration; Android pinch gesture untested',textBefore:before,textAfter:after,parentTextUnchanged:true,narrowFrame:narrow,noHorizontalBodyOverflow:true,sandbox:'allow-same-origin',errors,external:host.blocked,cli_sha256:host.cliSha256,ui_sha256:host.distSha256};
  await writeFile(path.join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({output,...result}));
}finally{await host?.close();}
