use std::sync::Arc;
use std::time::Duration;

use axum::serve::Listener as _;
use rustls::pki_types::{PrivatePkcs8KeyDer, ServerName};

use super::Listener;

#[tokio::test]
async fn an_idle_tcp_peer_does_not_block_a_later_tls_handshake() {
    let key = rcgen::KeyPair::generate().unwrap();
    let certificate = rcgen::CertificateParams::new(vec!["127.0.0.1".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    let mut listener = Listener::new(
        tcp,
        Arc::new(config),
        tokio_util::sync::CancellationToken::new(),
    );
    let _idle = tokio::net::TcpStream::connect(address).await.unwrap();
    let server = tokio::spawn(async move { listener.accept().await });
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate.der().clone()).unwrap();
    let client = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let _tls = tokio::time::timeout(
        Duration::from_secs(1),
        connector.connect(ServerName::IpAddress(address.ip().into()), stream),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn accepted_tls_streams_hold_the_global_connection_permit_until_drop() {
    let key = rcgen::KeyPair::generate().unwrap();
    let certificate = rcgen::CertificateParams::new(vec!["127.0.0.1".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    let mut listener = Listener::new(
        tcp,
        Arc::new(config),
        tokio_util::sync::CancellationToken::new(),
    );
    let accepting = tokio::spawn(async move {
        let mut held = Vec::new();
        for _ in 0..64 {
            held.push(listener.accept().await.0);
        }
        (listener, held)
    });
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate.der().clone()).unwrap();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ));
    let mut clients = Vec::new();
    for _ in 0..64 {
        let socket = tokio::net::TcpStream::connect(address).await.unwrap();
        clients.push(
            connector
                .connect(ServerName::IpAddress(address.ip().into()), socket)
                .await
                .unwrap(),
        );
    }
    let (mut listener, held) = accepting.await.unwrap();
    let accepting = tokio::spawn(async move { listener.accept().await });
    let socket = tokio::net::TcpStream::connect(address).await.unwrap();
    assert!(tokio::time::timeout(
        Duration::from_secs(1),
        connector.connect(ServerName::IpAddress(address.ip().into()), socket)
    )
    .await
    .unwrap()
    .is_err());
    accepting.abort();
    drop(held);
    drop(clients);
}

#[tokio::test(start_paused = true)]
async fn stalled_handshake_timeout_reclaims_its_connection_permit() {
    use tokio::io::AsyncReadExt;
    let key = rcgen::KeyPair::generate().unwrap();
    let certificate = rcgen::CertificateParams::new(vec!["127.0.0.1".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
    let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    let mut listener = Listener::new(
        tcp,
        Arc::new(config),
        tokio_util::sync::CancellationToken::new(),
    );
    let permits = Arc::clone(&listener.permits);
    let mut idle = tokio::net::TcpStream::connect(address).await.unwrap();
    let accepting = tokio::spawn(async move { listener.accept().await });
    for _ in 0..100 {
        if permits.available_permits() == 63 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(permits.available_permits(), 63);
    tokio::time::advance(Duration::from_secs(6)).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), idle.read(&mut [0_u8; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(permits.available_permits(), 64);
    accepting.abort();
}
