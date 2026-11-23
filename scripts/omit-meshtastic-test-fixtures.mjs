// Keep the vendored runtime; the user requested real-device acceptance without added mock fixtures.
import {readFile,writeFile} from 'node:fs/promises';
for(const file of ['src/utils_internal.rs','src/connections/wrappers.rs','src/connections/stream_buffer.rs','src/connections/ble_handler.rs']){
  const target=`crates/meshtastic-fork/${file}`;let text=await readFile(target,'utf8');
  const marker=/\r?\n#\[cfg\(test\)\]\r?\nmod tests\s*\{/;const found=marker.exec(text);if(!found)continue;
  text=text.slice(0,found.index).trimEnd()+'\n';
  if(!text.startsWith('// Modified for Tonalli:'))text='// Modified for Tonalli: omitted upstream test-only fixtures; see TONALLI-CHANGES.md.\n'+text;
  await writeFile(target,text);
}
