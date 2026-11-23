use std::net::IpAddr;

use super::discovery;

#[test]
#[ignore = "requires TONALLI_TEST_LAN_BIND naming a real private interface"]
fn real_selected_interface_survives_startup_configuration() {
    let bind: IpAddr = std::env::var("TONALLI_TEST_LAN_BIND")
        .expect("set TONALLI_TEST_LAN_BIND to a real private interface address")
        .parse()
        .expect("TONALLI_TEST_LAN_BIND must be a literal IP address");
    assert!(!bind.is_loopback(), "this regression requires actual mDNS");
    let listener = std::net::TcpListener::bind((bind, 0)).expect("selected address must be local");
    let mut discovery =
        discovery::Discovery::start(bind, listener.local_addr().unwrap().port(), &"b".repeat(64))
            .expect("start discovery on selected interface")
            .expect("non-loopback discovery");
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(2) {
        discovery
            .check()
            .expect("interface configuration must not count as real address removal");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    discovery
        .stop()
        .expect("withdraw selected-interface service");
}

#[test]
fn loopback_listener_does_not_advertise_on_any_interface() {
    for bind in ["127.0.0.1", "::1"] {
        assert!(
            discovery::service(bind.parse().unwrap(), 1234, &"a".repeat(64))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn discovery_advertises_only_the_bound_address_and_contains_no_txt_secrets() {
    let bind: IpAddr = "192.168.1.2".parse().unwrap();
    let service = discovery::service(bind, 1234, &"a".repeat(64))
        .unwrap()
        .unwrap();
    assert_eq!(service.get_type(), "_tonalli._tcp.local.");
    assert_eq!(service.get_port(), 1234);
    assert_eq!(service.get_addresses().len(), 1);
    assert!(service.get_addresses().contains(&bind));
    assert_eq!(service.get_properties().iter().count(), 0);
}

#[test]
fn daemon_disables_all_then_enables_selected_interface_before_registration() {
    use std::cell::RefCell;
    let bind: IpAddr = "192.168.1.2".parse().unwrap();
    let calls = RefCell::new(Vec::new());
    discovery::publish(
        bind,
        |enabled, interface| {
            calls.borrow_mut().push(match (enabled, interface) {
                (false, mdns_sd::IfKind::All) => "disable-all",
                (true, mdns_sd::IfKind::Addr(ip)) if ip == bind => "enable-bound-interface",
                _ => panic!("unexpected discovery interface command"),
            });
            Ok(())
        },
        || {
            calls.borrow_mut().push("register");
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        *calls.borrow(),
        ["disable-all", "enable-bound-interface", "register"]
    );
}

#[test]
fn daemon_interface_failure_prevents_enable_and_registration() {
    use std::cell::Cell;
    let calls = Cell::new(0);
    let registered = Cell::new(false);
    let result = discovery::publish(
        "192.168.1.2".parse().unwrap(),
        |_, _| {
            calls.set(calls.get() + 1);
            Err(crate::pack::PackError::Io(
                "synthetic interface setup failure".into(),
            ))
        },
        || {
            registered.set(true);
            Ok(())
        },
    );
    assert!(matches!(result, Err(crate::pack::PackError::Io(_))));
    assert_eq!(calls.get(), 1);
    assert!(!registered.get());
}
