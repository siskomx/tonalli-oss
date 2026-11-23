import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdir,readFile,access} from 'node:fs/promises';
import path from 'node:path';
import {destination,appWords} from './consumer-gui-flow.mjs';

// Run only against explicitly selected synthetic fixtures, after normal TLS trust/pairing.
export async function sharedMediaReader(page,host,{downloadDirectory}={}){
  assert(host.files.length>0&&host.files.length<=128);
  const expected=host.files,requests=[];page.on('request',request=>requests.push(new URL(request.url()).pathname));
  if(await page.title()==='Tonalli pairing'){
    await page.getByLabel('Pairing code',{exact:true}).fill(host.code);
    await page.getByLabel('Device name',{exact:true}).fill('Checked media reader');
    await page.getByRole('button',{name:'Pair',exact:true}).click();
  }
  await page.waitForFunction(()=>window.__ready?.ready,null,{timeout:60000});const t=await appWords(page);
  assert.equal(await page.getByTestId('nav-notes').count(),0);assert.equal(await page.getByTestId('nav-files').count(),0);assert.equal(await page.getByTestId('nav-gallery').count(),0);
  await destination(page,'shared');const shared=page.getByTestId('shared-files-page');
  await shared.locator('#shared-search').fill('');
  const records=await page.evaluate(async()=>{
    const session=await(await fetch('/api/session')).json();if(session.private_notes||session.private_content||!session.shared_media)throw Error('Wrong sharing capabilities');
    const reply=await fetch('/api/shared-media',{headers:{'X-Tonalli-Csrf':session.csrf}});if(!reply.ok)throw Error('Shared catalog unavailable');return(await reply.json()).files;
  });
  assert.deepEqual(records.map(row=>row.file.id).sort(),expected.map(file=>file.id).sort());
  assert.deepEqual(records.map(row=>row.file.sha256).sort(),expected.map(file=>file.sha256).sort());
  assert.equal(await shared.getByRole('link').count(),expected.length);
  const find=suffix=>{const file=expected.find(file=>file.name.endsWith(suffix));assert(file,suffix);return file;};
  async function open(file){await shared.locator('#shared-search').fill(file.name);await shared.getByRole('link',{name:file.name,exact:true}).click();await page.getByTestId('file-viewer').getByRole('heading',{name:file.name,exact:true}).first().waitFor();}
  const image=find('-A.png');await open(image);await page.waitForFunction(()=>document.querySelector('[data-testid="file-viewer"] img')?.naturalWidth===160);
  let actualDownload=false;
  if(downloadDirectory){
    await mkdir(downloadDirectory,{recursive:true});const output=path.join(downloadDirectory,image.name);
    await assert.rejects(access(output),'Do not overwrite an earlier download proof');
    const [download]=await Promise.all([page.waitForEvent('download'),page.getByTestId('file-viewer').getByRole('link',{name:t['files.download'],exact:true}).click()]);
    assert.equal(download.suggestedFilename(),image.name);await download.saveAs(output);
    assert.equal(createHash('sha256').update(await readFile(output)).digest('hex'),image.sha256);actualDownload=true;
  }
  const text=find('-Text.txt');await open(text);const preview=page.getByTestId('file-preview-text');await preview.waitFor();
  assert((await preview.innerText()).includes('<script>window.galleryUnsafe=1</script>'));assert.equal(await preview.locator('script').count(),0);assert.equal(await page.evaluate(()=>window.galleryUnsafe),undefined);
  const html=find('-Download.html');await open(html);assert.equal(await page.getByTestId('file-preview-text').count(),0);assert.equal(page.frames().length,1);assert.equal(await page.evaluate(()=>window.galleryUnsafe),undefined);
  const movie=find('-Movie.mp4');await open(movie);await page.waitForFunction(()=>document.querySelector('video')?.videoWidth===160);
  const player=page.getByTestId('video-player');assert(await page.locator('video').evaluate(video=>video.paused));
  await player.getByRole('button',{name:'Play',exact:true}).first().click();await page.waitForFunction(()=>{const video=document.querySelector('video');return !video.paused&&video.currentTime>=2;});
  const video=await page.locator('video').evaluate(video=>({width:video.videoWidth,height:video.videoHeight,seconds:video.currentTime,frames:video.getVideoPlaybackQuality?.().totalVideoFrames??video.webkitDecodedFrameCount}));assert(video.frames>0);
  const touch=await page.evaluate(()=>/Android/.test(navigator.userAgent));
  if(touch)await player.locator('.plyr').click({position:{x:40,y:40}});
  if(!await page.locator('video').evaluate(video=>video.paused))await player.locator('.plyr__controls [data-plyr="play"]').click();
  const seek=player.locator('[data-plyr="seek"]');await seek.scrollIntoViewIfNeeded();const box=await seek.boundingBox();assert(box&&box.width>35);
  if(touch){const session=await page.context().newCDPSession(page);await session.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:box.x+box.width*.7,y:box.y+box.height/2}]});await session.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});await session.detach();}
  else await seek.click({position:{x:box.width*.7,y:box.height/2}});
  await page.waitForFunction(()=>{const video=document.querySelector('video');return video.paused&&!video.seeking&&Math.abs(video.currentTime-63)<10;});const seekSeconds=await page.locator('video').evaluate(video=>video.currentTime);
  const music=find('-Music.wav');await open(music);await page.waitForFunction(()=>document.querySelector('audio[controls]')?.duration===10);
  const audio=page.locator('audio[controls]');await audio.click({position:{x:18,y:25}});await page.waitForFunction(()=>{const audio=document.querySelector('audio[controls]');return !audio.paused&&audio.currentTime>=1;});
  await audio.click({position:{x:18,y:25}});await page.waitForFunction(()=>document.querySelector('audio[controls]').paused);
  const audioSeconds=await audio.evaluate(audio=>audio.currentTime);assert(audioSeconds>=1);
  const copies=[];
  for(const file of expected){
    assert(file.bytes<=1024*1024,'Synthetic fixture bound');
    const data=await page.evaluate(async id=>{
      const response=await fetch(`/shared-media/${id}/download`,{cache:'no-store'});
      return {status:response.status,mime:response.headers.get('content-type'),disposition:response.headers.get('content-disposition'),bytes:[...new Uint8Array(await response.arrayBuffer())]};
    },file.id);
    assert.equal(data.status,200);assert.equal(data.mime,'application/octet-stream');assert(data.disposition.startsWith('attachment;'));assert.equal(data.bytes.length,file.bytes);
    const bytes=Buffer.from(data.bytes);assert.equal(createHash('sha256').update(bytes).digest('hex'),file.sha256);copies.push({id:file.id,bytes:bytes.length,sha256:file.sha256});
  }
  const isolation=await page.evaluate(async()=>{
    const {csrf}=await(await fetch('/api/session')).json(),headers={'X-Tonalli-Csrf':csrf};const rows={};
    for(const path of ['/api/notes','/api/files','/api/reading','/api/preferences','/api/sharing'])rows[path]=(await fetch(path,{headers})).status;
    return rows;
  });assert(Object.values(isolation).every(status=>status===404));
  assert(!requests.some(path=>path==='/api/video/progress'||path.startsWith('/api/music')),'Shared playback must not call private media controls');
  return {onlyChosenFilesListed:true,chosenCount:expected.length,imageDecoded:true,textEscaped:true,htmlNotExecuted:true,videoDecoded:video,seekSeconds,audioDecodedAndPlayed:true,audioSeconds,downloadedBytesMatch:copies,actualBrowserDownload:actualDownload,privateRoutesDenied:isolation,noPrivatePlaybackWrites:true,noAutoplay:true};
}
