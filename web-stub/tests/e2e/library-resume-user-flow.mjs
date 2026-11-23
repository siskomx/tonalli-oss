import assert from 'node:assert/strict';
import {destination,appWords} from './consumer-gui-flow.mjs';

// Use the installed Knots fixture and task-created managed text, never personal Notes.
export async function unifiedLibraryCheck(page,{fileName,editedText,articleTitle='Trefoil knot',packId='wikipedia-en-knots'}){
  const t=await appWords(page);await destination(page,'library');
  const library=page.getByTestId('unified-library');await library.waitFor();
  await library.getByLabel(t['unified.category'],{exact:true}).selectOption('all');
  await library.getByLabel(t['unified.search'],{exact:true}).fill(fileName);
  const file=library.getByRole('link',{name:fileName,exact:true});await file.waitFor();
  await file.click();const manager=page.getByTestId('managed-files');
  await manager.getByLabel(t['files.text'],{exact:true}).waitFor();
  assert.equal(await manager.getByLabel(t['files.text'],{exact:true}).inputValue(),editedText);
  await destination(page,'library');
  await library.getByLabel(t['unified.category'],{exact:true}).selectOption('books');
  await library.getByLabel(t['unified.search'],{exact:true}).fill('Trefoil');
  const href=`/article?${new URLSearchParams({pack:packId,url:'C/Trefoil_knot'})}`;
  await library.locator(`a[href="${href}"]`).click();
  await page.waitForFunction(()=>window.__ready?.article?.resourcesLoaded===true);
  const frame=page.frames().find(frame=>new URL(frame.url()).pathname.startsWith('/zim/'));assert.ok(frame);
  const saved=page.waitForResponse(response=>new URL(response.url()).pathname==='/api/reading'&&response.request().method()==='POST'&&response.status()===200);
  await frame.evaluate(()=>window.scrollTo(0,250));await saved;
  const y=await frame.evaluate(()=>window.scrollY);assert.ok(y>=200,'Fixture article must scroll');
  await destination(page,'library');const history=page.getByTestId('reading-history');
  const entry=history.locator('li').filter({has:page.locator(`a[href="${href}"]`)});await entry.waitFor();
  const bookmark=entry.getByRole('button',{name:t['reading.bookmark'],exact:true});
  if(await bookmark.count())await bookmark.click();
  await entry.getByRole('button',{name:t['reading.unbookmark'],exact:true}).waitFor();
  return {managedFileOpened:true,crossBookArticleOpened:true,scroll:y,bookmarked:true,articleTitle};
}
export async function readingReopenCheck(page,{articleTitle='Trefoil knot',minimumScroll=200,packId='wikipedia-en-knots'}={}){
  const t=await appWords(page);await destination(page,'library');
  const history=page.getByTestId('reading-history');
  const href=`/article?${new URLSearchParams({pack:packId,url:'C/Trefoil_knot'})}`;
  const row=history.locator('li').filter({has:page.locator(`a[href="${href}"]`)});
  await row.getByRole('button',{name:t['reading.unbookmark'],exact:true}).waitFor();
  await row.getByRole('link',{name:articleTitle,exact:true}).click();
  await page.waitForFunction(()=>window.__ready?.article?.resourcesLoaded===true);
  const frame=page.frames().find(frame=>new URL(frame.url()).pathname.startsWith('/zim/'));assert.ok(frame);
  await frame.waitForFunction(minimum=>window.scrollY>=minimum,minimumScroll);
  return {bookmarkRetained:true,scrollRestored:await frame.evaluate(()=>window.scrollY)};
}
