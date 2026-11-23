use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use tonalli_mesh_gateway::{config::Config, gateway, MeshError};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), MeshError> {
    let mut path = std::env::var_os("TONALLI_MESH_CONFIG").map(PathBuf::from);
    let mut handoff = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            path = Some(PathBuf::from(args.next().ok_or(MeshError::Configuration)?));
        } else if arg == "--confirm-node-free" {
            handoff = true;
        } else if arg == "--help" {
            println!("tonalli-mesh-spike --config EXTERNAL_CONFIG [--confirm-node-free]\nNo transport is opened without an enabled configuration and explicit handoff.\nUSB or BLE is selected in the external configuration; no automatic fallback.");
            return Ok(());
        } else {
            return Err(MeshError::Configuration);
        }
    }
    let path = path.ok_or(MeshError::Configuration)?;
    let config = Config::load(&path)?;
    if !config.enabled {
        println!("Mesh is disabled; no radio connection opened");
        return Ok(());
    }
    gateway::external_config(&path)?;
    let stop = CancellationToken::new();
    let interrupt = stop.clone();
    let signal = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupt.cancel();
        }
    });
    let mut gateway = match gateway::open(&config, handoff, stop).await {
        Ok(g) => g,
        Err(e) => {
            signal.abort();
            return Err(e);
        }
    };
    println!(
        "Radio configuration complete. Firmware: {}",
        gateway.firmware
    );
    println!(
        "Discovered {} nodes and {} enabled channels",
        gateway.list_nodes().await.len(),
        gateway.channels.len()
    );
    let result=async{
        if let Some(text)=config.spike.text.as_deref(){
            gateway.send_text(&config,text).await?;println!("One text queued to transport; delivery unconfirmed");
            if config.spike.show_test_text{println!("Sent test text: {}",text.escape_debug());}
            let received=gateway.receive_text(&config).await?;println!("Received channel text from another node");
            if config.spike.show_test_text{println!("Received test text: {}",received.escape_debug());}
        }else{
            println!("Read-only inspection; no text sent. Configure an accepted firmware and discovered channel before sending.");
            println!("Enabled channel indexes: {:?}",gateway.channels);
        }
        Ok(())
    }.await;
    let shutdown = gateway.close().await;
    signal.abort();
    result.and(shutdown)
}
