// Production UI with a controlled HTTP fixture; native/protocol acceptance is separate.
import assert from 'node:assert/strict';
import test from 'node:test';
import {createServer} from 'node:http';
import {readFile,mkdir} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {resolve,sep,extname} from 'node:path';
import {chromium} from '../../node_modules/playwright-core/index.mjs';
const dist=resolve(fileURLToPath(new URL('../../dist/',import.meta.url)));
const files=[{id:'a'.repeat(32),name:'Selected.wav',folder:'Fixture',bytes:44},{id:'b'.repeat(32),name:'Hidden.mp4',folder:'Fixture',bytes:100},{id:'c'.repeat(32),name:'Private.txt',folder:'Fixture',bytes:10}];
let fixture,browser,server,origin;const commands=[];
test.before(async()=>{
 server=createServer(async(req,res)=>{
  const path=new URL(req.url,'http://localhost').pathname;
  const json=(value,status=200)=>{res.writeHead(status,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(JSON.stringify(value));};
  if(path==='/api/session')return json({csrf:'tv-fixture-csrf',private_notes:true,private_tv:fixture.capability,private_content:false,private_ai:false});
  if(path==='/api/preferences')return json({theme:'light',show_maps:true,show_notes:true});
  if(path==='/api/admin')return json({configured:false});
  if(path==='/api/admin/lock')return json({});
  if(path==='/api/files')return json({files});
  if(path==='/api/music')return json({state:{revision:0,queue:{tracks:[],current:null,generation:0,seconds:0}},native:false,playlists:[]});
  if(path==='/api/sharing')return json({status:{running:false,busy:false,networks:[{id:'owned',label:'Fixture Wi-Fi'}],devices:[]},packs:[],selected_media:[]});
  if(path==='/api/tv')return json({status:{running:fixture.running,busy:false,url:fixture.running?'http://127.0.0.1:38400/description.xml':null,networks:[{id:'owned',label:'Fixture Wi-Fi'}]},selected_media:fixture.running?files.filter(f=>fixture.selected.includes(f.id)):[]});
  if(path.startsWith('/api/tv/')){
   assert.equal(req.headers['x-tonalli-csrf'],'tv-fixture-csrf');let body='';for await(const chunk of req)body+=chunk;const value=JSON.parse(body);commands.push({path,body:value});
   if(path.endsWith('/start')){assert.deepEqual(value,{network:'owned',media:[files[0].id]});if(fixture.fail){fixture.fail=false;return json({error:'tv_unavailable'},409);}fixture.running=true;fixture.selected=value.media;}
   else if(path.endsWith('/stop'))fixture.running=false;
   return json({accepted:true});
  }
  if(path.startsWith('/api/'))return json({error:'fixture_unavailable'},404);
  let file=resolve(dist,'.'+decodeURIComponent(path));if(!file.startsWith(dist+sep))file=resolve(dist,'index.html');
  try {const raw=await readFile(extname(file)?file:resolve(dist,'index.html'));res.writeHead(200,{'Content-Type':({'.js':'text/javascript','.css':'text/css','.json':'application/json','.svg':'image/svg+xml'})[extname(file)]||'text/html'});res.end(raw);}
  catch{res.writeHead(404);res.end();}
 });await new Promise(r=>server.listen(0,'127.0.0.1',r));origin=`http://127.0.0.1:${server.address().port}`;
 browser=await chromium.launch({channel:'msedge',headless:true});
});
test.after(async()=>{await browser?.close();await new Promise(r=>server?.close(r));});
for(const [locale,width] of [['en-US',1100],['es-MX',390]])test(`TV settings explicit selection, warning, retry and stop ${locale}`,async()=>{
 fixture={capability:true,running:false,selected:[],fail:true};commands.length=0;
 const t=JSON.parse(await readFile(new URL(`../../src/locales/${locale.startsWith('es')?'es':'en'}.json`,import.meta.url),'utf8'));
 const context=await browser.newContext({locale,viewport:{width,height:900}});const external=[],errors=[];
 await context.route('**/*',async route=>{if(new URL(route.request().url()).origin!==origin){external.push(route.request().url());await route.abort();}else await route.continue();});
 const page=await context.newPage();page.on('pageerror',e=>errors.push(e.message));
 try{
  await page.goto(origin+'/settings');await page.getByRole('tab',{name:t['settings.tab.sharing'],exact:true}).click();const panel=page.getByTestId('tv-sharing');await panel.getByText(t['tv.off'],{exact:true}).waitFor();assert.equal(commands.length,0);assert(await panel.getByTestId('tv-warning').isVisible());
  const start=panel.getByRole('button',{name:t['tv.start'],exact:true});assert(await start.isDisabled());assert.equal(await panel.locator(`[data-tv-media-id="${files[2].id}"]`).count(),0);
  await panel.locator(`[data-tv-media-id="${files[0].id}"]`).check();await start.click();await panel.getByRole('alert').waitFor();assert(!fixture.running);
  await start.click();await panel.getByText(t['tv.on'],{exact:true}).waitFor();assert.equal(await panel.getByText('Selected.wav',{exact:true}).count(),1);assert.equal(await panel.getByText('Hidden.mp4',{exact:true}).count(),0);assert(await panel.getByText(t['tv.snapshotHelp'],{exact:true}).isVisible());
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));await mkdir(new URL('../../../target/tv-ui-r1/',import.meta.url),{recursive:true});await panel.screenshot({path:fileURLToPath(new URL(`../../../target/tv-ui-r1/${locale}.png`,import.meta.url))});
  await panel.getByRole('button',{name:t['tv.stop'],exact:true}).click();await panel.getByText(t['tv.off'],{exact:true}).waitFor();assert(!fixture.running);assert.equal(commands.filter(c=>c.path.endsWith('/start')).length,2);assert.equal(commands.filter(c=>c.path.endsWith('/stop')).length,1);assert.deepEqual(external,[]);assert.deepEqual(errors,[]);
 }finally{await context.close();}
});
test('TV controls are absent without the private capability',async()=>{
 fixture={capability:false,running:false,selected:[]};const context=await browser.newContext();try{const page=await context.newPage();await page.goto(origin+'/settings');await page.waitForFunction(()=>window.__ready?.ready);assert.equal(await page.getByTestId('tv-sharing').count(),0);}finally{await context.close();}
});
