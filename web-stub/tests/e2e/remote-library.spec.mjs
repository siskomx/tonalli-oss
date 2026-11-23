import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { once } from 'node:events';
import { resolve,join,sep } from 'node:path';
import { mkdtemp,readFile,rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { chromium } from 'playwright-core';

test('native client uses real TLS pairing, reads, survives reload and returns locally after host revocation', async () => {
  const temporary=await mkdtemp(join(tmpdir(),'tonalli-native-client-'));const receipt=join(temporary,'private-ipc.json');
  const host = spawn(resolve('../target/debug/examples/remote-library-check.exe'), [resolve('dist'),receipt], { windowsHide:true, stdio:['pipe','pipe','pipe'] });
  const lines = createInterface({input:host.stdout}); const iterator=lines[Symbol.asyncIterator]();
  const first=await Promise.race([iterator.next(),new Promise((_,reject)=>setTimeout(()=>reject(Error('Real host startup timed out')),20000).unref())]);
  assert.equal(first.done,false);assert.equal(first.value,'ready');const info=JSON.parse(await readFile(receipt,'utf8'));
  const browser=await chromium.launch({executablePath:process.env.TONALLI_ARTICLE_BROWSER||'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',headless:true});
  const context=await browser.newContext();const page=await context.newPage();page.setDefaultTimeout(15000);
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  try {
    await page.goto(info.launch);await page.getByRole('link',{name:'Connect to another device',exact:true}).click();
    const card=page.getByTestId('remote-library');
    await card.locator('[name="library-invitation"]').fill(info.invitation);
    await card.locator('code').filter({hasText:info.fingerprint}).waitFor();
    await card.locator('[name="library-pairing-code"]').fill(info.code);
    await card.locator('[name="library-device-label"]').fill('Owned app reader');
    const connect=card.getByRole('button',{name:'Connect to another device',exact:true});assert.equal(await connect.isDisabled(),true);
    await card.getByRole('checkbox').check();await connect.click();
    await page.getByRole('link',{name:'Connection settings'}).waitFor();
    await page.getByTestId('library-row').first().waitFor();
    assert.equal(await page.getByTestId('nav-notes').count(),0);
    const privacy=await page.evaluate(async()=>{const {csrf}=await(await fetch('/api/session')).json();return Promise.all(['/api/notes','/api/files','/api/sharing','/api/reading','/media/private','/native/open'].map(async path=>[path,(await fetch(path,{headers:{'X-Tonalli-Csrf':csrf}})).status]));});
    assert.ok(privacy.every(([,status])=>status===404));
    await page.getByTestId('library-row').first().click();
    const article=page.locator('iframe');await article.waitFor();
    await page.waitForFunction(()=>document.querySelector('iframe')?.contentDocument?.body?.textContent?.trim().length>0);
    await page.reload();await page.waitForFunction(()=>document.querySelector('iframe')?.contentDocument?.body?.textContent?.trim().length>0);
    host.stdin.write('kick\n');assert.equal((await iterator.next()).value,'revoked');
    await page.getByTestId('nav-library').click();await page.getByRole('alert').filter({hasText:/library/i}).waitFor();
    await page.getByTestId('nav-settings').click();await page.getByRole('button',{name:'Return to my library',exact:true}).click();
    await page.getByTestId('nav-notes').waitFor();assert.equal(await page.getByRole('link',{name:'Connection settings'}).count(),0);
    const local=await page.evaluate(async()=>{const{csrf}=await(await fetch('/api/session')).json();return(await(await fetch('/api/packs',{headers:{'X-Tonalli-Csrf':csrf}})).json()).packs;});
    assert.deepEqual(local,[]);assert.deepEqual(errors,[]);
  } finally {
    await context.close();await browser.close();host.stdin.end('stop\n');
    if(host.exitCode===null)await once(host,'exit');lines.close();
    assert.ok(resolve(temporary).startsWith(resolve(tmpdir())+sep+'tonalli-native-client-'));await rm(temporary,{recursive:true,force:true});
  }
});
