import assert from 'node:assert/strict';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {createHash,randomUUID} from 'node:crypto';
import path from 'node:path';
import {root,startArticleHost} from './article-host.mjs';
import {appWords} from './consumer-gui-flow.mjs';

const fixture=JSON.parse(await readFile(path.join(root,'crates/core/tests/fixtures/documents/documents-v1.json'),'utf8'));
const output=path.join(root,'target/embedded-documents',randomUUID());await mkdir(output,{recursive:true});
async function until(page,predicate,arg){const deadline=Date.now()+20000;while(Date.now()<deadline){if(await page.evaluate(predicate,arg))return;await new Promise(resolve=>setTimeout(resolve,100));}throw Error('Expected saved document position was not observed');}
let host;const errors=[],dialogs=[];
try{
  host=await startArticleHost({bootstrap:'request',headless:true,extraPacks:[{id:'documents',file:fixture.path,bytes:fixture.bytes,sha256:fixture.sha256}]});
  const{page,origin}=host;page.setDefaultTimeout(20000);page.on('pageerror',error=>errors.push(error.message));page.on('dialog',dialog=>{dialogs.push(dialog.type());void dialog.dismiss();});
  async function article(url='C/index.html'){await page.goto(`${origin}/article?${new URLSearchParams({pack:'documents',url})}`,{waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready);}
  await article();const t=await appWords(page),frame=page.frameLocator('#article-frame');
  assert.equal(await frame.locator('object,embed').count(),0);assert.equal(await frame.getByRole('link',{name:t['article.openDocument'],exact:true}).count(),2);
  await frame.getByRole('link',{name:'Read PDF',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.textLayer')?.textContent.includes('Tonalli PDF page 1'));
  assert.equal(new URL(page.url()).searchParams.get('url'),'C/docs/guide.pdf','An embedded document must have its own reloadable article address');
  await page.getByTestId('pdf-reader').getByRole('button',{name:t['document.next'],exact:true}).click();await page.waitForFunction(()=>document.querySelector('.textLayer')?.textContent.includes('Tonalli PDF page 2'));
  await until(page,async()=>{const {csrf}=await(await fetch('/api/session')).json();const {entries}=await(await fetch('/api/reading',{headers:{'X-Tonalli-Csrf':csrf}})).json();return entries.some(entry=>entry.destination.kind==='article'&&entry.destination.pack==='documents'&&entry.destination.url==='C/docs/guide.pdf'&&entry.position.kind==='page'&&entry.position.number===2);});
  await page.getByRole('button',{name:t['article.back'],exact:true}).click();await frame.getByRole('heading',{name:'Embedded documents',exact:true}).waitFor();await page.waitForFunction(()=>window.__ready?.ready);
  await frame.getByRole('link',{name:'PDF redirect',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.textLayer')?.textContent.includes('Tonalli PDF page 2'));
  await page.screenshot({path:path.join(output,'embedded-pdf.png')});
  await page.getByRole('button',{name:t['article.saveDocument'],exact:true}).click();
  const passphrase='Owned embedded browser fixture';await page.getByLabel(t['admin.password'],{exact:true}).fill(passphrase);await page.getByLabel(t['admin.confirm'],{exact:true}).fill(passphrase);
  await page.getByRole('button',{name:t['admin.create'],exact:true}).click();await page.getByRole('button',{name:t['article.copy'],exact:true}).click();await page.getByRole('status').filter({hasText:t['article.copied']}).waitFor();
  const copied=await page.evaluate(async()=>{const {csrf}=await(await fetch('/api/session')).json(),headers={'X-Tonalli-Csrf':csrf};const {files}=await(await fetch('/api/files',{headers})).json();const file=files.find(file=>file.name==='guide.pdf');if(!file)throw Error('Managed PDF copy missing');const response=await fetch(`/api/files/${file.id}/download`,{headers});if(!response.ok)throw Error('Managed download unavailable');return{file,bytes:[...new Uint8Array(await response.arrayBuffer())]};});
  assert.equal(createHash('sha256').update(Buffer.from(copied.bytes)).digest('hex'),fixture.entries.find(entry=>entry.url==='C/docs/guide.pdf').sha256);
  await page.getByRole('link',{name:t['gallery.file'],exact:true}).click();await page.getByTestId('file-viewer').getByRole('heading',{name:'guide.pdf',exact:true}).waitFor();
  await article();await frame.getByRole('link',{name:'Read EPUB',exact:true}).click();const bookFrame=page.frameLocator('[data-testid="epub-frame"]');await bookFrame.getByRole('heading',{name:'First chapter',exact:true}).waitFor();
  const book=page.getByTestId('epub-reader');await book.getByRole('button',{name:t['book.next'],exact:true}).click();await page.waitForFunction(()=>document.querySelector('[data-testid="epub-reader"] select')?.value==='1'&&window.__ready?.ready);
  await book.getByRole('button',{name:t['book.next'],exact:true}).click();await bookFrame.getByRole('heading',{name:'Second chapter',exact:true}).waitFor();
  await until(page,async()=>{const {csrf}=await(await fetch('/api/session')).json();const {entries}=await(await fetch('/api/reading',{headers:{'X-Tonalli-Csrf':csrf}})).json();return entries.some(entry=>entry.destination.url==='C/docs/book.epub'&&entry.position.kind==='epub'&&JSON.parse(entry.position.location).chapter===2);});
  await page.reload({waitUntil:'domcontentloaded'});await bookFrame.getByRole('heading',{name:'Second chapter',exact:true}).waitFor();await page.screenshot({path:path.join(output,'embedded-epub.png')});
  assert.equal(new URL(page.url()).searchParams.get('url'),'C/docs/book.epub');
  await page.getByRole('button',{name:t['article.back'],exact:true}).click();await frame.getByRole('heading',{name:'Embedded documents',exact:true}).waitFor();
  assert.equal(new URL(page.url()).searchParams.get('url'),'C/index.html','Back after reload must return to the source article');
  await article('C/docs/comic.cbz');await page.waitForFunction(()=>document.querySelector('[data-testid="comic-image"]')?.naturalWidth===160);await page.getByTestId('comic-reader').getByRole('button',{name:t['document.next'],exact:true}).click();await page.waitForFunction(()=>document.querySelector('[data-testid="comic-image"]')?.naturalWidth===1);
  await article();await frame.getByRole('link',{name:'HTML named PDF',exact:true}).click();await frame.getByRole('heading',{name:'This is HTML',exact:true}).waitFor();assert.equal(await page.getByTestId('embedded-document').count(),0);
  await article();await frame.getByRole('link',{name:'Redirect with anchor',exact:true}).click();await page.waitForFunction(()=>document.querySelector('#article-frame').contentWindow.location.hash==='#section'&&document.querySelector('#article-frame').contentWindow.scrollY>500&&window.__ready?.ready);
  assert.deepEqual(errors,[]);assert.deepEqual(dialogs,[]);assert.deepEqual(host.blocked,[]);
  const result={label:'desktop browser integration; native Save Copy picker and phone acceptance untested',sourcePackSha256:fixture.sha256,embeddedPlaceholdersReplaced:2,pdfRendered:true,pdfRedirectRestoresPage2:true,epubSpineAndProgressAfterReload:true,comicPagesDecoded:true,htmlNamedPdfRemainsHtml:true,redirectAnchorPreserved:true,managedCopySha256:copied.file.sha256,managedCopyBytes:copied.file.bytes,fileDetailsOpened:true,errors,dialogs,external:host.blocked,cli_sha256:host.cliSha256,ui_sha256:host.distSha256};
  await writeFile(path.join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({output,...result}));
}catch(error){await writeFile(path.join(output,'failure.json'),JSON.stringify({message:String(error.message).slice(0,2000),errors,dialogs},null,2));throw error;}
finally{await host?.close();}
