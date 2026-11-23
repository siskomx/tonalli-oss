use std::net::IpAddr;
use std::time::Duration;

use mdns_sd::{DaemonEvent, IfKind, ServiceDaemon, ServiceInfo};

use crate::pack::PackError;

pub(super) fn service(
    bind: IpAddr,
    port: u16,
    pin: &str,
) -> Result<Option<ServiceInfo>, PackError> {
    if bind.is_loopback() {
        return Ok(None);
    }
    super::validate_bind(bind)?;
    if pin.len() != 64 || !pin.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PackError::Mismatch("invalid LAN discovery identity".into()));
    }
    let name = format!("Tonalli-{}", &pin[..12]);
    let mut info = ServiceInfo::new(
        "_tonalli._tcp.local.",
        &name,
        &format!("{}.", super::identity::HOSTNAME),
        bind,
        port,
        std::collections::HashMap::<String, String>::new(),
    )
    .map_err(error)?;
    info.set_interfaces(vec![IfKind::Addr(bind)]);
    Ok(Some(info))
}

pub(super) fn publish<T>(
    bind: IpAddr,
    mut configure: impl FnMut(bool, IfKind) -> Result<(), PackError>,
    register: impl FnOnce() -> Result<T, PackError>,
) -> Result<T, PackError> {
    configure(false, IfKind::All)?;
    configure(true, IfKind::Addr(bind))?;
    register()
}

pub(super) struct Discovery {
    daemon: Option<ServiceDaemon>,
    events: mdns_sd::Receiver<DaemonEvent>,
    name: String,
    bind: IpAddr,
}

impl Discovery {
    pub fn start(bind: IpAddr, port: u16, pin: &str) -> Result<Option<Self>, PackError> {
        let Some(info) = service(bind, port, pin)? else {
            return Ok(None);
        };
        let daemon = ServiceDaemon::new().map_err(error)?;
        let name = info.get_fullname().to_owned();
        let events = publish(
            bind,
            |enabled, interface| {
                if enabled {
                    daemon.enable_interface(interface).map_err(error)
                } else {
                    daemon.disable_interface(interface).map_err(error)
                }
            },
            || {
                // Interface selection emits IpDel; observe only subsequent runtime events.
                let events = daemon.monitor().map_err(error)?;
                daemon.register(info).map_err(error)?;
                Ok(events)
            },
        );
        let events = match events {
            Ok(events) => events,
            Err(failure) => {
                let _ = withdraw(&daemon, &name);
                return Err(failure);
            }
        };
        Ok(Some(Self {
            daemon: Some(daemon),
            events,
            name,
            bind,
        }))
    }

    pub fn check(&self) -> Result<(), PackError> {
        for event in self.events.try_iter() {
            match event {
                DaemonEvent::Error(failure) => return Err(error(failure)),
                DaemonEvent::NameChange(_) => return Err(error("LAN name conflict; sharing stopped rather than advertising a name outside its certificate")),
                DaemonEvent::IpDel(ip) if ip == self.bind => {
                    return Err(error("selected LAN address was removed"))
                }
                _ => {}
            }
        }
        if self.events.is_disconnected() {
            return Err(error("discovery daemon stopped"));
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), PackError> {
        let Some(daemon) = self.daemon.take() else {
            return Ok(());
        };
        withdraw(&daemon, &self.name)
    }
}

fn withdraw(daemon: &ServiceDaemon, name: &str) -> Result<(), PackError> {
    let unregister = daemon.unregister(name).map_err(error).and_then(|reply| {
        reply
            .recv_timeout(Duration::from_secs(1))
            .map(|_| ())
            .map_err(error)
    });
    let shutdown = daemon.shutdown().map_err(error).and_then(|reply| {
        reply
            .recv_timeout(Duration::from_secs(1))
            .map(|_| ())
            .map_err(error)
    });
    unregister.and(shutdown)
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn error(error: impl std::fmt::Display) -> PackError {
    PackError::Io(format!("LAN discovery: {error}"))
}
