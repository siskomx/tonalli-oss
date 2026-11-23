// Reproduce the bounded-stream adaptation of the exact crates.io 0.1.9 source.
import {cp,readFile,writeFile} from 'node:fs/promises';
import path from 'node:path';
const source=process.argv[2];if(!source)throw Error('Pass the meshtastic-0.1.9 registry directory');
const dest='crates/meshtastic-fork';
await cp(source,dest,{recursive:true,errorOnExist:true,force:false});
for(const file of ['src/lib.rs','src/connections/handlers.rs','src/connections/stream_api.rs','src/connections/stream_buffer.rs']) {
  let text=await readFile(path.join(dest,file),'utf8');
  text='// Modified for Tonalli: bounded queues, fail-closed overflow, bounded framing and complete writes.\n'+text;
  text=text.replaceAll('UnboundedSender','Sender').replaceAll('UnboundedReceiver','Receiver');
  for(const [type,capacity] of [['EncodedToRadioPacketWithHeader',16],['IncomingStreamData',32],['protobufs::FromRadio',128]])text=text.replaceAll(`tokio::sync::mpsc::unbounded_channel::<${type}>()`,`tokio::sync::mpsc::channel::<${type}>(${capacity})`);
  if(file.endsWith('handlers.rs'))text=text.replace('.send(data)','.send(data).await').replace('.send(packet_with_header)','.send(packet_with_header).await').replace('.write(message.data())','.write_all(message.data())').replace('buffer.process_incoming_bytes(message);','if buffer.process_incoming_bytes(message).is_err() { return; }');
  if(file.endsWith('stream_api.rs'))text=text.replace('.send(data_with_header)','.send(data_with_header).await');
  if(file.endsWith('stream_buffer.rs')){
    text=text.replace('pub fn process_incoming_bytes(&mut self, message: IncomingStreamData) {','pub fn process_incoming_bytes(&mut self, message: IncomingStreamData) -> Result<(), ()> {\n        if self.buffer.len() + message.as_ref().len() > 66560 { return Err(()); }');
    text=text.replace('self.decoded_packet_tx.send(decoded_packet)','self.decoded_packet_tx.try_send(decoded_packet)');
    text=text.replace(/error!\("Failed to send decoded packet: \{e\}"\);\s+break;/,'let _ = e;\n                    return Err(());');
    text=text.replace(/self.buffer.len\(\)\s+\);\s+\}/,'self.buffer.len()\n        );\n        Ok(())\n    }');
    text=text.replace('self.log_line.push(b);','if self.log_line.len() >= 1024 { self.log_line.clear(); }\n                self.log_line.push(b);');
    text=text.replace('use tokio::sync::mpsc::unbounded_channel;','use tokio::sync::mpsc::channel;').replaceAll('unbounded_channel::<protobufs::FromRadio>()','channel::<protobufs::FromRadio>(128)');
  }
  await writeFile(path.join(dest,file),text);
}
await writeFile(path.join(dest,'TONALLI-CHANGES.md'),'# Tonalli adaptation of meshtastic 0.1.9\n\nExact crates.io package, GPL-3.0; original LICENSE retained. Only stream queue/framing changes: outbound16, read32, decoded128 entries; async backpressure on byte/write queues; decoded overflow ends processing; framing66560B and log1024B limits; write_all prevents partial writes. No radio/protobuf/GATT replacement. Reproduce with scripts/vendor-meshtastic.mjs. Hardware acceptance remains required.\n');
const ble=path.join(dest,'src/connections/ble_handler.rs');
await writeFile(ble,'// Modified for Tonalli: stop discovery after collecting results.\n'+(await readFile(ble,'utf8')).replace('adapter.peripherals().await','let peripherals = adapter.peripherals().await;\n        adapter.stop_scan().await?;\n        peripherals'));
await import('./omit-meshtastic-test-fixtures.mjs');
// iOS has no USB host and serialport's port lister needs IOKit: report no serial ports there.
const utils=path.join(dest,'src/utils_internal.rs');
await writeFile(utils,(await readFile(utils,'utf8'))
  .replace('use tokio_serial::{available_ports, SerialPort, SerialStream};','#[cfg(not(target_os = "ios"))]\nuse tokio_serial::available_ports;\nuse tokio_serial::{SerialPort, SerialStream};')
  .replace('    let ports = available_ports()?\n        .into_iter()\n        .map(|port| port.port_name)\n        .collect();\n\n    Ok(ports)\n}','    #[cfg(target_os = "ios")]\n    {\n        // Modified for Tonalli: iOS apps have no USB host, and serialport\'s port lister needs\n        // IOKit, which iOS does not provide.\n        Ok(Vec::new())\n    }\n    #[cfg(not(target_os = "ios"))]\n    {\n        let ports = available_ports()?\n            .into_iter()\n            .map(|port| port.port_name)\n            .collect();\n\n        Ok(ports)\n    }\n}'));
await writeFile(path.join(dest,'TONALLI-CHANGES.md'),(await readFile(path.join(dest,'TONALLI-CHANGES.md'),'utf8'))+'\niOS: `available_serial_ports` returns no ports there (src/utils_internal.rs), because iOS has no USB host and the serialport lister needs IOKit.\n');
