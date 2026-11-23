#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod headless_run;
mod session;
mod shell;

use tonalli_desktop::options::{Options, USAGE};

fn main() {
    let started = std::time::Instant::now();
    let arguments = std::env::args_os()
        .skip(1)
        .map(|value| value.into_string())
        .collect::<Result<Vec<_>, _>>();
    let result = arguments
        .map_err(|_| "Arguments must be valid Unicode".to_owned())
        .and_then(|args| {
            if args.as_slice() == ["--help"] {
                println!("{USAGE}");
                return Ok(());
            }
            let options = Options::parse(args)?;
            #[cfg(windows)]
            if options.headless.is_some() {
                return headless_run::run(options);
            }
            #[cfg(not(windows))]
            if options.headless.is_some() {
                return Err("Headless sharing is available on Windows only".into());
            }
            shell::run(options, started).map_err(|error| {
                let detail = format!("{error:?}");
                let detail = detail
                    .split_once("token=")
                    .map_or(detail.as_str(), |(prefix, _)| prefix);
                format!(
                    "Desktop shell startup failed: {}",
                    detail.chars().take(512).collect::<String>()
                )
            })
        });
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
