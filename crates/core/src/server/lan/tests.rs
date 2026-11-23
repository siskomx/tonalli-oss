use super::*;

#[test]
fn default_bind_is_loopback() {
    let cfg = ServerCfg::default();
    assert_eq!(cfg.bind, "127.0.0.1".parse::<std::net::IpAddr>().unwrap());
    assert_eq!(cfg.port, 0);
    assert_eq!(cfg.max_clients, 8);
}

#[test]
fn wildcard_public_cgnat_and_mapped_binds_are_refused() {
    for address in [
        "0.0.0.0",
        "::",
        "8.8.8.8",
        "100.64.0.1",
        "100.127.255.254",
        "2001:4860:4860::8888",
        "::ffff:127.0.0.1",
        "::ffff:192.168.1.1",
        "fe80::1",
        "224.0.0.1",
        "255.255.255.255",
        "192.0.0.1",
    ] {
        assert!(
            matches!(
                validate_bind(address.parse().unwrap()),
                Err(PackError::Mismatch(_))
            ),
            "{address}"
        );
    }
}

#[test]
fn rfc1918_link_local_loopback_and_ula_binds_are_allowed() {
    for address in [
        "127.0.0.1",
        "127.255.255.254",
        "::1",
        "10.0.0.1",
        "10.255.255.254",
        "172.16.0.1",
        "172.31.255.254",
        "192.168.0.1",
        "192.168.255.254",
        "169.254.0.1",
        "169.254.255.254",
        "fc00::1",
        "fdff:ffff::1",
    ] {
        validate_bind(address.parse().unwrap())
            .unwrap_or_else(|error| panic!("{address}: {error}"));
    }
}

#[test]
fn private_ipv4_boundaries_do_not_expand_into_public_space() {
    for address in [
        "9.255.255.255",
        "11.0.0.0",
        "172.15.255.255",
        "172.32.0.0",
        "192.167.255.255",
        "192.169.0.0",
        "169.253.255.255",
        "169.255.0.0",
        "fbff::1",
        "fe00::1",
    ] {
        assert!(
            validate_bind(address.parse().unwrap()).is_err(),
            "{address}"
        );
    }
}
