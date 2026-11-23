use std::error::Error;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tonalli_core::pack::MmapSource;
use tonalli_core::zim::Zim;

fn percentile(values: &mut [f64], percentile: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let rank = ((values.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    values[rank.min(values.len() - 1)]
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut check: Vec<_> = (1..=30).map(f64::from).collect();
    assert_eq!(percentile(&mut check, 0.5), 15.0);
    assert_eq!(percentile(&mut check, 0.95), 29.0);
    let path = std::env::var_os("TONALLI_BENCH_ZIM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/spike/packs/wikipedia_en_all_nopic_2026-06.zim")
        });
    let index = std::env::var_os("TONALLI_BENCH_TITLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| path.with_extension("titles.idx"));
    let query = std::env::var("TONALLI_BENCH_PREFIX").unwrap_or_else(|_| "earth".into());
    let article = std::env::var("TONALLI_BENCH_ARTICLE").unwrap_or_else(|_| "C/Earth".into());
    let mode = std::env::var("TONALLI_BENCH_MODE").unwrap_or_else(|_| "warm".into());
    if !matches!(
        mode.as_str(),
        "warm" | "fresh" | "cold-suggest" | "cold-article"
    ) {
        return Err("unsupported TONALLI_BENCH_MODE".into());
    }
    let fresh = mode == "fresh";
    let zim = Zim::open(Arc::new(MmapSource::open(&path)?))?;
    println!(
        "dataset={path:?} bytes={} title_rows={}",
        std::fs::metadata(&path)?.len(),
        zim.title_count()?
    );
    if !index.exists() {
        let start = Instant::now();
        zim.build_title_index(&index)?;
        println!(
            "title_table_build_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    println!(
        "title_table={index:?} bytes={}",
        std::fs::metadata(&index)?.len()
    );
    let zim = zim.with_title_index(&index)?;
    if mode == "cold-suggest" || mode == "cold-article" {
        let operation = if mode == "cold-suggest" {
            "suggest"
        } else {
            "article"
        };
        println!("READY pid={} operation={operation}", std::process::id());
        io::stdout().flush()?;
        let mut command = String::new();
        if io::stdin().read_line(&mut command)? == 0
            || command.trim_end_matches(['\r', '\n']) != "run"
        {
            return Err("expected exactly 'run' on stdin after READY".into());
        }
        let start = Instant::now();
        let milliseconds = if operation == "suggest" {
            let hits = black_box(zim.suggest(&query, 10)?);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            if hits.is_empty() {
                return Err("benchmark prefix has no hits".into());
            }
            elapsed
        } else {
            let page = black_box(zim.article(&article)?);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            if page.bytes.is_empty() {
                return Err("benchmark article is empty".into());
            }
            elapsed
        };
        println!(
            "condition=operator-reset-required operation={operation} runs=1 reset_verified=false"
        );
        println!("{operation}_ms sample={milliseconds:.3}");
        return Ok(());
    }
    if !fresh {
        black_box(zim.suggest(&query, 10)?);
        black_box(zim.article(&article)?);
    }
    let runs = if fresh { 1 } else { 30 };
    let mut search = Vec::new();
    let mut open = Vec::new();
    for _ in 0..runs {
        let start = Instant::now();
        let hits = black_box(zim.suggest(&query, 10)?);
        search.push(start.elapsed().as_secs_f64() * 1000.0);
        if hits.is_empty() {
            return Err("benchmark prefix has no hits".into());
        }
        let start = Instant::now();
        let page = black_box(zim.article(&article)?);
        open.push(start.elapsed().as_secs_f64() * 1000.0);
        if page.bytes.is_empty() {
            return Err("benchmark article is empty".into());
        }
    }
    println!(
        "condition={} runs={runs} prefix={query:?} article={article:?}",
        if fresh {
            "fresh-process; OS cache not flushed by harness"
        } else {
            "warm process"
        }
    );
    if fresh {
        println!("suggest_ms sample={:.3}", search[0]);
        println!("article_ms sample={:.3}", open[0]);
    } else {
        println!(
            "suggest_ms p50={:.3} p95={:.3}",
            percentile(&mut search, 0.5),
            percentile(&mut search, 0.95)
        );
        println!(
            "article_ms p50={:.3} p95={:.3}",
            percentile(&mut open, 0.5),
            percentile(&mut open, 0.95)
        );
    }
    Ok(())
}
