//! Real TLS host plus loopback app for the native-client browser acceptance check.
use std::{
    io::{self, BufRead, Seek, Write},
    path::PathBuf,
    sync::Arc,
};
use tonalli_core::{
    pack::PreadSource,
    server::{LoopbackServer, PackId, PackRegistry, Server, ServerCfg},
    zim::Zim,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let web = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Pass built UI directory")?,
    )
    .canonicalize()?;
    let root = tempfile::tempdir()?;
    let receipt = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .ok_or("Pass a private IPC receipt path")?,
    );
    let registry = Arc::new(PackRegistry::default());
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
    registry.register_zim(
        PackId::new("shared-book")?,
        Arc::new(Zim::open(Arc::new(PreadSource::open(&fixture)?))?),
    )?;
    let mut selected = vec![PackId::new("shared-book")?];
    if let Some(path) = std::env::args_os().nth(3) {
        let id = PackId::new("shared-map")?;
        registry.register_tiles(
            id.clone(),
            Arc::new(PreadSource::open(&PathBuf::from(path))?),
        )?;
        selected.push(id);
    }
    let mut host = Server::start(
        ServerCfg {
            web_root: web.clone(),
            identity_dir: root.path().join("identity"),
            packs: selected,
            ..ServerCfg::default()
        },
        registry,
    )?;
    let mut app = LoopbackServer::start(
        Arc::new(PackRegistry::default()),
        root.path().join("private"),
        web.clone(),
    )?;
    attach_catalog(&app, &root.path().join("private"))?;
    // Generated test credentials go only to the caller's private temporary file.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(receipt)?;
    serde_json::to_writer(
        &mut output,
        &serde_json::json!({"launch":app.info().launch_url,"invitation":tonalli_core::remote::invitation(host.local_addr(),&host.certificate_der()),"fingerprint":host.certificate_sha256(),"code":host.pairing_code()?}),
    )?;
    println!("ready");
    let mut held_copy = None;
    for line in io::stdin().lock().lines() {
        match line?.as_str() {
            "hold-copy" => {
                let data = root.path().join("private");
                let store = tonalli_core::sync::SyncStore::open(&data)?;
                let row = store
                    .list()?
                    .into_iter()
                    .next()
                    .ok_or("No owned fixture copy")?;
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.share_mode(1 | 2);
                }
                held_copy = Some(options.open(data.join(store.cache_spec(&row)?.path))?);
                println!("copy-held");
            }
            "release-copy" => {
                drop(held_copy.take());
                println!("copy-released");
            }
            "kick" => {
                for device in host.devices()? {
                    host.kick(&device.id)?;
                }
                println!("revoked");
            }
            "stop" => break,
            "reopen" => {
                app.shutdown()?;
                app = LoopbackServer::start(
                    Arc::new(PackRegistry::default()),
                    root.path().join("private"),
                    web.clone(),
                )?;
                attach_catalog(&app, &root.path().join("private"))?;
                output.set_len(0)?;
                output.rewind()?;
                serde_json::to_writer(
                    &mut output,
                    &serde_json::json!({"launch":app.info().launch_url}),
                )?;
                output.flush()?;
                println!("reopened");
            }
            _ => {}
        }
    }
    app.shutdown()?;
    host.shutdown()?;
    Ok(())
}

fn attach_catalog(
    server: &LoopbackServer,
    data: &std::path::Path,
) -> Result<(), tonalli_core::pack::PackError> {
    server.attach_content(
        Arc::new(tonalli_core::content::ManagedStore::open(
            data,
            tonalli_core::catalog::PUBLISHER_PUBLIC_KEY,
        )?),
        Arc::new(tonalli_core::catalog::Catalog::bundled()?),
    )
}
