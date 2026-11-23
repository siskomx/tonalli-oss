import assert from 'node:assert/strict';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {randomUUID,createHash} from 'node:crypto';
import path from 'node:path';
import {root,startArticleHost} from './article-host.mjs';
import {destination,appWords} from './consumer-gui-flow.mjs';

function crc32(bytes){let crc=-1;for(const byte of bytes){crc^=byte;for(let bit=0;bit<8;bit++)crc=(crc>>>1)^(-(crc&1)&0xedb88320);}return(crc^-1)>>>0;}
assert.equal(crc32(Buffer.from('123456789')),0xcbf43926);
function storedZip(rows){
  const chunks=[],directory=[];let offset=0;
  for(const [name,value] of rows){
    const file=Buffer.from(name),data=Buffer.isBuffer(value)?value:Buffer.from(value),crc=crc32(data),header=Buffer.alloc(30),entry=Buffer.alloc(46);
    header.writeUInt32LE(0x04034b50);header.writeUInt16LE(20,4);header.writeUInt16LE(0x800,6);header.writeUInt16LE(0x21,12);header.writeUInt32LE(crc,14);header.writeUInt32LE(data.length,18);header.writeUInt32LE(data.length,22);header.writeUInt16LE(file.length,26);
    entry.writeUInt32LE(0x02014b50);entry.writeUInt16LE(0x314,4);entry.writeUInt16LE(20,6);entry.writeUInt16LE(0x800,8);entry.writeUInt16LE(0x21,14);entry.writeUInt32LE(crc,16);entry.writeUInt32LE(data.length,20);entry.writeUInt32LE(data.length,24);entry.writeUInt16LE(file.length,28);entry.writeUInt32LE((0o100644<<16)>>>0,38);entry.writeUInt32LE(offset,42);
    chunks.push(header,file,data);directory.push(entry,file);offset+=header.length+file.length+data.length;
  }
  const table=Buffer.concat(directory),footer=Buffer.alloc(22);footer.writeUInt32LE(0x06054b50);footer.writeUInt16LE(rows.length,8);footer.writeUInt16LE(rows.length,10);footer.writeUInt32LE(table.length,12);footer.writeUInt32LE(offset,16);
  return Buffer.concat([...chunks,table,footer]);
}
const output=path.join(root,'target/documents-books',randomUUID());await mkdir(output,{recursive:true});
async function waitForAsync(page,predicate,arg){const deadline=Date.now()+20000;while(Date.now()<deadline){if(await page.evaluate(predicate,arg))return;await new Promise(resolve=>setTimeout(resolve,100));}throw Error('Saved reading position was not observed');}
const gallery=JSON.parse(await readFile(path.join(root,'target/gallery-fixtures-42bf/manifest.json'),'utf8'));
async function image(suffix){const record=gallery.files.find(file=>file.name.endsWith(suffix));assert(record);const bytes=await readFile(record.path);assert.equal(createHash('sha256').update(bytes).digest('hex'),record.sha256);return bytes;}
const png=await image('-A.png'),gif=await image('-B.gif');
const chapter=title=>`<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="../styles.css"><title>${title}</title></head><body><h1>${title}</h1><p><a href="#p10">Jump forward</a> · <a href="two.xhtml">Next inside book</a> · <a href="https://example.invalid/never-open">Outside link</a></p><script>parent.bookUnsafe=1</script>${Array.from({length:150},(_,i)=>`<p id="p${i}">Reading paragraph ${i}. Niñez y educación. An offline book remains readable when its text reflows across different window widths. The saved place belongs to this device.</p>`).join('')}</body></html>`;
const epub=storedZip([
 ['mimetype','application/epub+zip'],['META-INF/container.xml','<container><rootfiles><rootfile full-path="OPS/package.opf"/></rootfiles></container>'],
 ['OPS/package.opf','<package><manifest><item id="one" href="one.xhtml" media-type="application/xhtml+xml"/><item id="two" href="two.xhtml" media-type="application/xhtml+xml"/><item id="css" href="../styles.css" media-type="text/css"/></manifest><spine><itemref idref="one"/><itemref idref="one"/><itemref idref="two"/></spine></package>'],
 ['OPS/one.xhtml',chapter('First chapter')],['OPS/two.xhtml',chapter('Second chapter')],['styles.css','body{font:20px/1.5 serif;margin:12px}p{margin:0 0 14px}'],
]);
const comic=storedZip([10,2,1,...Array.from({length:17},(_,i)=>i+3).filter(i=>i!==10),20].map(number=>[`pages/page${String(number).padStart(2,'0')}.${number===2?'gif':'png'}`,number===2?gif:png]));
const broken=storedZip([['bad.png','not an image']]);
const svgBook=storedZip([
 ['mimetype','application/epub+zip'],['META-INF/container.xml','<container><rootfiles><rootfile full-path="package.opf"/></rootfiles></container>'],
 ['package.opf','<package><manifest><item id="cover" href="cover.svg" media-type="image/svg+xml"/></manifest><spine><itemref idref="cover"/></spine></package>'],
 ['cover.svg','<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 600"><rect width="400" height="600" fill="#f5eedc"/><text x="20" y="60" font-size="30">SVG book cover</text><script>parent.bookUnsafe=1</script></svg>'],
]);
const fixtures=[['Book-check.epub',epub],['Comic-check.cbz',comic],['Broken-book.cbz',broken],['SVG-check.epub',svgBook]];
const manifest=[];for(const [name,bytes] of fixtures){await writeFile(path.join(output,name),bytes);manifest.push({name,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')});}await writeFile(path.join(output,'manifest.json'),JSON.stringify(manifest,null,2));
let host;const errors=[],dialogs=[],progress=[];
try{
 host=await startArticleHost({bootstrap:'request',headless:true});const{page,origin}=host;page.setDefaultTimeout(20000);page.on('pageerror',error=>errors.push(error.message));page.on('dialog',dialog=>{dialogs.push(dialog.type());void dialog.dismiss();});
 page.on('request',request=>{if(new URL(request.url()).pathname==='/api/reading'&&request.method()==='POST')progress.push(request.postDataJSON().position);});
 await page.goto(origin+'/reader',{waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready);const t=await appWords(page);
 const files=await page.evaluate(async fixtures=>{
  const {csrf}=await(await fetch('/api/session')).json(),headers={'X-Tonalli-Csrf':csrf,'Content-Type':'application/json'};
  const created=await fetch('/api/admin/setup',{method:'POST',headers,body:JSON.stringify({passphrase:'Owned book browser test'})});if(!created.ok)throw Error('Owned Admin fixture setup failed');const {token}=await created.json();
  for(const file of fixtures){const reply=await fetch(`/api/files/upload?${new URLSearchParams({name:file.name,folder:'Book-check'})}`,{method:'POST',headers:{'X-Tonalli-Csrf':csrf,'X-Tonalli-Admin':token,'Content-Type':'application/octet-stream'},body:Uint8Array.from(file.bytes)});if(!reply.ok)throw Error('Owned book import failed');}
  if((await fetch('/api/ping',{headers})).status!==404)throw Error('Use a production host without test endpoints');
  return(await(await fetch('/api/files',{headers})).json()).files;
 },fixtures.map(([name,bytes])=>({name,bytes:[...bytes]})));
 await page.reload({waitUntil:'domcontentloaded'});await page.waitForFunction(()=>window.__ready?.ready);
 async function open(name){await destination(page,'reader');await page.locator('#document-search').fill(name);await page.getByTestId('document-page').getByRole('link',{name,exact:true}).click();}
 const book=files.find(file=>file.name==='Book-check.epub'),comicFile=files.find(file=>file.name==='Comic-check.cbz');assert(book&&comicFile);
 await open('Book-check.epub');const frame=page.frameLocator('[data-testid="epub-frame"]');await frame.getByRole('heading',{name:'First chapter',exact:true}).waitFor();await page.waitForFunction(()=>window.__ready?.ready);
 assert.equal(await page.evaluate(()=>window.bookUnsafe),undefined);
 await frame.getByRole('link',{name:'Outside link',exact:true}).click();await page.getByRole('status').filter({hasText:t['article.offline']}).waitFor();assert.equal(new URL(page.url()).pathname,'/reader');
 await frame.getByRole('link',{name:'Jump forward',exact:true}).click();await page.waitForFunction(()=>document.querySelector('[data-testid="epub-frame"]').contentWindow.location.hash==='#p10');
 const epubView=page.getByTestId('epub-reader');await epubView.getByRole('button',{name:t['book.next'],exact:true}).click();await page.waitForFunction(()=>document.querySelector('[data-testid="epub-reader"] select')?.value==='1'&&window.__ready?.ready);
 await epubView.getByRole('button',{name:t['book.next'],exact:true}).click();await frame.getByRole('heading',{name:'Second chapter',exact:true}).waitFor();await page.waitForFunction(()=>window.__ready?.ready);
 const box=await page.getByTestId('epub-frame').boundingBox();assert(box);await page.mouse.move(box.x+box.width/2,box.y+100);await page.mouse.wheel(0,1400);
 await waitForAsync(page,async id=>{const {csrf}=await(await fetch('/api/session')).json();const {entries}=await(await fetch('/api/reading',{headers:{'X-Tonalli-Csrf':csrf}})).json();const position=entries.find(entry=>entry.destination.id===id)?.position;if(position?.kind!=='epub')return false;const saved=JSON.parse(position.location);return saved.chapter===2&&saved.y>500&&saved.node?.length>0;},book.id);
 const locator=await page.evaluate(async id=>{const {csrf}=await(await fetch('/api/session')).json();const {entries}=await(await fetch('/api/reading',{headers:{'X-Tonalli-Csrf':csrf}})).json();return JSON.parse(entries.find(entry=>entry.destination.id===id).position.location);},book.id);
 const beforeReflow=await page.evaluate(value=>{const iframe=document.querySelector('[data-testid="epub-frame"]'),doc=iframe.contentDocument;let node=doc.body;for(const index of value.node)node=node.childNodes[index];const range=doc.createRange();range.setStart(node,value.offset);range.collapse(true);return{top:range.getBoundingClientRect().top,scroll:iframe.contentWindow.scrollY,frameTop:iframe.getBoundingClientRect().top};},locator);assert(Math.abs(beforeReflow.top-8)<50,JSON.stringify({beforeReflow,locator}));
 await destination(page,'library');await page.setViewportSize({width:500,height:850});await open('Book-check.epub');await page.waitForFunction(()=>window.__ready?.ready);assert.equal(await epubView.getByLabel(t['book.chapterLabel'],{exact:true}).inputValue(),'2');
 const reflow=await page.evaluate(value=>{const iframe=document.querySelector('[data-testid="epub-frame"]'),doc=iframe.contentDocument;let node=doc.body;for(const index of value.node)node=node.childNodes[index];const range=doc.createRange();range.setStart(node,value.offset);range.collapse(true);return{top:range.getBoundingClientRect().top,scroll:iframe.contentWindow.scrollY};},locator);assert(Math.abs(reflow.top-8)<4,JSON.stringify({reflow,locator}));assert(reflow.scroll>0);
 await page.screenshot({path:path.join(output,'epub-reflow.png')});
 await open('Comic-check.cbz');const comicView=page.getByTestId('comic-reader');await page.waitForFunction(()=>document.querySelector('[data-testid="comic-image"]')?.naturalWidth===160);
 await comicView.getByRole('button',{name:t['document.next'],exact:true}).click();await page.waitForFunction(()=>document.querySelector('[data-testid="comic-image"]')?.naturalWidth===1);
 await comicView.getByLabel(t['book.mode'],{exact:true}).selectOption('scroll');await page.waitForFunction(()=>document.querySelector('[data-testid="comic-reader"] input')?.value==='2'&&window.__ready?.ready);
 const scroll=page.getByTestId('webtoon-scroll');await scroll.scrollIntoViewIfNeeded();const scrollBox=await scroll.boundingBox();assert(scrollBox);await page.mouse.move(scrollBox.x+scrollBox.width/2,scrollBox.y+100);await page.mouse.wheel(0,150);
 await waitForAsync(page,async id=>{const {csrf}=await(await fetch('/api/session')).json();const {entries}=await(await fetch('/api/reading',{headers:{'X-Tonalli-Csrf':csrf}})).json();const position=entries.find(entry=>entry.destination.id===id)?.position;if(position?.kind!=='epub')return false;const value=JSON.parse(position.location);return value.reader==='comic'&&value.page===2&&value.ratio>.2;},comicFile.id);
 const images=await scroll.locator('img').count();assert(images<=6,'Only nearby comic pages should be mounted');
 const comicPosition=await page.evaluate(async id=>{const{csrf}=await(await fetch('/api/session')).json();const{entries}=await(await fetch('/api/reading',{headers:{'X-Tonalli-Csrf':csrf}})).json();return JSON.parse(entries.find(entry=>entry.destination.id===id).position.location);},comicFile.id);
 await page.reload({waitUntil:'domcontentloaded'});await page.waitForFunction(()=>document.querySelector('[data-testid="comic-reader"] input')?.value==='2'&&window.__ready?.ready);assert.equal(await comicView.getByLabel(t['book.mode'],{exact:true}).inputValue(),'scroll');
 const ratio=await scroll.evaluate(element=>{const node=[...element.querySelectorAll('[data-index]')].find(node=>node.dataset.index==='1');return node?(element.scrollTop-parseFloat(node.style.transform.slice(11)))/node.getBoundingClientRect().height:null;});assert(ratio>.2&&ratio<.9);
 assert(Math.abs(ratio-comicPosition.ratio)<.02,JSON.stringify({expected:comicPosition.ratio,actual:ratio}));
 await page.screenshot({path:path.join(output,'comic-webtoon.png')});await open('Broken-book.cbz');await page.getByTestId('comic-reader').getByRole('alert').waitFor();
 await open('SVG-check.epub');await frame.getByText('SVG book cover',{exact:true}).waitFor();await page.waitForFunction(()=>window.__ready?.ready);assert.equal(await page.getByTestId('book-reader').getByRole('alert').count(),0);assert.equal(await page.evaluate(()=>window.bookUnsafe),undefined);
 assert.deepEqual(errors,[]);assert.deepEqual(dialogs,[]);assert.deepEqual(host.blocked,[]);
 const result={label:'desktop browser integration including narrow viewport; not native app or phone acceptance',productionHost:true,epubRendered:true,epubScriptsInactive:true,externalLinksBlocked:true,fragmentsWork:true,repeatedSpineEntriesWork:true,epubLogicalLocationAfterReflow:reflow,comicImagesDecoded:true,naturalPageOrder:true,webtoonMountedImages:images,webtoonModeAndOffsetAfterReload:ratio,corruptImageUsefulError:true,errors,dialogs,external:host.blocked,cli_sha256:host.cliSha256,ui_sha256:host.distSha256};await writeFile(path.join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({output,...result}));
}catch(error){console.log(JSON.stringify({output,progress}));await writeFile(path.join(output,'failure.json'),JSON.stringify({message:String(error.message).slice(0,1500),errors,dialogs,progress},null,2));throw error;}
finally{await host?.close();}
