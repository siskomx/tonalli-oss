//! Owned WinDNS completion and interface-scoped asynchronous resolver.
use super::{Identity, NetworkError};
use std::sync::{Arc, Condvar, Mutex};
#[cfg(test)]
use std::time::{Duration, Instant};
#[cfg(windows)]
#[path = "native_dns.rs"]
mod native;

fn check_identity(identity: Identity, current: impl Fn() -> bool) -> Result<(), NetworkError> {
    if identity.index == 0 || identity.luid == 0 || !current() {
        return Err(NetworkError::Revoked);
    }
    Ok(())
}
#[cfg(test)]
fn checked_lookup(
    identity: Identity,
    current: impl Fn() -> bool,
    lookup: impl FnOnce(Identity) -> Result<Vec<std::net::SocketAddr>, NetworkError>,
) -> Result<Vec<std::net::SocketAddr>, NetworkError> {
    check_identity(identity, &current)?;
    let result = lookup(identity);
    check_identity(identity, current)?;
    result
}

#[derive(Default)]
struct State {
    completed: bool,
    cancel_requested: bool,
    native_calls: usize,
    waker: Option<std::task::Waker>,
}
pub(super) struct PendingDns {
    name: Box<[u16]>,
    state: Mutex<State>,
    changed: Condvar,
    cancel: Box<dyn Fn() + Send + Sync>,
}
struct Completion {
    query: Arc<PendingDns>,
}
impl PendingDns {
    fn with_cancel(
        name: &str,
        cancel: impl Fn() + Send + Sync + 'static,
    ) -> (Arc<Self>, Completion) {
        let query = Arc::new(Self {
            name: name.encode_utf16().chain(std::iter::once(0)).collect(),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            cancel: Box::new(cancel),
        });
        let callback = Completion {
            query: Arc::clone(&query),
        };
        (query, callback)
    }
    #[cfg(test)]
    fn start(name: &str, _identity: Identity) -> (Arc<Self>, Completion) {
        Self::with_cancel(name, || {})
    }
    #[cfg(test)]
    fn name_address(&self) -> usize {
        self.name.as_ptr() as usize
    }
    fn begin_native(&self) -> Result<(), NetworkError> {
        let mut state = self.state.lock().map_err(|_| NetworkError::Dns)?;
        state.native_calls += 1;
        Ok(())
    }
    fn end_native(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.native_calls -= 1;
            self.changed.notify_all();
            let waker = state.waker.take();
            drop(state);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
    fn request_cancel(&self, native_cancel: impl FnOnce()) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.completed || state.cancel_requested {
            return;
        }
        state.cancel_requested = true;
        state.native_calls += 1;
        drop(state);
        native_cancel();
        if let Ok(mut state) = self.state.lock() {
            state.native_calls -= 1;
            self.changed.notify_all();
            let waker = state.waker.take();
            drop(state);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
    pub(super) fn cancel_owned(&self) {
        self.request_cancel(|| (self.cancel)());
    }
    #[cfg(test)]
    fn wait(&self, timeout: Duration) -> Result<(), NetworkError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(NetworkError::Timeout)?;
        let mut state = self.state.lock().map_err(|_| NetworkError::Dns)?;
        while !state.completed || state.native_calls != 0 {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(NetworkError::Timeout)?;
            state = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| NetworkError::Dns)?
                .0;
        }
        Ok(())
    }
    pub(super) fn drain(&self) -> Result<(), NetworkError> {
        let mut state = self.state.lock().map_err(|_| NetworkError::Dns)?;
        while !state.completed || state.native_calls != 0 {
            state = self.changed.wait(state).map_err(|_| NetworkError::Dns)?;
        }
        Ok(())
    }
    pub(super) fn completed(&self) -> bool {
        self.state
            .lock()
            .map(|s| s.completed && s.native_calls == 0)
            .unwrap_or(false)
    }
    fn poll_complete(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), NetworkError>> {
        let Ok(mut state) = self.state.lock() else {
            return std::task::Poll::Ready(Err(NetworkError::Dns));
        };
        if state.completed && state.native_calls == 0 {
            return std::task::Poll::Ready(Ok(()));
        }
        state.waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}
impl Completion {
    fn complete(self, records_release: impl FnOnce()) {
        records_release();
        if let Ok(mut state) = self.query.state.lock() {
            state.completed = true;
            self.query.changed.notify_all();
            let waker = state.waker.take();
            drop(state);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

#[cfg(windows)]
pub(super) struct WifiResolver(pub(super) Arc<super::WifiLease>);

#[cfg(windows)]
impl reqwest::dns::Resolve for WifiResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let lease = Arc::clone(&self.0);
        Box::pin(async move {
            use windows_sys::Win32::NetworkManagement::Dns::{DNS_TYPE_A, DNS_TYPE_AAAA};
            let mut addresses = Vec::new();
            for kind in [DNS_TYPE_A, DNS_TYPE_AAAA] {
                check_identity(lease.identity, || lease.observe_native().is_ok())?;
                let native = native::NativeQuery::prepare(name.as_str(), lease.identity, kind)?;
                lease.track_pending(native.pending())?;
                native.submit()?;
                let result = native.addresses().await;
                check_identity(lease.identity, || lease.observe_native().is_ok())?;
                if let Ok(found) = result {
                    addresses.extend(found.into_iter().map(|ip| std::net::SocketAddr::new(ip, 0)));
                }
            }
            if addresses.is_empty() {
                return Err(Box::new(NetworkError::Dns) as Box<dyn std::error::Error + Send + Sync>);
            }
            let addresses: reqwest::dns::Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    fn start() -> (Arc<PendingDns>, Completion) {
        PendingDns::start("catalog.invalid", Identity { index: 4, luid: 40 })
    }
    #[test]
    fn revoked_dns_never_starts_or_returns_addresses_after_identity_changes() {
        let identity = Identity { index: 4, luid: 40 };
        assert_eq!(
            checked_lookup(identity, || false, |_| panic!("revoked DNS must not start")),
            Err(NetworkError::Revoked)
        );
        assert_eq!(
            checked_lookup(
                Identity { index: 0, luid: 40 },
                || true,
                |_| panic!("zero means all interfaces and must never reach WinDNS")
            ),
            Err(NetworkError::Revoked)
        );
        let live = std::cell::Cell::new(true);
        let result = checked_lookup(
            identity,
            || live.get(),
            |captured| {
                assert_eq!(captured, identity);
                live.set(false);
                Ok(vec!["127.0.0.1:0".parse().unwrap()])
            },
        );
        assert_eq!(result, Err(NetworkError::Revoked));
    }
    #[test]
    fn timeout_cancel_ack_does_not_release_pending_native_storage() {
        let (query, callback) = start();
        let weak = Arc::downgrade(&query);
        let address = query.name_address();
        assert_eq!(
            query.wait(Duration::from_millis(1)),
            Err(NetworkError::Timeout)
        );
        query.request_cancel(|| {});
        assert!(!query.completed());
        assert_eq!(query.name_address(), address);
        drop(query);
        assert!(
            weak.upgrade().is_some(),
            "callback owns storage after caller leaves"
        );
        callback.complete(|| {});
        assert!(
            weak.upgrade().is_none(),
            "actual callback releases final native ownership"
        );
    }
    #[test]
    fn cancel_native_call_allows_offthread_completion_before_it_returns() {
        let (query, callback) = start();
        let (done, observed) = mpsc::sync_channel(1);
        query.request_cancel(|| {
            std::thread::spawn(move || {
                callback.complete(|| {});
                done.send(()).unwrap();
            });
            observed
                .recv_timeout(Duration::from_secs(2))
                .expect("callback lock inversion");
        });
        assert!(query.completed());
        assert_eq!(query.wait(Duration::from_millis(1)), Ok(()));
    }
    #[test]
    fn immediate_completion_is_visible_once_and_late_cancel_does_not_call_native() {
        let (query, callback) = start();
        let frees = Arc::new(AtomicUsize::new(0));
        let record = Arc::clone(&frees);
        callback.complete(move || {
            record.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(query.wait(Duration::from_millis(1)), Ok(()));
        query.request_cancel(|| panic!("completed cancel handle must not be reused"));
        assert_eq!(frees.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn dropped_resolver_owner_does_not_allow_attempt_cleanup_before_native_callback() {
        use super::super::{Adapter, WifiLease};
        let lease = Arc::new(
            WifiLease::from_adapters(&[Adapter {
                identity: Identity { index: 4, luid: 40 },
                ieee80211: true,
                operational: true,
                connected: true,
            }])
            .unwrap(),
        );
        let (query, callback) = start();
        lease.track_pending(Arc::clone(&query)).unwrap();
        drop(query); // Equivalent to the resolver future losing its own query reference.
        let closing = Arc::clone(&lease);
        let (done, observed) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let result = closing.finish();
            done.send(result).unwrap();
        });
        assert!(
            observed.recv_timeout(Duration::from_millis(30)).is_err(),
            "cleanup must wait for native terminal, not cancel acknowledgement"
        );
        callback.complete(|| {});
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(2)).unwrap(),
            Ok(())
        );
        worker.join().unwrap();
    }
    #[test]
    fn attempt_pending_tracking_is_bounded_to_its_two_address_queries() {
        use super::super::{Adapter, WifiLease};
        let lease = WifiLease::from_adapters(&[Adapter {
            identity: Identity { index: 4, luid: 40 },
            ieee80211: true,
            operational: true,
            connected: true,
        }])
        .unwrap();
        let (first, a) = start();
        let (second, b) = start();
        let (third, c) = start();
        lease.track_pending(first).unwrap();
        lease.track_pending(second).unwrap();
        assert_eq!(lease.track_pending(third), Err(NetworkError::Dns));
        a.complete(|| {});
        b.complete(|| {});
        c.complete(|| {});
        assert_eq!(lease.finish(), Ok(()));
    }
}
