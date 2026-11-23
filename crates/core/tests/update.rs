use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use tonalli_core::pack::PackError;
use tonalli_core::update::{Manifest, Version};

// Synthetic test-only keys. Never a publisher key.
fn key(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}

fn asset(platform: &str, url: &str) -> Value {
    json!({"platform": platform, "url": url, "bytes": 1234, "sha256": "ab".repeat(32)})
}

fn payload() -> Value {
    json!({
        "schema_version": 1,
        "version": "0.1.0-preview.3",
        "published": "2026-11-16",
        "notes": "Smaller AI model for older phones.",
        "page": "https://example.org/releases/tag/v0.1.0-preview.3",
        "assets": [
            asset("android-arm64", "https://example.org/Tonalli-Android-arm64.apk"),
            asset("windows-x64", "https://example.org/Tonalli-Windows-x64.zip"),
            asset("ios-arm64", "https://example.org/Tonalli-iOS-arm64.ipa"),
        ],
    })
}

fn signed(value: &Value, pair: &Ed25519KeyPair) -> (Vec<u8>, Vec<u8>) {
    let bytes = serde_json::to_vec(value).unwrap();
    let signature = pair.sign(&bytes).as_ref().to_vec();
    (bytes, signature)
}

fn verify(value: &Value) -> Result<Manifest, PackError> {
    let pair = key(7);
    let (bytes, signature) = signed(value, &pair);
    Manifest::verify(&bytes, &signature, pair.public_key().as_ref())
}

fn v(text: &str) -> Version {
    text.parse().unwrap()
}

#[test]
fn versions_order_like_releases_do() {
    assert!(v("0.1.0-preview.3") > v("0.1.0-preview.2"));
    assert!(v("0.1.0-preview.10") > v("0.1.0-preview.9"));
    assert!(v("0.1.0") > v("0.1.0-preview.9"));
    assert!(v("0.2.0-preview.1") > v("0.1.9"));
    assert!(v("1.0.0") > v("0.99.99"));
    assert_eq!(v("v0.1.0"), v("0.1.0"));
    assert_eq!(v("0.1.0-preview.2"), v("0.1.0-preview.2"));
}

#[test]
fn malformed_versions_are_rejected() {
    for text in [
        "", "1", "1.0", "1.0.x", "1.0.0.0", "-1.0.0", "1.0.0-", "1.0.0-pre..1", "1.0.0 ",
        "99999999999999999999.0.0", "1.0.0-pré",
    ] {
        assert!(text.parse::<Version>().is_err(), "{text:?} must be rejected");
    }
}

#[test]
fn a_correctly_signed_manifest_is_accepted() {
    let manifest = verify(&payload()).unwrap();
    assert_eq!(manifest.version(), v("0.1.0-preview.3"));
    assert_eq!(manifest.notes, "Smaller AI model for older phones.");
    let android = manifest.asset_for("android-arm64").unwrap();
    assert_eq!(android.bytes, 1234);
    assert!(manifest.asset_for("linux-x64").is_none());
    assert!(manifest.is_newer_than(&v("0.1.0-preview.2")));
    assert!(!manifest.is_newer_than(&v("0.1.0-preview.3")));
    assert!(!manifest.is_newer_than(&v("0.1.0")));
}

#[test]
fn a_tampered_or_foreign_signature_is_refused() {
    let pair = key(7);
    let (bytes, signature) = signed(&payload(), &pair);
    let mut tampered = bytes.clone();
    let at = tampered.iter().position(|b| *b == b'3').unwrap();
    tampered[at] = b'4';
    assert!(matches!(
        Manifest::verify(&tampered, &signature, pair.public_key().as_ref()),
        Err(PackError::Checksum)
    ));
    let stranger = key(9);
    assert!(matches!(
        Manifest::verify(&bytes, &signature, stranger.public_key().as_ref()),
        Err(PackError::Checksum)
    ));
    assert!(Manifest::verify(&bytes, &signature[..63], pair.public_key().as_ref()).is_err());
    assert!(Manifest::verify(&bytes, &signature, &[0; 31]).is_err());
}

#[test]
fn a_signed_manifest_with_unsafe_content_is_still_refused() {
    let mut plain_http = payload();
    plain_http["assets"][0]["url"] = json!("http://example.org/Tonalli.apk");
    assert!(verify(&plain_http).is_err());

    let mut local_file = payload();
    local_file["assets"][0]["url"] = json!("file:///C:/Windows/System32/cmd.exe");
    assert!(verify(&local_file).is_err());

    let mut bad_hash = payload();
    bad_hash["assets"][0]["sha256"] = json!("xyz");
    assert!(verify(&bad_hash).is_err());

    let mut empty = payload();
    empty["assets"][0]["bytes"] = json!(0);
    assert!(verify(&empty).is_err());

    let mut huge = payload();
    huge["assets"][0]["bytes"] = json!(u64::MAX);
    assert!(verify(&huge).is_err());

    let mut future = payload();
    future["schema_version"] = json!(2);
    assert!(verify(&future).is_err());

    let mut bad_version = payload();
    bad_version["version"] = json!("latest");
    assert!(verify(&bad_version).is_err());

    let mut duplicate = payload();
    duplicate["assets"]
        .as_array_mut()
        .unwrap()
        .push(asset("android-arm64", "https://example.org/other.apk"));
    assert!(verify(&duplicate).is_err());

    let mut unknown_field = payload();
    unknown_field["run"] = json!("cmd /c calc");
    assert!(verify(&unknown_field).is_err());

    let mut oversized_notes = payload();
    oversized_notes["notes"] = json!("x".repeat(20_000));
    assert!(verify(&oversized_notes).is_err());
}

#[test]
fn a_source_is_a_github_repo_or_an_https_folder_and_nothing_else() {
    use tonalli_core::update::{normalize_source, MANIFEST_FILE, SIGNATURE_FILE};
    let github = "https://github.com/example/tonalli/releases/download/update-feed/";
    for input in [
        "example/tonalli",
        "  example/tonalli  ",
        "github.com/example/tonalli",
        "https://github.com/example/tonalli",
        "https://github.com/example/tonalli/",
        "https://github.com/example/tonalli/releases",
        "https://github.com/example/tonalli/releases/tag/v0.1.0",
        "https://github.com/example/tonalli.git",
    ] {
        assert_eq!(normalize_source(input).unwrap(), github, "{input:?}");
    }
    // A GitHub address that names a release tag keeps that tag (a custom or test feed).
    for input in [
        "https://github.com/example/tonalli/releases/download/beta-feed",
        "https://github.com/example/tonalli/releases/download/beta-feed/",
        "https://github.com/example/tonalli/releases/download/beta-feed/update-manifest.json",
    ] {
        assert_eq!(
            normalize_source(input).unwrap(),
            "https://github.com/example/tonalli/releases/download/beta-feed/",
            "{input:?}"
        );
    }
    assert_eq!(
        normalize_source("https://updates.example.org/tonalli").unwrap(),
        "https://updates.example.org/tonalli/"
    );
    assert_eq!(
        normalize_source("https://updates.example.org/tonalli/").unwrap(),
        "https://updates.example.org/tonalli/"
    );
    assert_eq!(MANIFEST_FILE, "update-manifest.json");
    assert_eq!(SIGNATURE_FILE, "update-manifest.sig");
}

#[test]
fn unsafe_sources_are_refused() {
    use tonalli_core::update::normalize_source;
    for input in [
        "",
        "http://updates.example.org/tonalli",
        "ftp://updates.example.org/",
        "file:///C:/updates",
        "https://user:pass@updates.example.org/",
        "https://updates.example.org/tonalli?token=1",
        "https://updates.example.org/tonalli#x",
        "https://localhost/updates",
        "https://127.0.0.1/updates",
        "https://192.168.1.5/updates",
        "https://[::1]/updates",
        "just some words",
        "a/b/c",
        "../../etc",
        "owner/re po",
    ] {
        assert!(normalize_source(input).is_err(), "{input:?} must be refused");
    }
}

#[test]
fn the_trusted_key_must_be_exactly_32_bytes_of_hex() {
    use tonalli_core::update::parse_public_key;
    let pair = key(7);
    let hex: String = pair.public_key().as_ref().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(parse_public_key(&hex).unwrap().as_slice(), pair.public_key().as_ref());
    assert_eq!(
        parse_public_key(&format!("  {}  ", hex.to_uppercase())).unwrap().as_slice(),
        pair.public_key().as_ref()
    );
    for bad in ["", "abcd", &hex[..62], &format!("{hex}00"), &"zz".repeat(32)] {
        assert!(parse_public_key(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn an_oversized_payload_is_refused_before_parsing() {
    let pair = key(7);
    let bytes = vec![b' '; 300 * 1024];
    let signature = pair.sign(&bytes).as_ref().to_vec();
    assert!(Manifest::verify(&bytes, &signature, pair.public_key().as_ref()).is_err());
}
