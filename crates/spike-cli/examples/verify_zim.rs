//! Read-only internal ZIM checksum check, separate from publisher SHA-256 trust.
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tonalli_core::pack::{PackSource, PreadSource};
use tonalli_core::zim::Zim;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = PathBuf::from(arguments.next().ok_or("usage: verify_zim PATH")?);
    if arguments.next().is_some() {
        return Err("usage: verify_zim PATH".into());
    }
    let source: Arc<dyn PackSource> = Arc::new(PreadSource::open(&path)?);
    let bytes = source.len();
    let zim = Zim::open(source)?;
    let start = Instant::now();
    zim.verify_md5()?;
    let elapsed = start.elapsed();
    let hex = |value: &[u8]| {
        value
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    println!("dataset={} bytes={bytes}", path.display());
    println!(
        "uuid={} stored_md5={}",
        hex(&zim.uuid()),
        hex(&zim.stored_md5()?)
    );
    println!(
        "title_rows={} internal_md5=verified checksum_ms={:.3}",
        zim.title_count()?,
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}
