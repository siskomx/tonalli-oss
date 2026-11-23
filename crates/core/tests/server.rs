use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{COOKIE, SET_COOKIE};
use reqwest::Url;
use tempfile::TempDir;
use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::server::{LoopbackServer, PackId, PackRegistry};
use tonalli_core::zim::Zim;

const ZIM_CSP: &str = "default-src 'none'; script-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; media-src 'self'; object-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'";

struct Fixture {
    server: LoopbackServer,
    client: Client,
    zims: Vec<Arc<Zim>>,
    sources: Vec<Arc<ControlledSource>>,
    registry: Arc<PackRegistry>,
    _root: TempDir,
}
struct Credentials {
    content: String,
    api: String,
    csrf: String,
}

#[test]
fn sharing_control_is_private_and_requires_a_native_owner() {
    let fixture = Fixture::new(&[]);
    assert_eq!(
        fixture
            .client
            .get(fixture.url("/api/sharing"))
            .send()
            .unwrap()
            .status(),
        401
    );
    let credentials = fixture.login();
    assert_eq!(
        fixture
            .api("/api/sharing", &credentials)
            .send()
            .unwrap()
            .status(),
        503
    );
}

#[test]
fn sharing_commands_require_csrf_and_close_the_native_owner() {
    use tonalli_core::server::{SharingControl, SharingStatus};
    struct Owner(AtomicUsize);
    impl SharingControl for Owner {
        fn status(&self) -> Result<SharingStatus, PackError> {
            unreachable!()
        }
        fn start(
            &self,
            network: String,
            packs: Vec<String>,
            media: Arc<tonalli_core::server::SharedMedia>,
        ) -> Result<(), PackError> {
            assert_eq!(network, "native-choice");
            assert_eq!(packs, ["installed"]);
            assert!(media.is_empty());
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn stop(&self) -> Result<(), PackError> {
            self.0.fetch_add(10, Ordering::SeqCst);
            Ok(())
        }
        fn kick(&self, _: String) -> Result<(), PackError> {
            unreachable!()
        }
    }
    let mut f = Fixture::new(&[]);
    let owner = Arc::new(Owner(AtomicUsize::new(0)));
    f.server.attach_sharing(owner.clone()).unwrap();
    let c = f.login();
    let request = || {
        f.client
            .post(f.url("/api/sharing/start"))
            .header(COOKIE, &c.api)
            .header("Sec-Fetch-Dest", "empty")
            .json(&serde_json::json!({"network":"native-choice","packs":["installed"]}))
    };
    assert_eq!(request().send().unwrap().status(), 403);
    assert_eq!(owner.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        request()
            .header("X-Tonalli-Csrf", &c.csrf)
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(owner.0.load(Ordering::SeqCst), 1);
    f.server.shutdown().unwrap();
    f.server.shutdown().unwrap();
    assert_eq!(owner.0.load(Ordering::SeqCst), 11);
}

#[test]
fn installed_packs_are_authenticated_sorted_and_hide_unreadable_sources() {
    let f = Fixture::new(&[("zeta", false), ("alpha", true)]);
    let c = f.login();
    assert_eq!(
        f.client.get(f.url("/api/packs")).send().unwrap().status(),
        401
    );
    assert_eq!(f.content("/api/packs", &c).send().unwrap().status(), 401);
    assert_eq!(
        f.client
            .get(f.url("/api/packs"))
            .header(COOKIE, &c.api)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .status(),
        403
    );
    let reply = f.api("/api/packs", &c).send().unwrap();
    assert_eq!(reply.status(), 200);
    let value: serde_json::Value = reply.json().unwrap();
    let packs = value["packs"].as_array().unwrap();
    assert_eq!(packs.len(), 2);
    for (entry, (id, zim)) in packs
        .iter()
        .zip([("alpha", &f.zims[1]), ("zeta", &f.zims[0])])
    {
        assert_eq!(entry["id"], id);
        assert_eq!(entry["total"], zim.title_count().unwrap());
        let title = entry["title"].as_str().unwrap();
        assert!(!title.is_empty() && title.len() <= 512 && !title.chars().any(char::is_control));
        assert_ne!(title, id, "real fixture metadata Title should be shown");
        assert_eq!(
            entry
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["id", "title", "total"])
        );
    }
    f.sources[0].removed.store(true, Ordering::SeqCst);
    let value: serde_json::Value = f.api("/api/packs", &c).send().unwrap().json().unwrap();
    assert_eq!(value["packs"].as_array().unwrap().len(), 1);
    assert_eq!(value["packs"][0]["id"], "alpha");
}

#[test]
fn installed_packs_empty_registry_returns_an_empty_list() {
    let f = Fixture::new(&[]);
    let c = f.login();
    let response = f.api("/api/packs", &c).send().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<serde_json::Value>().unwrap(),
        serde_json::json!({"packs":[]})
    );
}

#[test]
fn session_private_notes_capability_is_true_on_loopback() {
    let f = Fixture::new(&[]);
    let credentials = f.login();
    let response = f.api("/api/session", &credentials).send().unwrap();
    assert_eq!(response.status(), 200);
    let session: serde_json::Value = response.json().unwrap();
    assert_eq!(session["private_notes"], true);
    assert_eq!(session["private_ai"], false);
    assert_eq!(session["csrf"], credentials.csrf);
}

#[cfg(feature = "ai")]
#[test]
fn private_ai_history_is_versioned_and_requires_root_cookie_and_csrf_even_while_ai_is_off() {
    let f = Fixture::new(&[]);
    let c = f.login();
    let path = "/api/ai/history/0123456789abcdef0123456789abcdef";
    let body = serde_json::json!({"revision":0,"title":"Saved conversation","model":"local","pack":null,
        "messages":[{"role":"user","content":"Question","context":true,"timestamp":1,"truncated":false},
                    {"role":"assistant","content":"Answer","context":true,"timestamp":2,"truncated":false}]});
    assert_eq!(
        f.client
            .get(f.url("/api/ai/history"))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        f.content("/api/ai/history", &c).send().unwrap().status(),
        401
    );
    let put = || {
        f.client
            .put(f.url(path))
            .header(COOKIE, &c.api)
            .header("Sec-Fetch-Dest", "empty")
            .json(&body)
    };
    assert_eq!(put().send().unwrap().status(), 403);
    let created = put().header("X-Tonalli-Csrf", &c.csrf).send().unwrap();
    assert_eq!(created.status(), 200);
    assert_eq!(created.json::<serde_json::Value>().unwrap()["revision"], 1);
    assert_eq!(
        put()
            .header("X-Tonalli-Csrf", &c.csrf)
            .send()
            .unwrap()
            .status(),
        409
    );
    let listed: serde_json::Value = f.api("/api/ai/history", &c).send().unwrap().json().unwrap();
    assert_eq!(listed["sessions"][0]["title"], "Saved conversation");
    assert!(
        listed["sessions"][0].get("messages").is_none(),
        "listing does not send transcripts"
    );
    assert_eq!(
        f.api(path, &c)
            .send()
            .unwrap()
            .json::<serde_json::Value>()
            .unwrap()["messages"][1]["content"],
        "Answer"
    );
    let delete = |confirmed| {
        f.client
            .delete(f.url(path))
            .header(COOKIE, &c.api)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &c.csrf)
            .json(&serde_json::json!({"revision":1,"confirmed":confirmed}))
            .send()
            .unwrap()
    };
    assert_eq!(delete(false).status(), 409);
    assert_eq!(delete(true).status(), 200);
    assert_eq!(f.api(path, &c).send().unwrap().status(), 404);
}

#[cfg(feature = "ai")]
#[test]
fn private_ai_transport_is_explicit_single_turn_and_off_without_attachment() {
    let f = Fixture::new(&[]);
    let c = f.login();
    let controls = f.api("/api/ai/models", &c).send().unwrap();
    assert_eq!(
        controls.status(),
        200,
        "compiled optional AI has private model controls while off"
    );
    let controls: serde_json::Value = controls.json().unwrap();
    assert_eq!(controls["state"], "off");
    assert_eq!(controls["model"], serde_json::Value::Null);
    assert_eq!(
        f.api("/api/session", &c)
            .send()
            .unwrap()
            .json::<serde_json::Value>()
            .unwrap()["private_ai_management"],
        true
    );
    for path in ["/api/ai/models/load", "/api/ai/models/off"] {
        assert_eq!(
            f.client
                .post(f.url(path))
                .header(COOKIE, &c.api)
                .header("Sec-Fetch-Dest", "empty")
                .json(&serde_json::json!({}))
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        f.client
            .get(f.url("/api/ai/models"))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        notes_post(&f, "/api/ai/models/off", &c, serde_json::json!({})).status(),
        200
    );
    assert_eq!(
        notes_post(
            &f,
            "/api/ai/models/load",
            &c,
            serde_json::json!({"model":"../model"})
        )
        .status(),
        400
    );
    let model = f.api("/api/ai/model", &c).send().unwrap();
    assert_eq!(model.status(), 503);
    assert_eq!(
        model.json::<serde_json::Value>().unwrap()["error"],
        "ai_unavailable"
    );
    let valid = serde_json::json!({"request_id":"turn-1", "model":"qwen17",
        "messages":[{"role":"user","content":"Hello"}], "max_tokens":128});
    let unavailable = notes_post(&f, "/api/ai/chat", &c, valid.clone());
    assert_eq!(unavailable.status(), 503);
    let mut grounded = valid.clone();
    grounded["pack"] = serde_json::json!("installed-book");
    assert_eq!(
        notes_post(&f, "/api/ai/chat", &c, grounded.clone()).status(),
        503
    );
    assert_eq!(
        f.client
            .post(f.url("/api/ai/chat"))
            .json(&grounded)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        f.client
            .post(f.url("/api/ai/chat"))
            .header(COOKIE, &c.api)
            .header("Sec-Fetch-Dest", "empty")
            .json(&grounded)
            .send()
            .unwrap()
            .status(),
        403
    );
    grounded["pack"] = serde_json::json!("../book");
    assert_eq!(notes_post(&f, "/api/ai/chat", &c, grounded).status(), 400);
    for change in [
        serde_json::json!({"messages":[{"role":"system","content":"Hello"}]}),
        serde_json::json!({"messages":[{"role":"user","content":"one"},{"role":"user","content":"two"}]}),
        serde_json::json!({"max_tokens":129}),
        serde_json::json!({"max_tokens":0}),
    ] {
        let mut body = valid.clone();
        for (key, value) in change.as_object().unwrap() {
            body[key] = value.clone();
        }
        let refused = notes_post(&f, "/api/ai/chat", &c, body);
        assert_eq!(refused.status(), 400);
        assert_eq!(
            refused.json::<serde_json::Value>().unwrap()["error"],
            "ai_invalid_request"
        );
    }
    assert_eq!(
        notes_post(
            &f,
            "/api/ai/cancel",
            &c,
            serde_json::json!({"request_id":"old-turn"})
        )
        .status(),
        404
    );
    assert_eq!(
        f.client
            .get(f.url("/api/ai/model"))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(f.content("/chat", &c).send().unwrap().status(), 200);
}

fn notes_post(
    f: &Fixture,
    path: &str,
    c: &Credentials,
    body: serde_json::Value,
) -> reqwest::blocking::Response {
    f.client
        .post(f.url(path))
        .header(COOKIE, &c.api)
        .header("Sec-Fetch-Dest", "empty")
        .header("X-Tonalli-Csrf", &c.csrf)
        .json(&body)
        .send()
        .unwrap()
}

#[test]
fn notes_locked_and_credential_guards_precede_vault_access() {
    let f = Fixture::new(&[]);
    let c = f.login();
    for path in ["/api/notes", "/api/notes/1", "/api/notes/search?q=x"] {
        assert_eq!(f.client.get(f.url(path)).send().unwrap().status(), 401);
        assert_eq!(f.content(path, &c).send().unwrap().status(), 401);
        assert_eq!(f.api(path, &c).send().unwrap().status(), 423);
    }
    assert_eq!(
        notes_post(
            &f,
            "/api/notes",
            &c,
            serde_json::json!({"title":"x","body":"y"})
        )
        .status(),
        423
    );
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"x"})
        )
        .status(),
        409
    );
    assert!(!f._root.path().join("private/notes.db").exists());
}

#[test]
fn notes_first_use_creates_encrypted_vault_and_never_replaces_it() {
    let f = Fixture::new(&[]);
    let c = f.login();
    let path = f._root.path().join("private/notes.db");
    assert_eq!(
        f.client
            .post(f.url("/api/notes/create"))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert!(!path.exists());
    let created = notes_post(
        &f,
        "/api/notes/create",
        &c,
        serde_json::json!({"passphrase":"consumer-test-only"}),
    );
    assert_eq!(created.status(), 201);
    let value: serde_json::Value = created.json().unwrap();
    let key = value["recovery_key"].as_str().unwrap();
    assert_eq!(key.len(), 64);
    let bytes = fs::read(&path).unwrap();
    assert!(!bytes.starts_with(b"SQLite format 3"));
    assert_eq!(
        notes_post(&f, "/api/notes/lock", &c, serde_json::json!({})).status(),
        204
    );
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/create",
            &c,
            serde_json::json!({"passphrase":"replacement"})
        )
        .status(),
        409
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"wrong"})
        )
        .status(),
        401
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"recovery_key":key})
        )
        .status(),
        200
    );
}

#[test]
fn notes_http_roundtrip_wrong_key_lock_and_rotation() {
    use tonalli_core::notes::Notes;
    let f = Fixture::new(&[]);
    let path = f._root.path().join("private/notes.db");
    let (notes, recovery) = Notes::create(&path, "correct").unwrap();
    drop(notes);
    let snapshot = || {
        (
            fs::read(&path).unwrap(),
            fs::read(path.with_extension("keys")).unwrap(),
        )
    };
    let before = snapshot();
    let c = f.login();
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"wrong"})
        )
        .status(),
        401
    );
    assert_eq!(snapshot(), before);
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"correct"})
        )
        .status(),
        200
    );
    let added = notes_post(
        &f,
        "/api/notes",
        &c,
        serde_json::json!({"title":"sentinel","body":"private sentinel"}),
    );
    assert_eq!(added.status(), 201);
    let id = added.json::<serde_json::Value>().unwrap()["id"]
        .as_i64()
        .unwrap();
    let read = f.api(&format!("/api/notes/{id}"), &c).send().unwrap();
    assert_eq!(read.headers()["cache-control"], "no-store");
    assert_eq!(
        read.json::<serde_json::Value>().unwrap()["body"],
        "private sentinel"
    );
    assert_eq!(
        f.api("/api/notes/invalid", &c).send().unwrap().status(),
        400
    );
    assert_eq!(f.api("/api/notes/999999", &c).send().unwrap().status(), 404);
    assert_eq!(
        notes_post(
            &f,
            "/api/notes",
            &c,
            serde_json::json!({"title":"x".repeat(4097),"body":"small"})
        )
        .status(),
        413
    );
    assert_eq!(
        f.api("/api/notes/search?q=sentinel", &c)
            .send()
            .unwrap()
            .json::<serde_json::Value>()
            .unwrap()["notes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        f.client
            .post(f.url("/api/notes/lock"))
            .header(COOKIE, &c.api)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &c.csrf)
            .send()
            .unwrap()
            .status(),
        204
    );
    assert_eq!(f.api("/api/notes", &c).send().unwrap().status(), 423);
    fs::remove_file(path.with_extension("keys")).unwrap();
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"correct"})
        )
        .status(),
        409
    );
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"recovery_key": recovery.to_string()})
        )
        .status(),
        200
    );
    f.server.rotate_token().unwrap();
    let fresh = f.login();
    assert_eq!(f.api("/api/notes", &fresh).send().unwrap().status(), 423);
}

#[test]
fn consumer_notes_manage_passphrase_export_recovery_and_edit_without_reset() {
    let f = Fixture::new(&[]);
    let c = f.login();
    let created: serde_json::Value = notes_post(
        &f,
        "/api/notes/create",
        &c,
        serde_json::json!({"passphrase":"original-fixture"}),
    )
    .json()
    .unwrap();
    let recovery = created["recovery_key"].as_str().unwrap();
    let path = f._root.path().join("private/notes.db");
    let snapshot = || {
        (
            fs::read(&path).unwrap(),
            fs::read(path.with_extension("keys")).unwrap(),
        )
    };
    let before = snapshot();
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/recovery",
            &c,
            serde_json::json!({"passphrase":"wrong"})
        )
        .status(),
        401
    );
    assert_eq!(snapshot(), before);
    let exported = notes_post(
        &f,
        "/api/notes/recovery",
        &c,
        serde_json::json!({"passphrase":"original-fixture"}),
    );
    assert_eq!(exported.status(), 200);
    assert_eq!(
        exported.json::<serde_json::Value>().unwrap()["recovery_key"],
        recovery
    );
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/passphrase",
            &c,
            serde_json::json!({"passphrase":"wrong","new_passphrase":"changed-fixture"})
        )
        .status(),
        401
    );
    assert_eq!(snapshot(), before);
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/passphrase",
            &c,
            serde_json::json!({"passphrase":"original-fixture","new_passphrase":"changed-fixture"})
        )
        .status(),
        204
    );
    assert_eq!(snapshot().0, before.0);
    assert_ne!(snapshot().1, before.1);
    let changed = snapshot();
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/passphrase",
            &c,
            serde_json::json!({"recovery_key":"0".repeat(64),"new_passphrase":"recovered-fixture"})
        )
        .status(),
        401
    );
    assert_eq!(snapshot(), changed);
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/passphrase",
            &c,
            serde_json::json!({"recovery_key":recovery,"new_passphrase":"recovered-fixture"})
        )
        .status(),
        204
    );
    let added: serde_json::Value = notes_post(
        &f,
        "/api/notes",
        &c,
        serde_json::json!({"title":"Before","body":"Encrypted text"}),
    )
    .json()
    .unwrap();
    let id = added["id"].as_i64().unwrap();
    let request = |method| {
        f.client
            .request(method, f.url(&format!("/api/notes/{id}")))
            .header(COOKIE, &c.api)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &c.csrf)
    };
    assert_eq!(
        request(reqwest::Method::PUT)
            .json(&serde_json::json!({"title":"Después","body":"Still encrypted"}))
            .send()
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        f.api(&format!("/api/notes/{id}"), &c)
            .send()
            .unwrap()
            .json::<serde_json::Value>()
            .unwrap()["title"],
        "Después"
    );
    assert_eq!(
        notes_post(&f, "/api/notes/lock", &c, serde_json::json!({})).status(),
        204
    );
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"changed-fixture"})
        )
        .status(),
        401
    );
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"recovered-fixture"})
        )
        .status(),
        200
    );
    assert_eq!(
        request(reqwest::Method::DELETE).send().unwrap().status(),
        204
    );
    assert_eq!(
        f.api(&format!("/api/notes/{id}"), &c)
            .send()
            .unwrap()
            .status(),
        404
    );
}

#[test]
fn every_notes_method_requires_full_credentials_and_redacts_invalid_inputs() {
    let f = Fixture::new(&[]);
    let c = f.login();
    for (method, path) in [
        (reqwest::Method::GET, "/api/map-management"),
        (reqwest::Method::POST, "/api/map-management"),
        (reqwest::Method::POST, "/api/map-management/mx/pause"),
        (reqwest::Method::POST, "/api/notes/recovery"),
        (reqwest::Method::POST, "/api/notes/passphrase"),
        (reqwest::Method::PUT, "/api/notes/1"),
        (reqwest::Method::DELETE, "/api/notes/1"),
        (reqwest::Method::POST, "/api/notes/create"),
        (reqwest::Method::POST, "/api/notes/unlock"),
        (reqwest::Method::POST, "/api/notes/lock"),
        (reqwest::Method::GET, "/api/notes"),
        (reqwest::Method::POST, "/api/notes"),
        (reqwest::Method::GET, "/api/notes/1"),
        (reqwest::Method::GET, "/api/notes/search?q=x"),
    ] {
        for cookie in [None, Some(&c.content)] {
            let mut request = f.client.request(method.clone(), f.url(path));
            if let Some(cookie) = cookie {
                request = request.header(COOKIE, cookie);
            }
            assert_eq!(request.send().unwrap().status(), 401, "{path}");
        }
        for (csrf, dest) in [(None, "empty"), (Some(c.csrf.as_str()), "iframe")] {
            let mut request = f
                .client
                .request(method.clone(), f.url(path))
                .header(COOKIE, &c.api)
                .header("Sec-Fetch-Dest", dest);
            if let Some(csrf) = csrf {
                request = request.header("X-Tonalli-Csrf", csrf);
            }
            assert_eq!(request.send().unwrap().status(), 403, "{path}");
        }
        assert_eq!(
            f.client
                .request(method, f.url(path))
                .header(COOKIE, &c.api)
                .header("Sec-Fetch-Dest", "empty")
                .header("X-Tonalli-Csrf", &c.csrf)
                .header("Origin", "https://untrusted.invalid")
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    for input in [
        serde_json::json!({}),
        serde_json::json!({"passphrase":"DO_NOT_ECHO","recovery_key":"DO_NOT_ECHO"}),
        serde_json::json!({"passphrase":"DO_NOT_ECHO","path":"DO_NOT_ECHO"}),
    ] {
        let response = notes_post(&f, "/api/notes/unlock", &c, input);
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.json::<serde_json::Value>().unwrap(),
            serde_json::json!({"error":"invalid_request"})
        );
    }
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"x".repeat(4097)})
        )
        .status(),
        413
    );
    assert_eq!(
        notes_post(
            &f,
            "/api/notes/unlock",
            &c,
            serde_json::json!({"passphrase":"x".repeat(1024*1024)})
        )
        .status(),
        413
    );
}
impl Fixture {
    fn new(packs: &[(&str, bool)]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let web = root.path().join("web");
        fs::create_dir(&web).unwrap();
        fs::write(
            web.join("index.html"),
            "<!doctype html><title>Reader tests</title>",
        )
        .unwrap();
        let registry = Arc::new(PackRegistry::default());
        let mut zims = Vec::new();
        let mut sources = Vec::new();
        for &(id, es) in packs {
            let name = if es {
                "wikipedia_es_climate-change_maxi_2026-10.zim"
            } else {
                "wikipedia_en_knots_maxi_2026-10.zim"
            };
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/spike/packs")
                .join(name);
            let source = Arc::new(ControlledSource {
                inner: Arc::new(PreadSource::open(&path).expect("fetch Task1 small ZIM fixtures")),
                removed: AtomicBool::new(false),
            });
            let zim = Arc::new(Zim::open(source.clone()).unwrap());
            registry
                .register_zim(PackId::new(id).unwrap(), zim.clone())
                .unwrap();
            sources.push(source);
            zims.push(zim);
        }
        let server =
            LoopbackServer::start(Arc::clone(&registry), root.path().join("private"), web).unwrap();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        Self {
            server,
            client,
            zims,
            sources,
            registry,
            _root: root,
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.server.info().port)
    }
    fn login(&self) -> Credentials {
        let response = self
            .client
            .get(self.server.info().launch_url)
            .send()
            .unwrap();
        assert_eq!(response.status(), 302);
        let values: Vec<_> = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
            .collect();
        let content = values
            .iter()
            .find(|v| v.starts_with("tonalli_content="))
            .unwrap()
            .clone();
        let api = values
            .iter()
            .find(|v| v.starts_with("tonalli_api="))
            .unwrap()
            .clone();
        let session = self
            .client
            .get(self.url("/api/session"))
            .header(COOKIE, &api)
            .header("Sec-Fetch-Dest", "empty")
            .send()
            .unwrap()
            .json::<serde_json::Value>()
            .unwrap();
        Credentials {
            content,
            api,
            csrf: session["csrf"].as_str().unwrap().into(),
        }
    }
    fn api(&self, path: &str, credentials: &Credentials) -> RequestBuilder {
        self.client
            .get(self.url(path))
            .header(COOKIE, &credentials.api)
            .header("Sec-Fetch-Dest", "empty")
            .header("X-Tonalli-Csrf", &credentials.csrf)
    }
    fn content(&self, path: &str, credentials: &Credentials) -> RequestBuilder {
        self.client
            .get(self.url(path))
            .header(COOKIE, &credentials.content)
    }
}
struct ControlledSource {
    inner: Arc<dyn PackSource>,
    removed: AtomicBool,
}
impl PackSource for ControlledSource {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<usize, PackError> {
        if self.removed.load(Ordering::SeqCst) {
            return Err(PackError::MediaRemoved);
        }
        self.inner.read_at(offset, bytes)
    }
}

#[test]
fn titles_are_paged_in_archive_order_with_total_and_limits() {
    let fixture = Fixture::new(&[("en", false)]);
    let credentials = fixture.login();
    let total = fixture.zims[0].title_count().unwrap();
    let json = fixture
        .api("/api/zim/titles?offset=1&limit=3", &credentials)
        .send()
        .unwrap();
    assert_eq!(json.status(), 200);
    let json = json.json::<serde_json::Value>().unwrap();
    assert_eq!(json["pack"], "en");
    assert_eq!(json["total"], total);
    assert_eq!(json["offset"], 1);
    assert_eq!(json["rows"].as_array().unwrap().len(), 3);
    for position in 1..4 {
        let expected = fixture.zims[0].title_at(position).unwrap();
        assert_eq!(json["rows"][position as usize - 1]["title"], expected.title);
        assert_eq!(json["rows"][position as usize - 1]["url"], expected.url);
    }
    for query in ["", "?limit=999999"] {
        let json = fixture
            .api(&format!("/api/zim/titles{query}"), &credentials)
            .send()
            .unwrap()
            .json::<serde_json::Value>()
            .unwrap();
        assert_eq!(json["rows"].as_array().unwrap().len(), total.min(200));
    }
    for query in ["?limit=0", "?offset=18446744073709551615&limit=200"] {
        let json = fixture
            .api(&format!("/api/zim/titles{query}"), &credentials)
            .send()
            .unwrap()
            .json::<serde_json::Value>()
            .unwrap();
        assert_eq!(json["total"], total);
        assert!(json["rows"].as_array().unwrap().is_empty());
    }
}

#[test]
fn suggest_returns_raw_urls_and_selected_pack() {
    let fixture = Fixture::new(&[("en", false), ("es", true)]);
    let credentials = fixture.login();
    for (pack, q, expected) in [
        ("en", "Trefoil knot", "C/Trefoil_knot"),
        ("es", "Energía renovable", "C/Energía_renovable"),
    ] {
        let response = fixture
            .api("/api/zim/suggest", &credentials)
            .query(&[("pack", pack), ("q", q), ("limit", "2")])
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        let json = response.json::<serde_json::Value>().unwrap();
        assert_eq!(json["pack"], pack);
        assert!(
            json["hits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["url"] == expected),
            "{pack}: {json}"
        );
        assert!(json["hits"].as_array().unwrap().len() <= 2);
    }
    let json = fixture
        .api("/api/zim/suggest?pack=en&q=&limit=0", &credentials)
        .send()
        .unwrap()
        .json::<serde_json::Value>()
        .unwrap();
    assert!(json["hits"].as_array().unwrap().is_empty());
}

#[test]
fn selection_and_query_errors_are_typed_http_errors() {
    for packs in [vec![], vec![("en", false), ("es", true)]] {
        let fixture = Fixture::new(&packs);
        let credentials = fixture.login();
        for path in [
            "/api/zim/titles",
            "/api/zim/suggest?q=Trefoil",
            "/api/zim/article?url=C/Trefoil_knot",
        ] {
            assert_eq!(
                fixture.api(path, &credentials).send().unwrap().status(),
                400
            );
        }
    }
    let fixture = Fixture::new(&[("en", false)]);
    let credentials = fixture.login();
    for path in [
        "/api/zim/titles?pack=missing",
        "/api/zim/article?url=C/not-an-entry",
    ] {
        assert_eq!(
            fixture.api(path, &credentials).send().unwrap().status(),
            404
        );
    }
    for path in [
        "/api/zim/titles?offset=-1",
        "/api/zim/titles?limit=oops",
        "/api/zim/titles?limit=18446744073709551616",
        "/api/zim/article",
        "/api/zim/article?url=",
        "/api/zim/titles?pack=../en",
    ] {
        assert_eq!(
            fixture.api(path, &credentials).send().unwrap().status(),
            400,
            "{path}"
        );
    }
    let q = "x".repeat(4097);
    assert_eq!(
        fixture
            .api("/api/zim/suggest", &credentials)
            .query(&[("q", &q)])
            .send()
            .unwrap()
            .status(),
        400
    );
}

#[test]
fn article_api_returns_canonical_metadata_without_body() {
    let fixture = Fixture::new(&[("en", false)]);
    let credentials = fixture.login();
    let response = fixture
        .api("/api/zim/article", &credentials)
        .query(&[("url", "C/(2,3)-torus_knot")])
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let json = response.json::<serde_json::Value>().unwrap();
    assert_eq!(json["pack"], "en");
    assert_eq!(json["title"], "Trefoil knot");
    assert_eq!(json["url"], "C/Trefoil_knot");
    assert_eq!(json["mime_type"], "text/html");
    assert_eq!(json["iframe_url"], "/zim/en/C/Trefoil_knot");
    assert!(json.get("bytes").is_none() && json.get("html").is_none());
}

#[test]
fn redirect_serves_final_article() {
    let fixture = Fixture::new(&[("en", false)]);
    let credentials = fixture.login();
    let response = fixture
        .content("/zim/en/C/(2,3)-torus_knot", &credentials)
        .send()
        .unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(response.headers()["location"], "/zim/en/C/Trefoil_knot");
    let response = fixture
        .content("/zim/en/C/Trefoil_knot", &credentials)
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/html");
    assert!(response.text().unwrap().contains("Trefoil"));
}

#[test]
fn spanish_article_with_ene_and_accents_in_path_resolves() {
    let fixture = Fixture::new(&[("es", true)]);
    let credentials = fixture.login();
    let response = fixture
        .api("/api/zim/article", &credentials)
        .query(&[("url", "C/Energía_renovable")])
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let json = response.json::<serde_json::Value>().unwrap();
    assert_eq!(json["url"], "C/Energía_renovable");
    assert_eq!(json["iframe_url"], "/zim/es/C/Energ%C3%ADa_renovable");
    let page = fixture
        .content(json["iframe_url"].as_str().unwrap(), &credentials)
        .send()
        .unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().unwrap().contains("Energía"));
}

#[test]
fn article_page_subresources_all_resolve() {
    let fixture = Fixture::new(&[("en", false), ("es", true)]);
    let credentials = fixture.login();
    for (index, pack, path) in [
        (0, "en", "C/Trefoil_knot"),
        (1, "es", "C/Energía_renovable"),
    ] {
        let page_url = Url::parse(&fixture.url(&format!("/zim/{pack}/{path}"))).unwrap();
        let response = fixture
            .client
            .get(page_url.clone())
            .header(COOKIE, &credentials.content)
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        let html = response.text().unwrap();
        let mut resources = BTreeSet::new();
        for reference in references(&html) {
            let Ok(url) = page_url.join(&reference) else {
                continue;
            };
            if url.origin() != page_url.origin() || reference.starts_with('#') {
                continue;
            }
            resources.insert(url.to_string());
        }
        assert!(!resources.is_empty());
        for url in resources {
            let url = Url::parse(&url).unwrap();
            let mount = format!("/zim/{pack}/");
            assert!(
                url.path().starts_with(&mount),
                "resource leaves its pack: {url}"
            );
            let entry_url =
                percent_encoding::percent_decode_str(url.path().strip_prefix(&mount).unwrap())
                    .decode_utf8()
                    .unwrap();
            let expected = fixture.zims[index].article(&entry_url).unwrap();
            let response = fixture
                .client
                .get(url.clone())
                .header(COOKIE, &credentials.content)
                .send()
                .unwrap();
            let response = if response.status() == 302 {
                let location = response.headers()["location"].to_str().unwrap();
                fixture.content(location, &credentials).send().unwrap()
            } else {
                response
            };
            assert_eq!(response.status(), 200, "{url}");
            assert_eq!(response.headers()["content-type"], expected.mime_type);
            assert_eq!(response.bytes().unwrap().as_ref(), expected.bytes);
        }
    }
}

#[derive(Default)]
struct References(RefCell<Vec<String>>);
impl TokenSink for References {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        if let Token::TagToken(tag) = token {
            if tag.kind == TagKind::StartTag {
                let raw = match tag.name.as_ref() {
                    "script" => Some(RawKind::ScriptData),
                    "style" => Some(RawKind::Rawtext),
                    _ => None,
                };
                for attribute in tag.attrs {
                    if matches!(attribute.name.local.as_ref(), "src" | "href") {
                        self.0.borrow_mut().push(attribute.value.to_string());
                    }
                }
                if let Some(raw) = raw {
                    return TokenSinkResult::RawData(raw);
                }
            }
        }
        TokenSinkResult::Continue
    }
}
fn references(html: &str) -> Vec<String> {
    let tokenizer = Tokenizer::new(References::default(), TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(html.into());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.0.into_inner()
}

#[test]
fn zim_responses_carry_csp_no_scripts_header() {
    let fixture = Fixture::new(&[("en", false)]);
    let credentials = fixture.login();
    for (path, cookie) in [
        ("/zim/en/C/Trefoil_knot", Some(&credentials.content)),
        ("/zim/en/C/(2,3)-torus_knot", Some(&credentials.content)),
        ("/zim/en/not-found", Some(&credentials.content)),
        ("/zim/en/C/Trefoil_knot", None),
    ] {
        let mut request = fixture.client.get(fixture.url(path));
        if let Some(cookie) = cookie {
            request = request.header(COOKIE, cookie);
        }
        let response = request.send().unwrap();
        assert_eq!(
            response.headers().get("content-security-policy").unwrap(),
            ZIM_CSP
        );
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }
}

#[test]
fn trusted_parent_html_blocks_external_iframe_navigation() {
    let fixture = Fixture::new(&[]);
    let credentials = fixture.login();
    let response = fixture.content("/", &credentials).send().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-security-policy").unwrap(),
        "frame-src 'self'"
    );
}

#[test]
fn reader_apis_require_full_api_credentials_and_content_uses_content_cookie() {
    let fixture = Fixture::new(&[("en", false)]);
    let credentials = fixture.login();
    for path in [
        "/api/zim/titles",
        "/api/zim/suggest?q=Trefoil",
        "/api/zim/article?url=C/Trefoil_knot",
    ] {
        for cookie in [None, Some(&credentials.content)] {
            let mut request = fixture.client.get(fixture.url(path));
            if let Some(cookie) = cookie {
                request = request.header(COOKIE, cookie);
            }
            assert_eq!(request.send().unwrap().status(), 401);
        }
        assert_eq!(
            fixture
                .client
                .get(fixture.url(path))
                .header(COOKIE, &credentials.api)
                .header("Sec-Fetch-Dest", "empty")
                .send()
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            fixture
                .client
                .get(fixture.url(path))
                .header(COOKIE, &credentials.api)
                .header("X-Tonalli-Csrf", &credentials.csrf)
                .header("Sec-Fetch-Dest", "iframe")
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    for cookie in [None, Some(&credentials.api)] {
        let mut request = fixture.client.get(fixture.url("/zim/en/C/Trefoil_knot"));
        if let Some(cookie) = cookie {
            request = request.header(COOKIE, cookie);
        }
        assert_eq!(request.send().unwrap().status(), 401);
    }
}

#[test]
fn revoked_credential_is_401_on_next_request() {
    let fixture = Fixture::new(&[("en", false)]);
    let credentials = fixture.login();
    fixture.server.rotate_token().unwrap();
    assert_eq!(
        fixture
            .content("/zim/en/C/Trefoil_knot", &credentials)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        fixture
            .api("/api/zim/titles", &credentials)
            .send()
            .unwrap()
            .status(),
        401
    );
}

#[test]
fn removed_zim_source_returns_http_error_without_panicking() {
    let fixture = Fixture::new(&[("en", false)]);
    let credentials = fixture.login();
    fixture.sources[0].removed.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture
            .api("/api/zim/titles", &credentials)
            .send()
            .unwrap()
            .status(),
        503
    );
    assert_eq!(
        fixture
            .content("/zim/en/C/Trefoil_knot", &credentials)
            .send()
            .unwrap()
            .status(),
        503
    );
}

#[test]
fn redirect_and_article_metadata_do_not_read_target_body() {
    let fixture = Fixture::new(&[]);
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let probe = &manifest["packs"][0]["body_probe"];
    let start = probe["blocked_offset"].as_u64().unwrap();
    let source = Arc::new(ProbeSource {
        inner: PreadSource::open(&base.join("benign-v1.zim")).unwrap(),
        start,
        end: start + probe["blocked_bytes"].as_u64().unwrap(),
        enabled: AtomicBool::new(false),
        blocked_reads: AtomicUsize::new(0),
    });
    let zim = Arc::new(Zim::open(source.clone()).unwrap());
    fixture
        .registry
        .register_zim(PackId::new("benign").unwrap(), zim)
        .unwrap();
    source.enabled.store(true, Ordering::SeqCst);
    let credentials = fixture.login();
    let redirect = fixture
        .content("/zim/benign/C/redirect/body-probe", &credentials)
        .send()
        .unwrap();
    assert_eq!(redirect.status(), 302);
    assert_eq!(
        redirect.headers()["location"],
        "/zim/benign/C/assets/body-probe.bin"
    );
    let metadata = fixture
        .api("/api/zim/article?url=C/redirect/body-probe", &credentials)
        .send()
        .unwrap();
    assert_eq!(metadata.status(), 200);
    assert_eq!(
        metadata.json::<serde_json::Value>().unwrap()["url"],
        "C/assets/body-probe.bin"
    );
    assert_eq!(source.blocked_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .content("/zim/benign/C/assets/body-probe.bin", &credentials)
            .send()
            .unwrap()
            .status(),
        503
    );
    assert!(source.blocked_reads.load(Ordering::SeqCst) > 0);
}

#[test]
fn authored_spanish_ene_path_and_internal_link_resolve() {
    let fixture = Fixture::new(&[]);
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim");
    fixture
        .registry
        .register_zim(
            PackId::new("benign").unwrap(),
            Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap()),
        )
        .unwrap();
    let credentials = fixture.login();
    let response = fixture
        .api("/api/zim/article", &credentials)
        .query(&[("url", "C/es/Niñez_y_educación.html")])
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let json = response.json::<serde_json::Value>().unwrap();
    assert_eq!(
        json["iframe_url"],
        "/zim/benign/C/es/Ni%C3%B1ez_y_educaci%C3%B3n.html"
    );
    let url = Url::parse(&fixture.url(json["iframe_url"].as_str().unwrap())).unwrap();
    let page = fixture
        .client
        .get(url.clone())
        .header(COOKIE, &credentials.content)
        .send()
        .unwrap();
    assert_eq!(page.status(), 200);
    let html = page.text().unwrap();
    let reference = references(&html)
        .into_iter()
        .find(|reference| reference.contains("en/index.html"))
        .expect("authored internal article link");
    let link = url.join(&reference).unwrap();
    let response = fixture
        .client
        .get(link)
        .header(COOKIE, &credentials.content)
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.text().unwrap().contains("English"));
}

struct ProbeSource {
    inner: PreadSource,
    start: u64,
    end: u64,
    enabled: AtomicBool,
    blocked_reads: AtomicUsize,
}
impl PackSource for ProbeSource {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<usize, PackError> {
        if self.enabled.load(Ordering::SeqCst)
            && offset < self.end
            && offset.saturating_add(bytes.len() as u64) > self.start
        {
            self.blocked_reads.fetch_add(1, Ordering::SeqCst);
            return Err(PackError::MediaRemoved);
        }
        self.inner.read_at(offset, bytes)
    }
}
