use url::Url;

pub fn allows(port: u16, url: &Url) -> bool {
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(port)
        && url.username().is_empty()
        && url.password().is_none()
}

pub fn native_action(port: u16, url: &Url) -> Option<&str> {
    if !allows(port, url) || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    match url.path() {
        "/native/open" => Some("open"),
        "/native/open-map" => Some("open-map"),
        "/native/advanced" => Some("advanced"),
        "/native/apply-update" => Some("apply-update"),
        path if media_export_id(path).is_some() => Some("export-media"),
        _ => None,
    }
}

pub fn media_export_id(path: &str) -> Option<&str> {
    path.strip_prefix("/native/export-media/").filter(|id| {
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_actions_require_exact_owned_urls() {
        let path = format!("/native/export-media/{}", "a".repeat(32));
        assert_eq!(
            native_action(
                42131,
                &Url::parse(&format!("http://127.0.0.1:42131{path}")).unwrap()
            ),
            Some("export-media")
        );
        for url in [
            format!("http://127.0.0.1:42132{path}"),
            format!("http://user@127.0.0.1:42131{path}"),
            format!("http://127.0.0.1:42131{path}?path=notes"),
            format!("http://127.0.0.1:42131{path}#x"),
        ] {
            assert_eq!(native_action(42131, &Url::parse(&url).unwrap()), None);
        }
        for path in [
            "/native/export-media/../notes",
            "/native/export-media/short",
            "/native/export-media/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert_eq!(media_export_id(path), None);
        }
        for address in [
            "http://127.0.0.1:42132/native/open",
            "http://user@127.0.0.1:42131/native/open",
            "https://example.org/native/open",
            "http://127.0.0.1:42131/native/open?path=secret",
            "http://127.0.0.1:42131/native/open#x",
            "http://127.0.0.1:42131/zim/a/native/open",
        ] {
            assert_eq!(native_action(42131, &Url::parse(address).unwrap()), None);
        }
        assert_eq!(
            native_action(
                42131,
                &Url::parse("http://127.0.0.1:42131/native/open").unwrap()
            ),
            Some("open")
        );
    }

    #[test]
    fn applying_an_update_is_an_exact_owned_action() {
        let action = |address: &str| {
            native_action(42131, &Url::parse(address).unwrap()).map(str::to_owned)
        };
        assert_eq!(
            action("http://127.0.0.1:42131/native/apply-update").as_deref(),
            Some("apply-update")
        );
        for address in [
            "http://127.0.0.1:42132/native/apply-update",
            "http://127.0.0.1:42131/native/apply-update?now=1",
            "http://127.0.0.1:42131/native/apply-update#x",
            "http://127.0.0.1:42131/native/apply-update/",
            "https://127.0.0.1:42131/native/apply-update",
        ] {
            assert_eq!(action(address), None, "{address}");
        }
    }

    #[test]
    fn only_the_owned_loopback_origin_can_navigate() {
        for address in [
            "https://example.org/",
            "http://127.0.0.1:42132/library",
            "http://localhost:42131/",
            "http://127.0.0.1.example.org:42131/",
            "http://user@127.0.0.1:42131/",
            "http://user:password@127.0.0.1:42131/",
            "https://127.0.0.1:42131/",
            "file:///C:/Users/user/private.txt",
            "javascript:alert(1)",
            "tauri://localhost/",
            "about:blank",
        ] {
            assert!(
                !allows(42131, &Url::parse(address).expect("test URL")),
                "external navigation accepted"
            );
        }
        for address in [
            "http://127.0.0.1:42131/launch?token=synthetic-token",
            "http://127.0.0.1:42131/library?pack=iso",
            "http://127.0.0.1:42131/zim/iso/C/Trefoil_knot",
        ] {
            assert!(allows(42131, &Url::parse(address).expect("test URL")));
        }
    }
}
