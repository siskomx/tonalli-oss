import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {createInterface} from 'node:readline';
import {mkdir,writeFile} from 'node:fs/promises';
import {randomUUID,createHash} from 'node:crypto';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright-core';
import {destination,appWords} from './consumer-gui-flow.mjs';

const root=fileURLToPath(new URL('../../../',import.meta.url));
function pdfFixture(){
  const objects=['<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /JavaScript /JS (app.alert\\(\'document script must stay inactive\'\\)) >> >>',
    '<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>',null,
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>',null,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'];
  for(const [index,number] of [[3,1],[5,2]]){const stream=`BT /F1 20 Tf 30 230 Td (Tonalli PDF page ${number}) Tj ET\n`;objects[index]=`<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}endstream`;}
  let body='%PDF-1.7\n',offsets=[0];for(const [index,object] of objects.entries()){offsets.push(Buffer.byteLength(body));body+=`${index+1} 0 obj\n${object}\nendobj\n`;}
  const xref=Buffer.byteLength(body);body+=`xref\n0 ${objects.length+1}\n0000000000 65535 f \n`+offsets.slice(1).map(offset=>`${String(offset).padStart(10,'0')} 00000 n \n`).join('');
  body+=`trailer\n<< /Size ${objects.length+1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;return Buffer.from(body);
}
const output=path.join(root,'target/documents-browser',randomUUID());await mkdir(output,{recursive:true});
const bytes=pdfFixture(),invalid=Buffer.from('%PDF-1.7\nnot a valid document\n');
await writeFile(path.join(output,'Document-check.pdf'),bytes);await writeFile(path.join(output,'Invalid-document.pdf'),invalid);
let child,browser,context,page;const errors=[],external=[],dialogs=[];
async function start(){
 child=spawn(path.join(root,'target/debug/spike-cli.exe'),['serve','--web',path.join(root,'web-stub/dist'),'--data-dir',path.join(output,'data')],{windowsHide:true,stdio:['ignore','pipe','pipe']});
 const launch=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Production host startup timed out')),30000);const reader=createInterface({input:child.stdout});reader.once('line',line=>{clearTimeout(timer);reader.close();resolve(line);});child.once('error',error=>{clearTimeout(timer);reject(error);});child.once('exit',code=>{clearTimeout(timer);reject(Error(`Production host exited ${code}`));});});
 assert.equal(new URL(launch).hostname,'127.0.0.1');
 context=await browser.newContext({locale:'en-US',viewport:{width:1100,height:850}});page=await context.newPage();page.setDefaultTimeout(20000);
 const origin=new URL(launch).origin;
 page.on('pageerror',error=>errors.push(error.message));page.on('dialog',dialog=>{dialogs.push(dialog.type());void dialog.dismiss();});
 page.on('request',request=>{if(/^https?:/.test(request.url())&&new URL(request.url()).origin!==origin)external.push(request.url());});
 await page.goto(launch,{waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready);
}
async function stop(){await context?.close();context=null;if(child&&child.exitCode===null&&child.signalCode===null){const exited=once(child,'exit');child.kill();await exited;}child=null;}
try{
 browser=await chromium.launch({executablePath:'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',headless:true});await start();const words=await appWords(page);
 const files=await page.evaluate(async({pdf,bad})=>{
  const {csrf}=await(await fetch('/api/session')).json(),headers={'X-Tonalli-Csrf':csrf,'Content-Type':'application/json'};
  const response=await fetch('/api/admin/setup',{method:'POST',headers,body:JSON.stringify({passphrase:'Owned document browser test'})});if(!response.ok)throw Error('Owned Admin fixture setup failed');
  const {token}=await response.json();
  for(const [name,data] of [['Document-check.pdf',pdf],['Invalid-document.pdf',bad]]){
   const upload=await fetch(`/api/files/upload?${new URLSearchParams({name,folder:'Document-check'})}`,{method:'POST',headers:{'X-Tonalli-Csrf':csrf,'X-Tonalli-Admin':token,'Content-Type':'application/octet-stream'},body:Uint8Array.from(data)});if(!upload.ok)throw Error('Owned document import failed');
  }
  if((await fetch('/api/ping',{headers})).status!==404)throw Error('Use a production host without test endpoints');
  return(await(await fetch('/api/files',{headers})).json()).files;
 },{pdf:[...bytes],bad:[...invalid]});
 const selected=files.find(file=>file.name==='Document-check.pdf');assert.equal(selected.sha256,createHash('sha256').update(bytes).digest('hex'));
 async function open(name){await destination(page,'reader');await page.locator('#document-search').fill(name);await page.getByTestId('document-page').getByRole('link',{name,exact:true}).click();}
 async function rendered(number){await page.waitForFunction(number=>document.querySelector('.textLayer')?.textContent.includes(`Tonalli PDF page ${number}`),number);assert.equal(await page.getByTestId('pdf-reader').getByRole('alert').count(),0);}
 await open('Document-check.pdf');await rendered(1);
 const canvas=await page.getByTestId('pdf-area').locator('canvas').evaluate(canvas=>{const pixels=canvas.getContext('2d').getImageData(0,0,canvas.width,canvas.height).data;let dark=0;for(let i=0;i<pixels.length;i+=4)if(pixels[i]<180&&pixels[i+3])dark++;return{width:canvas.width,height:canvas.height,dark};});assert(canvas.dark>100);
 await page.screenshot({path:path.join(output,'pdf-page1.png')});
 await page.getByTestId('pdf-reader').getByRole('button',{name:words['document.next'],exact:true}).click();await rendered(2);
 const deadline=Date.now()+20000;let saved=false;
 while(!saved&&Date.now()<deadline){saved=await page.evaluate(async id=>{const {csrf}=await(await fetch('/api/session')).json();const history=await(await fetch('/api/reading',{headers:{'X-Tonalli-Csrf':csrf}})).json();return history.entries.some(entry=>entry.destination.id===id&&entry.position.kind==='page'&&entry.position.number===2);},selected.id);if(!saved)await new Promise(resolve=>setTimeout(resolve,100));}assert(saved,'Page two progress must reach disk before restarting');
 const beforeZoom=await page.getByTestId('pdf-area').locator('canvas').evaluate(canvas=>parseFloat(canvas.style.width));
 await page.getByTestId('pdf-reader').getByLabel('Zoom',{exact:true}).selectOption('1.5');
 await page.waitForFunction(before=>parseFloat(document.querySelector('[data-testid="pdf-area"] canvas')?.style.width)>before*1.4&&document.querySelector('.textLayer')?.textContent.includes('Tonalli PDF page 2'),beforeZoom);await rendered(2);
 await stop();await start();await open('Document-check.pdf');await rendered(2);assert.equal(await page.getByLabel('Page',{exact:true}).inputValue(),'2');
 await open('Invalid-document.pdf');await page.getByTestId('pdf-reader').getByRole('alert').waitFor();
 assert.deepEqual(errors,[]);assert.deepEqual(external,[]);assert.deepEqual(dialogs,[]);
 const report={label:'desktop browser integration; not native app or phone acceptance',productionHost:true,documentSha256:selected.sha256,canvas,textSelectable:true,pageNavigation:true,fitAndZoom:true,pageProgressAfterProcessRestart:true,corruptDocumentUsefulError:true,pdfJavaScriptNotExecuted:true,errors,external,dialogs};
 await writeFile(path.join(output,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({output,...report}));
}finally{await stop();await browser?.close();}
