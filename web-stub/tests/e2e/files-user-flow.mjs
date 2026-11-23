import assert from 'node:assert/strict';
import {appWords,destination} from './consumer-gui-flow.mjs';

// Use only recorded task-created files and an owned synthetic Admin profile.
export async function manageFile(page,{name,nextName,originalText,editedText,remove=false}) {
  const t=await appWords(page);
  if(new URL(page.url()).pathname!=='/files')await destination(page,'files');
  const manager=page.getByTestId('managed-files');
  const row=()=>manager.getByTestId('managed-file').filter({has:page.getByRole('heading',{name:nextName||name,exact:true})});
  const initial=manager.getByTestId('managed-file').filter({has:page.getByRole('heading',{name,exact:true})});
  await initial.waitFor();const id=await initial.getAttribute('data-file-id');assert.match(id,/^[a-f0-9]{32}$/);
  if(nextName) {
    await initial.getByRole('button',{name:t['files.rename'],exact:true}).click();
    await manager.getByLabel(t['files.newName'],{exact:true}).fill(nextName);
    await manager.getByRole('button',{name:t['files.saveName'],exact:true}).click();await row().waitFor();
    assert.equal(await row().getAttribute('data-file-id'),id);
  }
  await row().getByRole('button',{name:t['files.viewText'],exact:true}).click();
  const editor=manager.getByLabel(t['files.text'],{exact:true});await editor.waitFor();assert.equal(await editor.inputValue(),originalText);
  if(editedText!==undefined) {
    await editor.fill(editedText);await manager.getByRole('button',{name:t['files.saveText'],exact:true}).click();await editor.waitFor({state:'detached'});
    await row().getByRole('button',{name:t['files.viewText'],exact:true}).click();await editor.waitFor();assert.equal(await editor.inputValue(),editedText);
  }
  await manager.getByRole('button',{name:t['files.close'],exact:true}).click();
  await row().getByRole('button',{name:t['files.delete'],exact:true}).click();
  const dialog=page.getByRole('dialog',{name:t['files.confirmDelete'],exact:true});await dialog.waitFor();
  await dialog.getByRole('button',{name:t['files.cancel'],exact:true}).click();await dialog.waitFor({state:'hidden'});await row().waitFor();
  if(remove) {
    await row().getByRole('button',{name:t['files.delete'],exact:true}).click();await dialog.getByRole('button',{name:t['files.confirmRemove'],exact:true}).click();
    await row().waitFor({state:'detached'});
  }
  assert.equal(await page.getByRole('alert').count(),0);
  return {id,renamed:!!nextName,textChecked:true,edited:editedText!==undefined,deleteCancelled:true,removed:remove};
}
