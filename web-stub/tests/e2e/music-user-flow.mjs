import assert from 'node:assert/strict';
import {appWords,destination} from './consumer-gui-flow.mjs';

// Only task-owned silent PCM files/playlists. Does not touch Notes or other media.
export async function musicQueueCheck(page){
  const t=await appWords(page);await destination(page,'music');
  const music=page.getByTestId('music-page'),player=page.getByTestId('music-player');
  await music.locator('#music-search').fill('Tonalli-music-check-55ac');
  const a=music.locator('li').filter({hasText:'Tonalli-music-check-55ac-A.wav'}).first();
  const b=music.locator('li').filter({hasText:'Tonalli-music-check-55ac-B.wav'}).first();
  await a.getByRole('button',{name:t['music.play'],exact:true}).click();
  await player.getByRole('button',{name:t['music.pause'],exact:true}).waitFor();
  await b.getByRole('button',{name:t['music.addQueue'],exact:true}).click();
  await player.getByRole('button',{name:t['music.pause'],exact:true}).click();
  await player.getByRole('button',{name:t['music.play'],exact:true}).waitFor();
  const slider=player.getByRole('slider',{name:t['music.seek'],exact:true});await slider.waitFor();
  const box=await slider.boundingBox();assert(box);await slider.click({position:{x:box.width*.25,y:box.height/2}});
  await page.waitForTimeout(1500);
  const first=await snapshot(page);assert(first.queue.seconds>=12&&first.queue.seconds<=18,JSON.stringify(first));
  const label=music.locator('#playlist-name');
  const choose=music.getByRole('combobox',{name:t['music.choose'],exact:true});
  const addition=music.getByRole('combobox',{name:t['music.addItem'],exact:true});
  const existingChild=first.playlists.filter(row=>row.name==='Music check 55ac child');assert(existingChild.length<=1);
  if(!existingChild.length){await label.fill('Music check 55ac child');await music.getByRole('button',{name:t['music.create'],exact:true}).click();}
  await choose.selectOption({label:'Music check 55ac child'});
  if(!existingChild[0]?.items.length){await addition.selectOption({label:'Tonalli-music-check-55ac-B.wav'});await music.getByRole('button',{name:t['music.addItem'],exact:true}).click();}
  const existingParent=first.playlists.filter(row=>row.name==='Music check 55ac parent');assert(existingParent.length<=1);
  if(!existingParent.length){await label.fill('Music check 55ac parent');await music.getByRole('button',{name:t['music.create'],exact:true}).click();}
  await choose.selectOption({label:'Music check 55ac parent'});
  if(!existingParent[0]?.items.length){await addition.selectOption({label:'Tonalli-music-check-55ac-A.wav'});await music.getByRole('button',{name:t['music.addItem'],exact:true}).click();}
  if((existingParent[0]?.items.length||0)<2){await addition.selectOption({label:`${t['music.playlist']}: Music check 55ac child`});await music.getByRole('button',{name:t['music.addItem'],exact:true}).click();}
  const next=await snapshot(page);
  const parent=next.playlists.find(row=>row.name==='Music check 55ac parent'),child=next.playlists.find(row=>row.name==='Music check 55ac child');
  assert(parent&&child);assert.equal(parent.items.length,2);assert.deepEqual(parent.items[1],{kind:'playlist',id:child.id});
  await destination(page,'library');await player.getByRole('button',{name:t['music.play'],exact:true}).waitFor();
  const history=page.getByTestId('reading-history');await history.getByRole('link',{name:'Tonalli-music-check-55ac-A.wav',exact:true}).click();
  await page.waitForFunction(()=>window.__ready?.route==='/music'&&window.__ready.ready);
  const resumed=await snapshot(page);assert.equal(resumed.queue.current,0);assert(Math.abs(resumed.queue.seconds-first.queue.seconds)<1);
  return {native:resumed.native,seekSeconds:first.queue.seconds,nestedPlaylist:true,queueRetained:true,historyOpensMusic:true,pausedPositionRetained:true};
}
export async function snapshot(page){return page.evaluate(async()=>{
  const {csrf}=await(await fetch('/api/session')).json(),headers={'X-Tonalli-Csrf':csrf};
  const value=await(await fetch('/api/music',{headers})).json();return {native:value.native,...value.state};
});}

async function clickMusic(page,button,action){
  const [response]=await Promise.all([page.waitForResponse(response=>new URL(response.url()).pathname==='/api/music'
    &&response.request().method()==='POST'&&response.request().postDataJSON()?.action===action),button.click()]);
  assert.equal(response.status(),200);
}
export async function musicEditCheck(page){
  const t=await appWords(page);await destination(page,'music');const music=page.getByTestId('music-page');
  await music.locator('#music-search').fill('Tonalli-music-check-55ac');
  const before=await snapshot(page),a=before.queue.tracks[before.queue.current];assert(a);
  const rows=await page.evaluate(async()=>{const {csrf}=await(await fetch('/api/session')).json();return(await(await fetch('/api/files',{headers:{'X-Tonalli-Csrf':csrf}})).json()).files;});
  const b=rows.find(row=>row.name==='Tonalli-music-check-55ac-B.wav');assert(b);
  assert.equal(rows.find(row=>row.id===a)?.name,'Tonalli-music-check-55ac-A.wav');
  if(!before.queue.tracks.includes(b.id))await clickMusic(page,music.locator('li').filter({hasText:b.name}).first().getByRole('button',{name:t['music.addQueue'],exact:true}),'queue');
  const queue=music.getByRole('heading',{name:t['music.queue'],exact:true}).locator('..');
  const rowB=queue.locator('li').filter({hasText:b.name});
  await clickMusic(page,rowB.getByRole('button',{name:`${t['music.up']} ${b.name}`,exact:true}),'queue');
  const moved=await snapshot(page);assert.deepEqual(moved.queue.tracks,[b.id,a]);assert.equal(moved.queue.current,1);assert.equal(moved.queue.seconds,before.queue.seconds);
  await clickMusic(page,rowB.getByRole('button',{name:t['music.remove'],exact:true}),'queue');
  const removed=await snapshot(page);assert.deepEqual(removed.queue.tracks,[a]);assert.equal(removed.queue.current,0);assert.equal(removed.queue.seconds,before.queue.seconds);
  const label=music.locator('#playlist-name'),choose=music.getByRole('combobox',{name:t['music.choose'],exact:true});
  if(!removed.playlists.some(row=>row.name==='Music check 55ac delete')){await label.fill('Music check 55ac delete');await clickMusic(page,music.getByRole('button',{name:t['music.create'],exact:true}),'create_playlist');}
  await choose.selectOption({label:'Music check 55ac delete'});
  const deleteButton=music.getByRole('button',{name:t['music.delete'],exact:true}).first();
  const dialog=music.getByRole('dialog',{name:t['music.delete'],exact:true});
  await deleteButton.click();await dialog.getByRole('button',{name:t['files.cancel'],exact:true}).click();
  assert((await snapshot(page)).playlists.some(row=>row.name==='Music check 55ac delete'));
  await deleteButton.click();await clickMusic(page,dialog.getByRole('button',{name:t['music.delete'],exact:true}),'delete_playlist');
  const after=await snapshot(page);assert(!after.playlists.some(row=>row.name==='Music check 55ac delete'));assert.deepEqual(after.queue,removed.queue);
  const afterRows=await page.evaluate(async()=>{const {csrf}=await(await fetch('/api/session')).json();return(await(await fetch('/api/files',{headers:{'X-Tonalli-Csrf':csrf}})).json()).files;});
  assert.deepEqual(afterRows,rows);
  return {queueReorderedWithoutLosingPosition:true,queueRemovalKeptCurrentTrack:true,playlistDeleteCancelledThenConfirmed:true,managedFilesUnchanged:true};
}
