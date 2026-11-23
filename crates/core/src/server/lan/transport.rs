use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

const CONNECTIONS: usize = 64;
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(5);
type Pending = Pin<Box<dyn Future<Output = Option<(Stream, SocketAddr)>> + Send>>;

pub(super) struct Listener {
    socket: TcpListener,
    acceptor: TlsAcceptor,
    permits: Arc<Semaphore>,
    pending: FuturesUnordered<Pending>,
    stop: CancellationToken,
}

impl Listener {
    pub fn new(
        socket: TcpListener,
        config: Arc<rustls::ServerConfig>,
        stop: CancellationToken,
    ) -> Self {
        Self {
            socket,
            acceptor: config.into(),
            permits: Arc::new(Semaphore::new(CONNECTIONS)),
            pending: FuturesUnordered::new(),
            stop,
        }
    }
}

impl axum::serve::Listener for Listener {
    type Io = Stream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Stream, SocketAddr) {
        loop {
            tokio::select! {
                completed = self.pending.next(), if !self.pending.is_empty() => {
                    if let Some(Some(stream)) = completed { return stream; }
                }
                accepted = self.socket.accept() => {
                    let (socket, peer) = match accepted {
                        Ok(accepted) => accepted,
                        Err(_) => { tokio::time::sleep(Duration::from_millis(50)).await; continue; }
                    };
                    let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else { continue; };
                    let acceptor = self.acceptor.clone();
                    let connection = Arc::new(Connection { stop: self.stop.child_token(), active: Mutex::new(None) });
                    let cancelled = Box::pin(connection.stop.clone().cancelled_owned());
                    self.pending.push(Box::pin(async move {
                        let inner = tokio::time::timeout(HANDSHAKE_LIMIT, acceptor.accept(socket)).await.ok()?.ok()?;
                        Some((Stream { inner, cancelled, connection, _permit: permit }, peer))
                    }));
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

pub(super) struct Stream {
    inner: TlsStream<TcpStream>,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
    connection: Arc<Connection>,
    _permit: OwnedSemaphorePermit,
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.is_cancelled(cx) {
            return Poll::Ready(Err(aborted()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.is_cancelled(cx) {
            return Poll::Ready(Err(aborted()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.is_cancelled(cx) {
            return Poll::Ready(Err(aborted()));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.is_cancelled(cx) {
            return Poll::Ready(Err(aborted()));
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl Stream {
    fn is_cancelled(&mut self, cx: &mut Context<'_>) -> bool {
        self.cancelled.as_mut().poll(cx).is_ready() || self.connection.poll_revoked(cx)
    }
}

pub(super) struct ResponseLease;

struct ResponseWatch {
    lease: Weak<ResponseLease>,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
}

pub(super) struct Connection {
    stop: CancellationToken,
    active: Mutex<Option<ResponseWatch>>,
}

impl Connection {
    pub fn attach(&self, revoked: CancellationToken) -> std::io::Result<Arc<ResponseLease>> {
        let mut active = self.active.lock().map_err(|_| aborted())?;
        // The listener serves HTTP/1 only, with one live response per connection.
        if active
            .as_ref()
            .is_some_and(|watch| watch.lease.strong_count() != 0)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "LAN response is already active",
            ));
        }
        let lease = Arc::new(ResponseLease);
        *active = Some(ResponseWatch {
            lease: Arc::downgrade(&lease),
            cancelled: Box::pin(revoked.cancelled_owned()),
        });
        Ok(lease)
    }

    fn poll_revoked(&self, cx: &mut Context<'_>) -> bool {
        let Ok(mut active) = self.active.lock() else {
            return true;
        };
        let Some(watch) = active.as_mut() else {
            return false;
        };
        let Some(_lease) = watch.lease.upgrade() else {
            *active = None;
            return false;
        };
        if watch.cancelled.as_mut().poll(cx).is_ready() {
            self.stop.cancel();
            true
        } else {
            false
        }
    }
}

fn aborted() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        "LAN listener stopped",
    )
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;

#[derive(Clone)]
pub(super) struct Peer {
    pub address: SocketAddr,
    pub connection: Arc<Connection>,
}

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, Listener>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, Listener>) -> Self {
        Self {
            address: *stream.remote_addr(),
            connection: Arc::clone(&stream.io().connection),
        }
    }
}
