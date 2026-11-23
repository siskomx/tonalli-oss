//! Separate explicit TV owner; native address enumeration is shared with browser sharing.
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::thread::JoinHandle;
use tonalli_core::{
    pack::PackError,
    server::{TvCatalog, TvControl, TvServer, TvStatus},
};
#[derive(Default)]
pub struct DesktopTv {
    state: Arc<Mutex<State>>,
    watcher: Mutex<Option<Watcher>>,
}
#[derive(Default)]
struct State {
    server: Option<TvServer>,
    closed: bool,
    error: Option<String>,
}
struct Watcher {
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}
fn watch(state: Weak<Mutex<State>>) -> Result<Watcher, PackError> {
    let (stop, stopped) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("desktop-tv-network".into())
        .spawn(move || {
            while matches!(
                stopped.recv_timeout(std::time::Duration::from_secs(2)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                let Some(state) = state.upgrade() else {
                    return;
                };
                let networks = crate::sharing::networks().unwrap_or_default();
                let Ok(mut state) = state.lock() else {
                    return;
                };
                if state.closed {
                    return;
                }
                let Some(server) = state.server.as_ref() else {
                    return;
                };
                if !server.is_running()
                    || !networks
                        .iter()
                        .any(|n| n.id == server.address().ip().to_string())
                {
                    if let Some(mut server) = state.server.take() {
                        let _ = server.shutdown();
                    }
                    state.error = Some("TV mode stopped; check your local network".into());
                    return;
                }
            }
        })?;
    Ok(Watcher {
        stop,
        worker: Some(worker),
    })
}
impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn unavailable() -> PackError {
    PackError::Mismatch("TV mode unavailable; check your local network".into())
}
impl TvControl for DesktopTv {
    fn status(&self) -> Result<TvStatus, PackError> {
        let networks = crate::sharing::networks()?;
        let state = self.state.lock().map_err(|_| unavailable())?;
        let server = state.server.as_ref().filter(|s| s.is_running());
        Ok(TvStatus {
            running: server.is_some(),
            busy: false,
            url: server.map(|s| format!("http://{}/description.xml", s.address())),
            networks,
            error: state.error.clone(),
            selected_media: Vec::new(),
        })
    }
    fn start(&self, network: String, catalog: Arc<TvCatalog>) -> Result<(), PackError> {
        let mut watcher = self.watcher.lock().map_err(|_| unavailable())?;
        {
            let state = self.state.lock().map_err(|_| unavailable())?;
            if state.closed || state.server.is_some() {
                return Err(unavailable());
            }
        }
        // Join the previous monitor without holding the state lock it may need.
        drop(watcher.take());
        if !crate::sharing::networks()?.iter().any(|n| n.id == network) {
            return Err(unavailable());
        }
        let bind = network
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| unavailable())?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if state.closed || state.server.is_some() {
            return Err(unavailable());
        }
        let server = TvServer::start(bind.into(), 0, catalog)?;
        let monitor = watch(Arc::downgrade(&self.state))?;
        state.server = Some(server);
        state.error = None;
        *watcher = Some(monitor);
        Ok(())
    }
    fn stop(&self) -> Result<(), PackError> {
        self.stop_owner(false)
    }
    fn close(&self) -> Result<(), PackError> {
        self.stop_owner(true)
    }
}
impl DesktopTv {
    fn stop_owner(&self, closing: bool) -> Result<(), PackError> {
        let mut watcher = self.watcher.lock().map_err(|_| unavailable())?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        state.closed |= closing;
        let result = state
            .server
            .take()
            .map(|mut server| server.shutdown())
            .transpose();
        state.error = None;
        let previous = watcher.take();
        drop(state);
        drop(previous);
        result.map(|_| ())
    }
}
impl Drop for DesktopTv {
    fn drop(&mut self) {
        let _ = self.stop_owner(true);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant},
    };
    use tonalli_core::{media::MediaStore, pack::PreadSource, server::SharedMedia};
    fn active() -> (tempfile::TempDir, DesktopTv, std::net::SocketAddr) {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.wav");
        fs::write(&source,b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0").unwrap();
        let store = MediaStore::open(root.path()).unwrap();
        let file = store
            .import(
                Arc::new(PreadSource::open(&source).unwrap()),
                "Selected.wav",
                "",
            )
            .unwrap();
        let catalog = Arc::new(
            TvCatalog::new(Arc::new(
                SharedMedia::capture(root.path(), &[file.id]).unwrap(),
            ))
            .unwrap(),
        );
        let server = TvServer::start("127.0.0.1".parse().unwrap(), 0, catalog).unwrap();
        let address = server.address();
        let owner = DesktopTv::default();
        owner.state.lock().unwrap().server = Some(server);
        (root, owner, address)
    }
    #[test]
    fn stop_releases_tv_without_closing_the_owner() {
        let (_root, owner, address) = active();
        owner.stop().unwrap();
        assert!(owner.state.lock().unwrap().server.is_none());
        assert!(!owner.state.lock().unwrap().closed);
        assert!(
            std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err()
        );
        owner.stop().unwrap();
    }
    #[test]
    fn close_releases_tv_and_marks_the_owner_closed() {
        let (_root, owner, _) = active();
        let weak = Arc::downgrade(&owner.state);
        owner.close().unwrap();
        assert!(owner.state.lock().unwrap().closed);
        assert!(owner.state.lock().unwrap().server.is_none());
        drop(owner);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn background_monitor_withdraws_an_ineligible_binding() {
        let (_root, owner, _) = active();
        *owner.watcher.lock().unwrap() = Some(watch(Arc::downgrade(&owner.state)).unwrap());
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if owner.state.lock().unwrap().server.is_none() {
                break;
            }
            assert!(
                Instant::now() < until,
                "loopback is never an eligible native LAN adapter"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(owner.state.lock().unwrap().error.is_some());
        owner.close().unwrap();
        assert!(owner.watcher.lock().unwrap().is_none());
    }
}
