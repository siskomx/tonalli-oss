// Exercise the production protocol boundary without unrelated lib-unit/worker tests.
#[path = "../src/server/tv/ssdp/mod.rs"]
mod ssdp;
const GROUP: &str = "239.255.255.250:1900";
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
const UUID: &str = "b512ca81-64d4-4380-b64b-0cec32b12fd2";
fn loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}
#[tokio::test]
async fn ssdp_burst_is_bounded_and_the_next_window_recovers() {
    let prepared = ssdp::Prepared::new(
        Ipv4Addr::LOCALHOST,
        "127.0.0.1:48901".parse().unwrap(),
        UUID,
    )
    .unwrap();
    let address = prepared.local_addr().unwrap();
    assert!(
        std::net::UdpSocket::bind(address).is_err(),
        "owned loopback listener occupies its socket"
    );
    let stop = CancellationToken::new();
    let owner = tokio::spawn(prepared.into_async().unwrap().run(stop.clone(), loopback));
    let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    for _ in 0..24 {
        client
            .send_to(&request("ssdp:all", "1"), address)
            .await
            .unwrap();
    }
    let until = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut buffer = [0; 2048];
    let mut replies = 0;
    while let Ok(Ok(_)) = tokio::time::timeout_at(until, client.recv_from(&mut buffer)).await {
        replies += 1;
    }
    assert!(
        (1..=40).contains(&replies),
        "eight requests maximum, five identities per request: {replies}"
    );
    client
        .send_to(&request("upnp:rootdevice", "1"), address)
        .await
        .unwrap();
    let (size, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    assert!(std::str::from_utf8(&buffer[..size])
        .unwrap()
        .contains("ST: upnp:rootdevice\r\n"));
    stop.cancel();
    owner.await.unwrap().unwrap();
}
#[tokio::test]
async fn ssdp_owned_udp_search_returns_five_identities_then_releases_socket() {
    let prepared = ssdp::Prepared::new(
        Ipv4Addr::LOCALHOST,
        "127.0.0.1:48901".parse().unwrap(),
        UUID,
    )
    .unwrap();
    let address = prepared.local_addr().unwrap();
    let discovery = prepared.into_async().unwrap();
    let stop = CancellationToken::new();
    let owner = tokio::spawn(discovery.run(stop.clone(), loopback));
    let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client
        .send_to(&request("ssdp:all", "1"), address)
        .await
        .unwrap();
    let mut buffer = [0; 2048];
    let mut replies = std::collections::HashSet::new();
    for _ in 0..5 {
        let (size, peer) =
            tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
                .await
                .expect("SSDP reply")
                .unwrap();
        assert!(peer.ip().is_loopback());
        let value = std::str::from_utf8(&buffer[..size]).unwrap().to_owned();
        assert!(value.contains("LOCATION: http://127.0.0.1:48901/description.xml"));
        replies.insert(value);
    }
    assert_eq!(replies.len(), 5);
    client
        .send_to(
            &request("urn:schemas-upnp-org:device:InternetGatewayDevice:1", "1"),
            address,
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), client.recv_from(&mut buffer))
            .await
            .is_err()
    );
    client
        .send_to(&request("ssdp:all", "5"), address)
        .await
        .unwrap();
    stop.cancel();
    owner.await.unwrap().unwrap();
    while client.try_recv_from(&mut buffer).is_ok() {}
    assert!(
        tokio::time::timeout(Duration::from_millis(150), client.recv_from(&mut buffer))
            .await
            .is_err()
    );
    drop(std::net::UdpSocket::bind(address).expect("owned listener released"));
}
#[tokio::test]
async fn ssdp_sends_nothing_to_a_rejected_peer() {
    let prepared = ssdp::Prepared::new(
        Ipv4Addr::LOCALHOST,
        "127.0.0.1:48901".parse().unwrap(),
        UUID,
    )
    .unwrap();
    let address = prepared.local_addr().unwrap();
    let stop = CancellationToken::new();
    let owner = tokio::spawn(prepared.into_async().unwrap().run(stop.clone(), |_| false));
    let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client
        .send_to(&request("ssdp:all", "1"), address)
        .await
        .unwrap();
    let mut buffer = [0; 2048];
    assert!(
        tokio::time::timeout(Duration::from_millis(1200), client.recv_from(&mut buffer))
            .await
            .is_err()
    );
    stop.cancel();
    owner.await.unwrap().unwrap();
}
fn request(target: &str, mx: &str) -> Vec<u8> {
    format!("M-SEARCH * HTTP/1.1\r\nHOST: {GROUP}\r\nMAN: \"ssdp:discover\"\r\nMX: {mx}\r\nST: {target}\r\n\r\n").into_bytes()
}
