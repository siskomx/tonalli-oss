use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use reqwest::blocking::Client;
use reqwest::header::{COOKIE, SET_COOKIE};
use spike_cli::serve;
use tonalli_core::pack::PackError;
use tonalli_core::pack::PreadSource;
use tonalli_core::server::LoopbackServer;
use tonalli_core::zim::Zim;

#[allow(dead_code)]
#[path = "../../core/tests/fixtures/pmtiles/mod.rs"]
mod pmtiles_fixture;

fn web() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("index.html"),
        "<!doctype html><title>CLI shell fixture</title>",
    )
    .unwrap();
    directory
}

fn arguments(web: &std::path::Path, data: &std::path::Path) -> Vec<String> {
    vec![
        "--web".into(),
        web.display().to_string(),
        "--data-dir".into(),
        data.display().to_string(),
    ]
}

#[test]
fn serve_requires_explicit_web_and_private_data_paths() {
    for arguments in [vec![], vec!["--web".into(), "missing".into()]] {
        assert!(matches!(
            serve::start(&arguments),
            Err(PackError::Mismatch(_))
        ));
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_spike-cli"))
        .arg("serve")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--data-dir"));
}

#[test]
fn serve_rejects_bad_ids_duplicates_kinds_and_unknown_flags_before_listening() {
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    for pack in [
        "../bad=missing.zim",
        "=missing.zim",
        "bad",
        "good=missing.txt",
    ] {
        let mut args = arguments(shell.path(), data.path());
        args.extend(["--pack".into(), pack.into()]);
        assert!(matches!(serve::start(&args), Err(PackError::Mismatch(_))));
    }
    let mut args = arguments(shell.path(), data.path());
    args.extend([
        "--pack".into(),
        "same=missing.zim".into(),
        "--pack".into(),
        "same=other.pmtiles".into(),
    ]);
    assert!(matches!(serve::start(&args), Err(PackError::Mismatch(_))));
    let mut args = arguments(shell.path(), data.path());
    args.push("--manifest".into());
    assert!(matches!(serve::start(&args), Err(PackError::Mismatch(_))));
}

#[test]
fn serve_start_owns_a_loopback_server_and_runs_the_launch_session_flow() {
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    let mut server = serve::start(&arguments(shell.path(), data.path())).unwrap();
    let info = server.info();
    assert_ne!(info.port, 0);
    let launch = reqwest::Url::parse(&info.launch_url).unwrap();
    assert_eq!(launch.host_str(), Some("127.0.0.1"));
    assert_eq!(launch.port(), Some(info.port));
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let origin = format!("http://127.0.0.1:{}", info.port);
    assert_eq!(client.get(&origin).send().unwrap().status(), 401);
    let response = client.get(launch).send().unwrap();
    assert_eq!(response.status(), 302);
    let cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
    let page = client.get(&origin).header(COOKIE, &cookies).send().unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().unwrap().contains("CLI shell fixture"));
    let session = client
        .get(format!("{origin}/api/session"))
        .header(COOKIE, cookies)
        .header("Sec-Fetch-Dest", "empty")
        .send()
        .unwrap();
    assert_eq!(session.status(), 200);
    assert!(session.text().unwrap().contains("\"csrf\""));
    server.shutdown().unwrap();
    assert!(client.get(origin).send().is_err());
}

#[test]
fn serve_loads_explicit_zim_and_pmtiles_sources() {
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    let packs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/spike/packs");
    let maps = tempfile::tempdir().unwrap();
    let map = maps.path().join("fixture.pmtiles");
    fs::write(&map, pmtiles_fixture::simple()).unwrap();
    let mut args = arguments(shell.path(), data.path());
    args.extend([
        "--pack".into(),
        format!(
            "knots={}",
            packs.join("wikipedia_en_knots_maxi_2026-10.zim").display()
        ),
        "--pack".into(),
        format!("fixture={}", map.display()),
    ]);
    serve::start(&args).unwrap().shutdown().unwrap();
}

#[test]
fn serve_rejects_a_pmtiles_magic_prefix_without_a_valid_archive() {
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    let maps = tempfile::tempdir().unwrap();
    let path = maps.path().join("invalid.pmtiles");
    fs::write(&path, b"PMTiles\x03").unwrap();
    let mut args = arguments(shell.path(), data.path());
    args.extend(["--pack".into(), format!("invalid={}", path.display())]);
    assert!(serve::start(&args).is_err());
}

#[test]
fn serve_rejects_missing_sources_and_unbuilt_shells() {
    let shell = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    assert!(serve::start(&arguments(shell.path(), data.path())).is_err());
    fs::write(shell.path().join("index.html"), "shell").unwrap();
    let mut args = arguments(shell.path(), data.path());
    args.extend(["--pack".into(), "missing=missing.zim".into()]);
    assert!(serve::start(&args).is_err());
}

#[test]
fn serve_refuses_data_inside_the_served_static_root() {
    let shell = web();
    assert!(matches!(
        serve::start(&arguments(shell.path(), &shell.path().join("private-data"))),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn serve_binary_prints_a_live_loopback_launch_url() {
    use std::io::BufRead;
    use std::process::{Child, Command, Stdio};
    struct Running(Child);
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    let mut process = Running(
        Command::new(env!("CARGO_BIN_EXE_spike-cli"))
            .arg("serve")
            .args(arguments(shell.path(), data.path()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = process.0.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        if std::io::BufReader::new(stdout).read_line(&mut line).is_ok() {
            let _ = send.send(line);
        }
    });
    let line = receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    reader.join().unwrap();
    let url = reqwest::Url::parse(line.trim()).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_eq!(url.path(), "/launch");
    assert!(
        process.0.try_wait().unwrap().is_none(),
        "serve must retain its owned server after printing"
    );
    let response = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(url)
        .send()
        .unwrap();
    assert_eq!(response.status(), 302);
}

fn zim_path(es: bool) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs")
        .join(if es {
            "wikipedia_es_climate-change_maxi_2026-10.zim"
        } else {
            "wikipedia_en_knots_maxi_2026-10.zim"
        })
}
fn title_table(path: &std::path::Path) {
    Zim::open(Arc::new(PreadSource::open(&zim_path(false)).unwrap()))
        .unwrap()
        .build_title_index(path)
        .unwrap();
}
struct Session {
    client: Client,
    origin: String,
    cookies: String,
    csrf: String,
}
impl Session {
    fn login(server: &LoopbackServer) -> Self {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();
        let info = server.info();
        let response = client.get(&info.launch_url).send().unwrap();
        assert_eq!(response.status(), 302);
        let cookies = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        let origin = format!("http://127.0.0.1:{}", info.port);
        let session = client
            .get(format!("{origin}/api/session"))
            .header(COOKIE, &cookies)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .json::<serde_json::Value>()
            .unwrap();
        Self {
            client,
            origin,
            cookies,
            csrf: session["csrf"].as_str().unwrap().into(),
        }
    }
    fn api(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .get(format!("{}{path}", self.origin))
            .header(COOKIE, &self.cookies)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &self.csrf)
    }
}

#[test]
fn serve_attaches_explicit_title_index_in_either_option_order_without_mutating_it() {
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    let index = data.path().join("knots.titles");
    title_table(&index);
    let bytes = fs::read(&index).unwrap();
    let modified = fs::metadata(&index).unwrap().modified().unwrap();
    for before in [true, false] {
        let mut args = arguments(shell.path(), data.path());
        let pack = vec!["--pack".into(), format!("en={}", zim_path(false).display())];
        let table = vec!["--title-index".into(), format!("en={}", index.display())];
        if before {
            args.extend(table);
            args.extend(pack);
        } else {
            args.extend(pack);
            args.extend(table);
        }
        let mut server = serve::start(&args).unwrap();
        let session = Session::login(&server);
        let response = session
            .api("/api/zim/suggest")
            .query(&[("pack", "en"), ("q", "TRÉFOIL KNOT")])
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.json::<serde_json::Value>().unwrap()["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["url"] == "C/Trefoil_knot"));
        server.shutdown().unwrap();
        assert_eq!(fs::read(&index).unwrap(), bytes);
        assert_eq!(fs::metadata(&index).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn serve_validates_title_index_bindings_before_listening() {
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    for index in [
        "bad",
        "=path",
        "../bad=path",
        "en=",
        "unknown=path",
        "map=path",
    ] {
        let mut args = arguments(shell.path(), data.path());
        args.extend([
            "--pack".into(),
            "en=missing.zim".into(),
            "--pack".into(),
            "map=missing.pmtiles".into(),
            "--title-index".into(),
            index.into(),
        ]);
        assert!(
            matches!(serve::start(&args), Err(PackError::Mismatch(_))),
            "{index}"
        );
    }
    let mut args = arguments(shell.path(), data.path());
    args.extend([
        "--pack".into(),
        "en=missing.zim".into(),
        "--title-index".into(),
        "en=first".into(),
        "--title-index".into(),
        "en=second".into(),
    ]);
    assert!(matches!(serve::start(&args), Err(PackError::Mismatch(_))));
}

#[test]
fn serve_missing_or_mismatched_title_index_never_builds_or_replaces_one() {
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    let index = data.path().join("missing.titles");
    let mut args = arguments(shell.path(), data.path());
    args.extend([
        "--pack".into(),
        format!("en={}", zim_path(false).display()),
        "--title-index".into(),
        format!("en={}", index.display()),
    ]);
    assert!(matches!(serve::start(&args), Err(PackError::Io(_))));
    assert!(!index.exists());
    title_table(&index);
    let bytes = fs::read(&index).unwrap();
    let mut args = arguments(shell.path(), data.path());
    args.extend([
        "--pack".into(),
        format!("es={}", zim_path(true).display()),
        "--title-index".into(),
        format!("es={}", index.display()),
    ]);
    assert!(matches!(serve::start(&args), Err(PackError::Mismatch(_))));
    assert_eq!(fs::read(&index).unwrap(), bytes);
}

#[test]
#[ignore = "requires the verified 52GB Wikipedia pack and existing root-built title table"]
fn full_wikipedia_http_paging_reads_real_rows_beyond_100k() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pack = root.join("docs/spike/packs/wikipedia_en_all_nopic_2026-06.zim");
    let index = root.join("target/full-wikipedia.titles.idx");
    let expected = Zim::open(Arc::new(
        PreadSource::open(&pack).expect("download and verify full benchmark pack first"),
    ))
    .unwrap();
    let total = expected.title_count().unwrap();
    assert_eq!(total, 19_191_219);
    let index_metadata = fs::metadata(&index)
        .expect("attach the existing root-built table; never rebuild in this test");
    let shell = web();
    let data = tempfile::tempdir().unwrap();
    let mut args = arguments(shell.path(), data.path());
    args.extend([
        "--pack".into(),
        format!("wiki={}", pack.display()),
        "--title-index".into(),
        format!("wiki={}", index.display()),
    ]);
    let started = std::time::Instant::now();
    let mut server = serve::start(&args).unwrap();
    println!(
        "functional full-corpus startup including existing table verification: {:?}",
        started.elapsed()
    );
    let session = Session::login(&server);
    for offset in [0, 50_000, 99_800, 100_000, total - 200, total - 3] {
        let json = session
            .api("/api/zim/titles")
            .query(&[
                ("pack", "wiki".to_owned()),
                ("offset", offset.to_string()),
                ("limit", "200".to_owned()),
            ])
            .send()
            .unwrap();
        assert_eq!(json.status(), 200);
        let json = json.json::<serde_json::Value>().unwrap();
        assert_eq!(json["pack"], "wiki");
        assert_eq!(json["total"], total);
        assert_eq!(json["offset"], offset);
        let rows = json["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 200.min(total - offset));
        for (index, row) in rows.iter().enumerate() {
            let expected = expected.title_at((offset + index) as u32).unwrap();
            assert_eq!(row["title"], expected.title);
            assert_eq!(row["url"], expected.url);
        }
    }
    server.shutdown().unwrap();
    let after = fs::metadata(&index).unwrap();
    assert_eq!(after.len(), index_metadata.len());
    assert_eq!(
        after.modified().unwrap(),
        index_metadata.modified().unwrap()
    );
}
