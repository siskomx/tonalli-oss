use std::{fs, net::SocketAddr, sync::Arc};
use tonalli_core::{
    media::{MediaFile, MediaStore},
    pack::PreadSource,
    server::{SharedMedia, TvBrowse, TvCatalog, TvError, TvServer, TvService},
};
const WAV: &[u8] =
    b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01\x08\x02\0\0\0";
struct Fixture {
    root: tempfile::TempDir,
    store: MediaStore,
    files: Vec<MediaFile>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = MediaStore::open(root.path()).unwrap();
        let source = root.path().join("source");
        let files = [
            ("A & 'song.wav", WAV, "Selected"),
            ("Zed.wav", WAV, "Selected"),
            ("cover.png", PNG, "Selected"),
            ("Hidden.wav", WAV, "Private"),
            ("notes.txt", b"PRIVATE SENTINEL" as &[u8], "Private"),
            ("Fake.wav", b"not audio" as &[u8], "Selected"),
        ]
        .into_iter()
        .map(|(name, bytes, folder)| {
            fs::write(&source, bytes).unwrap();
            store
                .import(Arc::new(PreadSource::open(&source).unwrap()), name, folder)
                .unwrap()
        })
        .collect();
        Self { root, store, files }
    }
    fn catalog(&self, indices: &[usize]) -> Result<TvCatalog, tonalli_core::pack::PackError> {
        let ids = indices
            .iter()
            .map(|i| self.files[*i].id.clone())
            .collect::<Vec<_>>();
        TvCatalog::new(Arc::new(SharedMedia::capture(self.root.path(), &ids)?))
    }
}
fn addr() -> SocketAddr {
    "127.0.0.1:38400".parse().unwrap()
}
fn query<'a>(
    object_id: &'a str,
    metadata: bool,
    start: u32,
    count: u32,
    sort: &'a str,
) -> TvBrowse<'a> {
    TvBrowse {
        object_id,
        metadata,
        start,
        count,
        sort,
    }
}
#[test]
fn tv_catalog_rejects_empty_non_media_and_unvalidated_container_selections() {
    let f = Fixture::new();
    for indices in [&[][..], &[4], &[5], &[0, 4]] {
        assert!(f.catalog(indices).is_err());
    }
    assert!(f.catalog(&[0]).is_ok());
}
#[test]
fn tv_catalog_only_lists_and_reads_explicit_selection_and_selected_covers() {
    let f = Fixture::new();
    let catalog = f.catalog(&[0, 2]).unwrap();
    let xml = catalog.browse(addr(), query("0", false, 0, 0, "")).unwrap();
    assert_eq!((xml.returned, xml.total), (2, 2));
    assert!(xml.xml.contains("A &amp; &apos;song.wav"));
    assert!(xml.xml.contains("object.item.audioItem.musicTrack"));
    assert!(xml.xml.contains("upnp:albumArtURI"));
    assert!(xml
        .xml
        .contains(&format!("http://127.0.0.1:38400/media/{}", f.files[2].id)));
    assert!(!xml.xml.contains("Private"));
    assert!(!xml.xml.contains(&f.files[3].id));
    assert!(!xml.xml.contains("Selected"));
    assert!(catalog.source(&f.files[3].id).is_none());
    assert!(catalog.source("../notes.db").is_none());
    let (source, mime) = catalog.source(&f.files[0].id).unwrap();
    assert_eq!(mime, "audio/wav");
    let mut bytes = [0; 12];
    assert_eq!(source.read_at(0, &mut bytes).unwrap(), 12);
    assert_eq!(&bytes[..4], b"RIFF");
    let audio_only = f.catalog(&[0]).unwrap();
    assert!(!audio_only
        .browse(addr(), query("0", false, 0, 0, ""))
        .unwrap()
        .xml
        .contains("albumArtURI"));
    f.store.rename(&f.files[0].id, 1, "New.wav").unwrap();
    assert!(catalog
        .browse(addr(), query("0", false, 0, 0, ""))
        .unwrap()
        .xml
        .contains("A &amp; &apos;song.wav"));
    let root = catalog.browse(addr(), query("0", true, 0, 0, "")).unwrap();
    assert_eq!((root.returned, root.total), (1, 1));
    assert!(root.xml.contains("parentID=\"-1\""));
    assert!(root.xml.contains("childCount=\"2\""));
    assert!(catalog.protocol_info().contains("http-get:*:audio/wav:*"));
}
#[test]
fn tv_browse_is_paginated_typed_and_cannot_publish_an_external_origin() {
    let f = Fixture::new();
    let catalog = f.catalog(&[0, 1, 2]).unwrap();
    let first = catalog
        .browse(addr(), query("0", false, 0, 1, "+dc:title"))
        .unwrap();
    assert_eq!((first.returned, first.total), (1, 3));
    assert!(first.xml.contains("A &amp;"));
    let last = catalog
        .browse(addr(), query("0", false, 0, 1, "-dc:title"))
        .unwrap();
    assert!(last.xml.contains("Zed.wav"));
    let beyond = catalog
        .browse(addr(), query("0", false, u32::MAX, u32::MAX, ""))
        .unwrap();
    assert_eq!((beyond.returned, beyond.total), (0, 3));
    assert_eq!(
        catalog
            .browse(addr(), query("missing", true, 0, 0, ""))
            .err(),
        Some(TvError::Missing)
    );
    assert_eq!(
        catalog
            .browse(addr(), query(&f.files[0].id, false, 0, 0, ""))
            .err(),
        Some(TvError::NotContainer)
    );
    assert_eq!(
        catalog
            .browse(addr(), query("0", false, 0, 0, "+res@size"))
            .err(),
        Some(TvError::SortUnsupported)
    );
    assert_eq!(
        catalog
            .browse("8.8.8.8:80".parse().unwrap(), query("0", false, 0, 0, ""))
            .err(),
        Some(TvError::Invalid)
    );
    assert_eq!(
        catalog
            .browse("127.0.0.1:0".parse().unwrap(), query("0", false, 0, 0, ""))
            .err(),
        Some(TvError::Invalid)
    );
    let one = catalog
        .browse(addr(), query(&f.files[1].id, true, 0, 0, ""))
        .unwrap();
    assert_eq!((one.returned, one.total), (1, 1));
    assert!(one.xml.contains("Zed.wav"));
}

const CDS: &str = "urn:schemas-upnp-org:service:ContentDirectory:1";
const CM: &str = "urn:schemas-upnp-org:service:ConnectionManager:1";
fn soap(service: &str, action: &str, args: &str) -> Vec<u8> {
    format!("<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:{action} xmlns:u=\"{service}\">{args}</u:{action}></s:Body></s:Envelope>").into_bytes()
}
fn browse_args(filter: &str) -> String {
    format!("<ObjectID>0</ObjectID><BrowseFlag>BrowseDirectChildren</BrowseFlag><Filter>{filter}</Filter><StartingIndex>0</StartingIndex><RequestedCount>1</RequestedCount><SortCriteria>+dc:title</SortCriteria>")
}
#[test]
fn tv_soap_browse_escapes_nested_xml_and_honors_metadata_filter() {
    let f = Fixture::new();
    let c = f.catalog(&[0, 2]).unwrap();
    let header = format!("\"{CDS}#Browse\"");
    let reply = c
        .control(
            addr(),
            TvService::ContentDirectory,
            &header,
            &soap(CDS, "Browse", &browse_args("*")),
        )
        .unwrap();
    assert!(reply.contains("BrowseResponse"));
    assert!(reply.contains("&lt;DIDL-Lite"));
    assert!(reply.contains("A &amp;amp; &amp;apos;song.wav"));
    assert!(reply.contains("<NumberReturned>1</NumberReturned>"));
    assert!(reply.contains("<TotalMatches>2</TotalMatches>"));
    assert!(reply.contains("albumArtURI"));
    assert!(reply.contains("http-get:*:audio/wav:*"));
    let minimal = c
        .control(
            addr(),
            TvService::ContentDirectory,
            &header,
            &soap(CDS, "Browse", &browse_args("")),
        )
        .unwrap();
    assert!(!minimal.contains("albumArtURI"));
    assert!(!minimal.contains("http-get"));
    assert!(minimal.contains("dc:title"));
    let mut reader = quick_xml::Reader::from_str(&reply);
    loop {
        if matches!(reader.read_event().unwrap(), quick_xml::events::Event::Eof) {
            break;
        }
    }
}
#[test]
fn tv_soap_exposes_only_read_only_content_and_connection_actions() {
    let f = Fixture::new();
    let c = f.catalog(&[0]).unwrap();
    for (action, expected) in [
        ("GetSearchCapabilities", "<SearchCaps></SearchCaps>"),
        ("GetSortCapabilities", "<SortCaps>dc:title</SortCaps>"),
        ("GetSystemUpdateID", "<Id>0</Id>"),
    ] {
        let r = c
            .control(
                addr(),
                TvService::ContentDirectory,
                &format!("\"{CDS}#{action}\""),
                &soap(CDS, action, ""),
            )
            .unwrap();
        assert!(r.contains(expected));
    }
    for (action, args, expected) in [
        ("GetProtocolInfo", "", "http-get:*:audio/wav:*"),
        (
            "GetCurrentConnectionIDs",
            "",
            "<ConnectionIDs>0</ConnectionIDs>",
        ),
        (
            "GetCurrentConnectionInfo",
            "<ConnectionID>0</ConnectionID>",
            "<Direction>Output</Direction>",
        ),
    ] {
        let r = c
            .control(
                addr(),
                TvService::ConnectionManager,
                &format!("\"{CM}#{action}\""),
                &soap(CM, action, args),
            )
            .unwrap();
        assert!(r.contains(expected));
    }
    assert_eq!(
        c.control(
            addr(),
            TvService::ContentDirectory,
            &format!("\"{CDS}#DestroyObject\""),
            &soap(CDS, "DestroyObject", "<ObjectID>0</ObjectID>")
        )
        .err(),
        Some(TvError::Action)
    );
    assert_eq!(
        c.control(
            addr(),
            TvService::ConnectionManager,
            &format!("\"{CM}#GetCurrentConnectionInfo\""),
            &soap(
                CM,
                "GetCurrentConnectionInfo",
                "<ConnectionID>1</ConnectionID>"
            )
        )
        .err(),
        Some(TvError::ConnectionMissing)
    );
}
#[test]
fn tv_soap_rejects_ambiguous_spoofed_nested_and_entity_requests() {
    let f = Fixture::new();
    let c = f.catalog(&[0]).unwrap();
    let header = format!("\"{CDS}#Browse\"");
    let body = String::from_utf8(soap(CDS, "Browse", &browse_args("*"))).unwrap();
    let bad=[body.replace(CDS,"urn:forged"),body.replace("<ObjectID>0</ObjectID>","<ObjectID>0</ObjectID><ObjectID>0</ObjectID>"),body.replace("<ObjectID>0</ObjectID>","<ObjectID><nested>0</nested></ObjectID>"),format!("<!DOCTYPE s [<!ENTITY x SYSTEM 'file:///notes.db'>]>{body}"),body.replace("<ObjectID>0</ObjectID>","<ObjectID>&external;</ObjectID>"),body.replace("</s:Body>","<u:GetSortCapabilities xmlns:u=\"urn:schemas-upnp-org:service:ContentDirectory:1\"/></s:Body>"),format!("{body}{body}"),body.replace("<RequestedCount>1</RequestedCount>","<RequestedCount>4294967296</RequestedCount>"),body.replace("<StartingIndex>0</StartingIndex>","<StartingIndex>-1</StartingIndex>"),body.replace("<ObjectID>0</ObjectID>","<x:ObjectID xmlns:x=\"urn:forged\">0</x:ObjectID>"),body.replace("<Filter>*</Filter>",&format!("<Filter>{}</Filter>","a".repeat(17000)))];
    for b in bad {
        assert_eq!(
            c.control(addr(), TvService::ContentDirectory, &header, b.as_bytes())
                .err(),
            Some(TvError::Invalid)
        );
    }
    assert_eq!(
        c.control(
            addr(),
            TvService::ContentDirectory,
            &format!("\"{CDS}#GetSortCapabilities\""),
            body.as_bytes()
        )
        .err(),
        Some(TvError::Invalid)
    );
    assert_eq!(
        c.control(
            addr(),
            TvService::ConnectionManager,
            &header,
            body.as_bytes()
        )
        .err(),
        Some(TvError::Invalid)
    );
}

#[test]
fn tv_connection_identifiers_use_the_protocol_signed_integer_type() {
    let f = Fixture::new();
    let c = f.catalog(&[0]).unwrap();
    let header = format!("\"{CM}#GetCurrentConnectionInfo\"");
    for value in ["-1", "-2147483648", "2147483647"] {
        assert_eq!(
            c.control(
                addr(),
                TvService::ConnectionManager,
                &header,
                &soap(
                    CM,
                    "GetCurrentConnectionInfo",
                    &format!("<ConnectionID>{value}</ConnectionID>")
                )
            )
            .err(),
            Some(TvError::ConnectionMissing),
            "{value}"
        );
    }
    for value in ["2147483648", "-2147483649"] {
        assert_eq!(
            c.control(
                addr(),
                TvService::ConnectionManager,
                &header,
                &soap(
                    CM,
                    "GetCurrentConnectionInfo",
                    &format!("<ConnectionID>{value}</ConnectionID>")
                )
            )
            .err(),
            Some(TvError::Invalid),
            "{value}"
        );
    }
}

#[test]
fn tv_http_only_exposes_selected_media_and_rejects_private_routes_and_foreign_hosts() {
    let f = Fixture::new();
    let catalog = Arc::new(f.catalog(&[0, 2]).unwrap());
    let mut server = TvServer::start("127.0.0.1".parse().unwrap(), 0, catalog).unwrap();
    let base = format!("http://{}", server.address());
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let description = client
        .get(format!("{base}/description.xml"))
        .send()
        .unwrap();
    assert_eq!(description.status(), 200);
    let text = description.text().unwrap();
    assert!(text.contains("urn:schemas-upnp-org:device:MediaServer:1"));
    assert!(text.contains(CDS));
    assert!(text.contains(CM));
    assert!(!text.contains("Private"));
    for path in ["/content-directory.xml", "/connection-manager.xml"] {
        let response = client.get(format!("{base}{path}")).send().unwrap();
        assert_eq!(response.status(), 200);
        let text = response.text().unwrap();
        let mut reader = quick_xml::Reader::from_str(&text);
        loop {
            if matches!(reader.read_event().unwrap(), quick_xml::events::Event::Eof) {
                break;
            }
        }
    }
    for path in [
        "/",
        "/api/notes",
        "/api/files",
        "/api/sharing",
        "/api/tv",
        "/api/tv/start",
        "/api/tv/stop",
        "/api/room",
        "/api/admin",
        "/settings",
        "/notes.db",
        "/pair",
    ] {
        assert_eq!(
            client.get(format!("{base}{path}")).send().unwrap().status(),
            404,
            "{path}"
        );
        assert_eq!(
            client
                .post(format!("{base}{path}"))
                .send()
                .unwrap()
                .status(),
            404,
            "POST {path}"
        );
    }
    assert_eq!(
        client
            .get(format!("{base}/media/{}", f.files[3].id))
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!("{base}/media/{}", f.files[4].id))
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!("{base}/description.xml"))
            .header("Host", "foreign.invalid")
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{base}/description.xml"))
            .header("Origin", "https://foreign.invalid")
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{base}/description.xml"))
            .header("Sec-Fetch-Site", "cross-site")
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .put(format!("{base}/media/{}", f.files[0].id))
            .send()
            .unwrap()
            .status(),
        405
    );
    server.shutdown().unwrap();
}
#[test]
fn tv_http_reuses_positioned_ranges_read_only_soap_and_stops_owned_serving() {
    let f = Fixture::new();
    let catalog = Arc::new(f.catalog(&[0, 2]).unwrap());
    assert!(matches!(
        TvServer::start("8.8.8.8".parse().unwrap(), 0, Arc::clone(&catalog)),
        Err(tonalli_core::pack::PackError::Mismatch(_))
    ));
    let mut server = TvServer::start("127.0.0.1".parse().unwrap(), 0, catalog).unwrap();
    let base = format!("http://{}", server.address());
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let media = format!("{base}/media/{}", f.files[0].id);
    let response = client.get(&media).send().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["Content-Type"], "audio/wav");
    assert_eq!(response.headers()["Accept-Ranges"], "bytes");
    assert_eq!(response.bytes().unwrap().as_ref(), WAV);
    let response = client.head(&media).send().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["Content-Length"], WAV.len().to_string());
    assert!(response.bytes().unwrap().is_empty());
    let response = client
        .get(&media)
        .header("Range", "bytes=12-15")
        .send()
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(
        response.headers()["Content-Range"],
        format!("bytes 12-15/{}", WAV.len())
    );
    assert_eq!(response.bytes().unwrap().as_ref(), &WAV[12..16]);
    assert_eq!(
        client
            .get(&media)
            .header("Range", "bytes=9999-")
            .send()
            .unwrap()
            .status(),
        416
    );
    let response = client
        .post(format!("{base}/control/content-directory"))
        .header("Content-Type", "text/xml; charset=utf-8")
        .header("SOAPAction", format!("\"{CDS}#Browse\""))
        .body(soap(CDS, "Browse", &browse_args("*")))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.text().unwrap().contains("BrowseResponse"));
    let bad = client
        .post(format!("{base}/control/content-directory"))
        .header("SOAPAction", format!("\"{CDS}#DestroyObject\""))
        .body(soap(CDS, "DestroyObject", "<ObjectID>0</ObjectID>"))
        .send()
        .unwrap();
    assert_eq!(bad.status(), 500);
    assert!(bad.text().unwrap().contains("<errorCode>401</errorCode>"));
    assert_eq!(
        client
            .post(format!("{base}/control/content-directory"))
            .body(vec![b'x'; 17000])
            .send()
            .unwrap()
            .status(),
        413
    );
    server.shutdown().unwrap();
    assert!(client.get(&media).send().is_err());
    server.shutdown().unwrap();
}

#[test]
fn tv_http_bounds_active_streams_and_stop_cancels_their_remaining_bodies() {
    use std::io::Read;
    let f = Fixture::new();
    let mut bytes = WAV.to_vec();
    let data_len = 8 * 1024 * 1024u32;
    bytes[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
    bytes[40..44].copy_from_slice(&data_len.to_le_bytes());
    bytes.resize(44 + data_len as usize, 0);
    let source = f.root.path().join("long.wav");
    fs::write(&source, &bytes).unwrap();
    let file = f
        .store
        .import(
            Arc::new(PreadSource::open(&source).unwrap()),
            "Long.wav",
            "",
        )
        .unwrap();
    let catalog = TvCatalog::new(Arc::new(
        SharedMedia::capture(f.root.path(), &[file.id.clone()]).unwrap(),
    ))
    .unwrap();
    let mut server = TvServer::start("127.0.0.1".parse().unwrap(), 0, Arc::new(catalog)).unwrap();
    let base = format!("http://{}", server.address());
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let mut responses = Vec::new();
    for _ in 0..4 {
        let mut response = client
            .get(format!("{base}/media/{}", file.id))
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        let mut prefix = [0; 64];
        response.read_exact(&mut prefix).unwrap();
        assert_eq!(&prefix[..4], b"RIFF");
        responses.push(response);
    }
    assert_eq!(
        client
            .get(format!("{base}/description.xml"))
            .send()
            .unwrap()
            .status(),
        503
    );
    let started = std::time::Instant::now();
    server.shutdown().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    for mut response in responses {
        let mut rest = Vec::new();
        let complete = response.read_to_end(&mut rest);
        assert!(
            complete.is_err() || rest.len() + 64 < bytes.len(),
            "Stop must cut off an active body"
        );
    }
}

#[test]
fn tv_http_caps_incomplete_connections_and_recovers_after_they_close() {
    use std::io::{Read, Write};
    let f = Fixture::new();
    let mut server = TvServer::start(
        "127.0.0.1".parse().unwrap(),
        0,
        Arc::new(f.catalog(&[0]).unwrap()),
    )
    .unwrap();
    let mut sockets = Vec::new();
    for _ in 0..64 {
        let mut socket = std::net::TcpStream::connect(server.address()).unwrap();
        write!(
            socket,
            "GET /description.xml HTTP/1.1\r\nHost: {}\r\n",
            server.address()
        )
        .unwrap();
        sockets.push(socket);
    }
    let mut extra = std::net::TcpStream::connect(server.address()).unwrap();
    extra
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let sent = write!(
        extra,
        "GET /description.xml HTTP/1.1\r\nHost: {}\r\n",
        server.address()
    );
    let mut byte = [0];
    match sent.and_then(|()| extra.read(&mut byte)) {
        Ok(0) => {}
        Err(error)
            if !matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) => {}
        other => panic!("Connection beyond the limit must close: {other:?}"),
    };
    drop(extra);
    drop(sockets);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(1))
        .build()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if client
            .get(format!("http://{}/description.xml", server.address()))
            .send()
            .is_ok_and(|response| response.status() == 200)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Closed connections must release their slots"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    server.shutdown().unwrap();
}

fn accept_callback(listener: &std::net::TcpListener) -> std::net::TcpStream {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Expected an event callback"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(error) => panic!("Callback listener: {error}"),
        }
    }
}
fn read_callback(
    stream: &mut std::net::TcpStream,
) -> (std::collections::BTreeMap<String, String>, String) {
    use std::io::Read;
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 8192);
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    let text = String::from_utf8(bytes).unwrap();
    let mut lines = text.split("\r\n");
    assert_eq!(lines.next(), Some("NOTIFY /event HTTP/1.1"));
    let mut headers = std::collections::BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').unwrap();
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let size = headers["content-length"].parse::<usize>().unwrap();
    assert!(size <= 4096);
    let mut body = vec![0; size];
    stream.read_exact(&mut body).unwrap();
    (headers, String::from_utf8(body).unwrap())
}
fn callback_receiver(
    count: usize,
) -> (
    std::net::SocketAddr,
    std::sync::mpsc::Receiver<(std::collections::BTreeMap<String, String>, String)>,
    std::thread::JoinHandle<()>,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        for _ in 0..count {
            let mut stream = accept_callback(&listener);
            let event = read_callback(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            sender.send(event).unwrap();
        }
    });
    (address, receiver, worker)
}
fn tv_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap()
}
#[test]
fn tv_gena_initial_events_renew_scope_and_unsubscribe() {
    let f = Fixture::new();
    let mut server = TvServer::start(
        "127.0.0.1".parse().unwrap(),
        0,
        Arc::new(f.catalog(&[0]).unwrap()),
    )
    .unwrap();
    let base = format!("http://{}", server.address());
    let client = tv_client();
    let (callback, events, worker) = callback_receiver(2);
    let subscribe = reqwest::Method::from_bytes(b"SUBSCRIBE").unwrap();
    let unsubscribe = reqwest::Method::from_bytes(b"UNSUBSCRIBE").unwrap();
    let response = client
        .request(
            subscribe.clone(),
            format!("{base}/events/content-directory"),
        )
        .header("CALLBACK", format!("<http://{callback}/event>"))
        .header("NT", "upnp:event")
        .header("TIMEOUT", "Second-infinite")
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["timeout"], "Second-300");
    let sid = response.headers()["sid"].to_str().unwrap().to_owned();
    assert!(sid.starts_with("uuid:"));
    let (headers, body) = events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(headers["sid"], sid);
    assert_eq!(headers["nt"], "upnp:event");
    assert_eq!(headers["nts"], "upnp:propchange");
    assert_eq!(headers["seq"], "0");
    assert!(body.contains("<SystemUpdateID>0</SystemUpdateID>"));
    assert!(!body.contains(&f.files[0].id));
    let renewed = client
        .request(
            subscribe.clone(),
            format!("{base}/events/content-directory"),
        )
        .header("SID", &sid)
        .header("TIMEOUT", "Second-120")
        .send()
        .unwrap();
    assert_eq!(renewed.status(), 200);
    assert_eq!(renewed.headers()["sid"], sid);
    assert_eq!(renewed.headers()["timeout"], "Second-120");
    assert_eq!(
        client
            .request(
                subscribe.clone(),
                format!("{base}/events/connection-manager")
            )
            .header("SID", &sid)
            .send()
            .unwrap()
            .status(),
        412
    );
    assert_eq!(
        client
            .request(unsubscribe, format!("{base}/events/content-directory"))
            .header("SID", &sid)
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .request(
                subscribe.clone(),
                format!("{base}/events/content-directory")
            )
            .header("SID", &sid)
            .send()
            .unwrap()
            .status(),
        412
    );
    let cm = client
        .request(subscribe, format!("{base}/events/connection-manager"))
        .header("CALLBACK", format!("<http://{callback}/event>"))
        .header("NT", "upnp:event")
        .send()
        .unwrap();
    assert_eq!(cm.status(), 200);
    assert_ne!(cm.headers()["sid"], sid);
    let (_, body) = events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(body.contains("<CurrentConnectionIDs>0</CurrentConnectionIDs>"));
    assert!(body.contains("http-get:*:audio/wav:*"));
    worker.join().unwrap();
    server.shutdown().unwrap();
}
#[test]
fn tv_gena_rejects_callback_escape_and_mixed_credentials() {
    let f = Fixture::new();
    let mut server = TvServer::start(
        "127.0.0.1".parse().unwrap(),
        0,
        Arc::new(f.catalog(&[0]).unwrap()),
    )
    .unwrap();
    let url = format!("http://{}/events/content-directory", server.address());
    let client = tv_client();
    let subscribe = reqwest::Method::from_bytes(b"SUBSCRIBE").unwrap();
    for callback in [
        "<http://8.8.8.8:80/event>",
        "<http://127.0.0.2:8123/event>",
        "<http://localhost:8123/event>",
        "<https://127.0.0.1:8123/event>",
        "<http://user:pass@127.0.0.1:8123/event>",
        "<http://127.0.0.1:8123/event#fragment>",
        "<http://127.0.0.1:0/event>",
    ] {
        assert_eq!(
            client
                .request(subscribe.clone(), &url)
                .header("CALLBACK", callback)
                .header("NT", "upnp:event")
                .send()
                .unwrap()
                .status(),
            412,
            "{callback}"
        );
    }
    assert_eq!(
        client
            .request(subscribe.clone(), &url)
            .header("SID", "uuid:unknown")
            .header("CALLBACK", "<http://127.0.0.1:8123/event>")
            .header("NT", "upnp:event")
            .send()
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        client
            .request(subscribe.clone(), &url)
            .header("SID", "uuid:unknown")
            .send()
            .unwrap()
            .status(),
        412
    );
    assert_eq!(
        client
            .request(subscribe, &url)
            .header("CALLBACK", "<http://127.0.0.1:8123/event>")
            .header("NT", "wrong")
            .send()
            .unwrap()
            .status(),
        412
    );
    server.shutdown().unwrap();
}
#[test]
fn tv_gena_expired_leases_and_inflight_notify_end_with_the_owner() {
    let f = Fixture::new();
    let mut server = TvServer::start(
        "127.0.0.1".parse().unwrap(),
        0,
        Arc::new(f.catalog(&[0]).unwrap()),
    )
    .unwrap();
    let url = format!("http://{}/events/content-directory", server.address());
    let client = tv_client();
    let (callback, events, worker) = callback_receiver(1);
    let subscribe = reqwest::Method::from_bytes(b"SUBSCRIBE").unwrap();
    let response = client
        .request(subscribe.clone(), &url)
        .header("CALLBACK", format!("<http://{callback}/event>"))
        .header("NT", "upnp:event")
        .header("TIMEOUT", "Second-1")
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let sid = response.headers()["sid"].to_str().unwrap().to_owned();
    events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    worker.join().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(
        client
            .request(subscribe.clone(), &url)
            .header("SID", sid)
            .send()
            .unwrap()
            .status(),
        412
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let callback = listener.local_addr().unwrap();
    assert_eq!(
        client
            .request(subscribe, &url)
            .header("CALLBACK", format!("<http://{callback}/event>"))
            .header("NT", "upnp:event")
            .send()
            .unwrap()
            .status(),
        200
    );
    let mut socket = accept_callback(&listener);
    read_callback(&mut socket);
    let started = std::time::Instant::now();
    server.shutdown().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    use std::io::Read;
    let mut byte = [0];
    assert_eq!(socket.read(&mut byte).unwrap(), 0);
}

#[test]
fn tv_gena_bounds_subscribers_and_unsubscribe_releases_capacity() {
    let f = Fixture::new();
    let mut server = TvServer::start(
        "127.0.0.1".parse().unwrap(),
        0,
        Arc::new(f.catalog(&[0]).unwrap()),
    )
    .unwrap();
    let url = format!("http://{}/events/content-directory", server.address());
    let client = tv_client();
    let (callback, events, worker) = callback_receiver(32);
    let subscribe = reqwest::Method::from_bytes(b"SUBSCRIBE").unwrap();
    let mut ids = std::collections::BTreeSet::new();
    for _ in 0..32 {
        let response = client
            .request(subscribe.clone(), &url)
            .header("CALLBACK", format!("<http://{callback}/event>"))
            .header("NT", "upnp:event")
            .send()
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(ids.insert(response.headers()["sid"].to_str().unwrap().to_owned()));
    }
    for _ in 0..32 {
        events
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }
    worker.join().unwrap();
    assert_eq!(
        client
            .request(subscribe.clone(), &url)
            .header("CALLBACK", format!("<http://{callback}/event>"))
            .header("NT", "upnp:event")
            .send()
            .unwrap()
            .status(),
        503
    );
    let sid = ids.first().unwrap();
    assert_eq!(
        client
            .request(reqwest::Method::from_bytes(b"UNSUBSCRIBE").unwrap(), &url)
            .header("SID", sid)
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .request(subscribe, &url)
            .header("CALLBACK", format!("<http://{callback}/event>"))
            .header("NT", "upnp:event")
            .send()
            .unwrap()
            .status(),
        200
    );
    server.shutdown().unwrap();
}
#[test]
fn tv_gena_unsubscribe_cancels_a_pending_callback_connection() {
    use std::io::Read;
    let f = Fixture::new();
    let mut server = TvServer::start(
        "127.0.0.1".parse().unwrap(),
        0,
        Arc::new(f.catalog(&[0]).unwrap()),
    )
    .unwrap();
    let url = format!("http://{}/events/content-directory", server.address());
    let client = tv_client();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let callback = listener.local_addr().unwrap();
    let response = client
        .request(reqwest::Method::from_bytes(b"SUBSCRIBE").unwrap(), &url)
        .header("CALLBACK", format!("<http://{callback}/event>"))
        .header("NT", "upnp:event")
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let sid = response.headers()["sid"].to_str().unwrap().to_owned();
    let mut socket = accept_callback(&listener);
    read_callback(&mut socket);
    assert_eq!(
        client
            .request(reqwest::Method::from_bytes(b"UNSUBSCRIBE").unwrap(), &url)
            .header("SID", sid)
            .send()
            .unwrap()
            .status(),
        200
    );
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let mut byte = [0];
    assert_eq!(socket.read(&mut byte).unwrap(), 0);
    server.shutdown().unwrap();
}
