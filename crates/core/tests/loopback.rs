use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::{COOKIE, LOCATION, SET_COOKIE};
use tempfile::TempDir;
use tonalli_core::server::{LoopbackServer, PackId, PackRegistry};

struct Fixture {
    server: LoopbackServer,
    client: Client,
    web: PathBuf,
    data: PathBuf,
    _root: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let web = root.path().join("web");
        let data = root.path().join("private");
        fs::create_dir_all(web.join("assets")).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::write(
            web.join("index.html"),
            b"<!doctype html><title>Tonalli</title>",
        )
        .unwrap();
        fs::write(web.join("assets/app.js"), b"console.log('Tonalli');").unwrap();
        fs::write(data.join("notes.db"), b"private database bytes").unwrap();
        fs::write(data.join("notes.keys"), b"private key bytes").unwrap();
        let server =
            LoopbackServer::start(Arc::new(PackRegistry::default()), data.clone(), web.clone())
                .unwrap();
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            _root: root,
            server,
            client,
            web,
            data,
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.server.info().port)
    }
    fn launch(&self) -> Response {
        self.client
            .get(&self.server.info().launch_url)
            .send()
            .unwrap()
    }
    fn login(&self) -> Cookies {
        let response = self.launch();
        assert_eq!(response.status(), 302);
        let cookies: Vec<_> = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect();
        Cookies {
            content: cookies
                .iter()
                .find(|value| value.starts_with("tonalli_content="))
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .into(),
            api: cookies
                .iter()
                .find(|value| value.starts_with("tonalli_api="))
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .into(),
        }
    }
    fn csrf(&self, cookies: &Cookies) -> String {
        let response = self
            .client
            .get(self.url("/api/session"))
            .header(COOKIE, &cookies.api)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json::<serde_json::Value>().unwrap()["csrf"]
            .as_str()
            .unwrap()
            .into()
    }
}
struct Cookies {
    content: String,
    api: String,
}

#[test]
fn spa_aliases_serve_only_the_trusted_index_with_query_and_head() {
    let fixture = Fixture::new();
    for path in ["/library", "/article", "/map", "/notes"] {
        assert_eq!(
            fixture
                .client
                .get(fixture.url(path))
                .send()
                .unwrap()
                .status(),
            401
        );
    }
    let cookies = fixture.login();
    let index = fs::read(fixture.web.join("index.html")).unwrap();
    for path in [
        "/library?pack=wiki",
        "/article?pack=wiki&url=C%2FTest",
        "/map?packs=mx,ca",
        "/notes",
    ] {
        let response = fixture
            .client
            .get(fixture.url(path))
            .header(COOKIE, &cookies.content)
            .send()
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        assert!(response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert_eq!(
            response.headers()["content-security-policy"],
            "frame-src 'self'"
        );
        assert!(!response.headers().contains_key(LOCATION));
        assert_eq!(
            response.url().query(),
            path.split_once('?').map(|(_, query)| query)
        );
        assert_eq!(response.bytes().unwrap().as_ref(), index);
        let head = fixture
            .client
            .head(fixture.url(path))
            .header(COOKIE, &cookies.content)
            .send()
            .unwrap();
        assert_eq!(head.status(), 200);
        assert_eq!(head.headers()["content-length"], index.len().to_string());
        assert!(head.bytes().unwrap().is_empty());
    }
}

#[test]
fn spa_aliases_never_swallow_unknown_reserved_or_private_paths() {
    let fixture = Fixture::new();
    let cookies = fixture.login();
    let csrf = fixture.csrf(&cookies);
    for path in [
        "/not-a-route",
        "/library/",
        "/library/anything",
        "/%6cibrary",
        "//library",
        "/LIBRARY",
        "/assets/missing.js",
        "/api/unknown",
        "/zim/missing/C/Article",
        "/packs/missing.pmtiles",
        "/notes.db",
        "/notes.keys",
        "/settings.json",
    ] {
        let response = fixture
            .client
            .get(fixture.url(path))
            .header(COOKIE, format!("{}; {}", cookies.content, cookies.api))
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &csrf)
            .send()
            .unwrap();
        assert_eq!(response.status(), 404, "{path}");
        assert!(!response.text().unwrap().contains("<title>Tonalli</title>"));
    }
}

#[test]
fn launch_with_good_token_sets_cookie_and_redirects() {
    let fixture = Fixture::new();
    let response = fixture.launch();
    assert_eq!(response.status(), 302);
    assert_eq!(response.headers()[LOCATION], "/");
    let values: Vec<_> = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert_eq!(values.len(), 2);
    assert!(values
        .iter()
        .all(|value| value.contains("HttpOnly") && value.contains("SameSite=Strict")));
    assert!(values
        .iter()
        .any(|value| value.starts_with("tonalli_content=") && value.contains("Path=/;")));
    assert!(values
        .iter()
        .any(|value| value.starts_with("tonalli_api=") && value.contains("Path=/api;")));
    assert!(values.iter().all(|value| !value.contains("Domain=")));
    assert_eq!(response.headers()["cache-control"], "no-store");
}

#[test]
fn launch_with_bad_token_is_401() {
    let fixture = Fixture::new();
    for path in ["/launch", "/launch?token=bad"] {
        assert_eq!(
            fixture
                .client
                .get(fixture.url(path))
                .send()
                .unwrap()
                .status(),
            401
        );
    }
}

#[test]
fn route_without_cookie_is_401() {
    let fixture = Fixture::new();
    for path in [
        "/",
        "/assets/app.js",
        "/zim/test/article",
        "/packs/test.pmtiles",
        "/api/session",
    ] {
        assert_eq!(
            fixture
                .client
                .get(fixture.url(path))
                .send()
                .unwrap()
                .status(),
            401,
            "{path}"
        );
    }
    assert_eq!(
        fixture
            .client
            .get(fixture.url(&format!("/?token={}", fixture.server.info().token)))
            .send()
            .unwrap()
            .status(),
        401
    );
}

#[test]
fn api_cookie_cannot_load_zim_content() {
    let fixture = Fixture::new();
    let cookies = fixture.login();
    for path in [
        "/",
        "/assets/app.js",
        "/zim/test/article",
        "/packs/test.pmtiles",
    ] {
        assert_eq!(
            fixture
                .client
                .get(fixture.url(path))
                .header(COOKIE, &cookies.api)
                .send()
                .unwrap()
                .status(),
            401
        );
    }
}

#[test]
fn session_requires_api_cookie_and_empty_destination() {
    let fixture = Fixture::new();
    let cookies = fixture.login();
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/api/session"))
            .header(COOKIE, &cookies.content)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/api/session"))
            .header(COOKIE, &cookies.api)
            .send()
            .unwrap()
            .status(),
        403
    );
    let csrf = fixture.csrf(&cookies);
    assert_eq!(csrf.len(), 64);
}

#[test]
fn cross_site_fetch_metadata_is_403() {
    let fixture = Fixture::new();
    let cookies = fixture.login();
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/"))
            .header(COOKIE, &cookies.content)
            .header("Sec-Fetch-Site", "cross-site")
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        fixture
            .client
            .get(&fixture.server.info().launch_url)
            .header("Sec-Fetch-Site", "cross-site")
            .send()
            .unwrap()
            .status(),
        403
    );
}

#[test]
fn foreign_origin_header_is_403() {
    let fixture = Fixture::new();
    let cookies = fixture.login();
    for origin in ["https://example.com", "null", "http://localhost:1234"] {
        assert_eq!(
            fixture
                .client
                .get(fixture.url("/"))
                .header(COOKIE, &cookies.content)
                .header("Origin", origin)
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/"))
            .header(COOKIE, &cookies.content)
            .header("Origin", fixture.url(""))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/"))
            .header(COOKIE, &cookies.content)
            .header("Host", "example.com")
            .send()
            .unwrap()
            .status(),
        403
    );
}

#[test]
fn launch_exchange_is_single_use_and_rotation_issues_a_new_launch() {
    let fixture = Fixture::new();
    let old = fixture.server.info();
    fixture.login();
    assert_eq!(
        fixture.client.get(&old.launch_url).send().unwrap().status(),
        401
    );
    let fresh = fixture.server.rotate_token().unwrap();
    assert_ne!(fresh.token, old.token);
    assert_eq!(
        fixture.client.get(&old.launch_url).send().unwrap().status(),
        401
    );
    assert_eq!(
        fixture
            .client
            .get(&fresh.launch_url)
            .send()
            .unwrap()
            .status(),
        302
    );
}

#[test]
fn rotate_token_invalidates_existing_cookie() {
    let fixture = Fixture::new();
    let cookies = fixture.login();
    let csrf = fixture.csrf(&cookies);
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/"))
            .header(COOKIE, &cookies.content)
            .send()
            .unwrap()
            .status(),
        200
    );
    fixture.server.rotate_token().unwrap();
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/"))
            .header(COOKIE, &cookies.content)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/api/session"))
            .header(COOKIE, &cookies.api)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", csrf)
            .send()
            .unwrap()
            .status(),
        401
    );
}

#[test]
fn static_assets_are_confined_and_private_files_are_never_routes() {
    let fixture = Fixture::new();
    let cookies = fixture.login();
    let response = fixture
        .client
        .get(fixture.url("/assets/app.js"))
        .header(COOKIE, &cookies.content)
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/javascript");
    assert_eq!(response.text().unwrap(), "console.log('Tonalli');");
    for path in [
        "/notes.db",
        "/notes.keys",
        "/private/notes.db",
        "/api/notes",
        "/api/settings",
        "/assets/%2e%2e%2f%2e%2e%2fprivate/notes.db",
        "/assets/%5c..%5cprivate%5cnotes.db",
        "/assets/app.js:stream",
    ] {
        let response = fixture
            .client
            .get(fixture.url(path))
            .header(COOKIE, format!("{}; {}", cookies.content, cookies.api))
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", fixture.csrf(&cookies))
            .send()
            .unwrap();
        assert!(!response.status().is_success(), "{path}");
        assert!(!response.text().unwrap().contains("private database bytes"));
    }
}

#[test]
fn web_root_must_not_include_private_data_directory() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("index.html"), "app").unwrap();
    assert!(LoopbackServer::start(
        Arc::new(PackRegistry::default()),
        root.path().join("private"),
        root.path().to_owned()
    )
    .is_err());
}

#[test]
fn linked_asset_cannot_escape_the_canonical_web_root() {
    let fixture = Fixture::new();
    fs::write(fixture.data.join("secret.js"), "private JavaScript bytes").unwrap();
    let link = fixture.web.join("linked");
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&fixture.data)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&fixture.data, &link).unwrap();
    let cookies = fixture.login();
    let response = fixture
        .client
        .get(fixture.url("/linked/secret.js"))
        .header(COOKIE, &cookies.content)
        .send()
        .unwrap();
    assert_eq!(response.status(), 404);
    assert!(!response
        .text()
        .unwrap()
        .contains("private JavaScript bytes"));
}

#[test]
fn duplicate_authentication_cookies_fail_closed() {
    let fixture = Fixture::new();
    let cookies = fixture.login();
    for (path, cookie) in [("/", &cookies.content), ("/api/session", &cookies.api)] {
        let response = fixture
            .client
            .get(fixture.url(path))
            .header(COOKIE, format!("{cookie}; {cookie}"))
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap();
        assert_eq!(response.status(), 401);
    }
}

#[test]
fn head_does_not_consume_a_launch_token() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture
            .client
            .head(&fixture.server.info().launch_url)
            .send()
            .unwrap()
            .status(),
        405
    );
    assert_eq!(fixture.launch().status(), 302);
}

#[test]
fn dropping_server_releases_its_listener() {
    let address = {
        let fixture = Fixture::new();
        let address: SocketAddr = format!("127.0.0.1:{}", fixture.server.info().port)
            .parse()
            .unwrap();
        assert!(TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok());
        address
    };
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err());
}

#[test]
fn binds_only_to_127_0_0_1_and_shutdown_releases_the_listener() {
    let mut fixture = Fixture::new();
    let port = fixture.server.info().port;
    let address: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    assert!(TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok());
    fixture.server.shutdown().unwrap();
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err());
}

#[test]
fn pack_ids_reject_paths_and_reserved_characters() {
    for id in [
        "",
        "../notes",
        "pack/name",
        "pack name",
        "pack%2fname",
        "pack:stream",
    ] {
        assert!(PackId::new(id).is_err());
    }
    assert_eq!(
        PackId::new("wikipedia-es_1").unwrap().as_str(),
        "wikipedia-es_1"
    );
}

#[cfg(feature = "test-routes")]
mod api {
    use super::*;
    #[test]
    fn content_cookie_cannot_call_api_routes() {
        let fixture = Fixture::new();
        let cookies = fixture.login();
        assert_eq!(
            fixture
                .client
                .get(fixture.url("/api/ping"))
                .header(COOKIE, &cookies.content)
                .header("Sec-Fetch-Dest", "empty")
                .send()
                .unwrap()
                .status(),
            401
        );
    }
    #[test]
    fn api_call_without_csrf_header_is_403() {
        let fixture = Fixture::new();
        let cookies = fixture.login();
        assert_eq!(
            fixture
                .client
                .get(fixture.url("/api/ping"))
                .header(COOKIE, &cookies.api)
                .header("Sec-Fetch-Dest", "empty")
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    #[test]
    fn api_call_with_image_or_iframe_fetch_dest_is_403() {
        let fixture = Fixture::new();
        let cookies = fixture.login();
        let csrf = fixture.csrf(&cookies);
        for destination in ["image", "iframe", "document", "style", "script"] {
            assert_eq!(
                fixture
                    .client
                    .get(fixture.url("/api/ping"))
                    .header(COOKIE, &cookies.api)
                    .header("Sec-Fetch-Dest", destination)
                    .header("X-Tonalli-Csrf", &csrf)
                    .send()
                    .unwrap()
                    .status(),
                403
            );
        }
        let response = fixture
            .client
            .get(fixture.url("/api/ping"))
            .header(COOKIE, &cookies.api)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &csrf)
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().unwrap(), "ok");
    }
}
