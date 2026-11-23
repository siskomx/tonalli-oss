//! Task-owned country-download acceptance; never opens the user's normal app data.
use std::{path::PathBuf, time::Duration};
use tonalli_core::country_maps::CountryMaps;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().ok_or("task-owned output directory required")?);
    let id = args.next().ok_or("country required")?;
    let zoom = args.next().unwrap_or_else(|| "8".into()).parse::<u8>()?;
    let maps = CountryMaps::open(root)?;
    maps.reconcile()?;
    let client = reqwest::blocking::Client::builder().https_only(true).connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(90)).build()?;
    for estimate in [true, false] {
        let ticket = maps.prepare(&id, zoom, estimate)?;
        let run = maps.claim(ticket)?;
        let result = maps.execute(&run, &client);
        maps.finish(run)?;
        result?;
        let row = maps.list()?.into_iter().find(|row| row.id == id).ok_or("missing country")?;
        println!("country={} state={} output={} transfer={} installed={}", row.id, row.state, row.output_bytes, row.download_bytes, row.installed.is_some());
    }
    Ok(())
}
