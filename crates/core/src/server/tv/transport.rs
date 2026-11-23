//! Bound socket ownership like the existing LAN listener, without a TLS downgrade path.
use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
pub(super) struct Listener {
    socket: TcpListener,
    permits: Arc<Semaphore>,
    stop: CancellationToken,
}
impl Listener {
    pub fn new(socket: TcpListener, stop: CancellationToken) -> Self {
        Self {
            socket,
            permits: Arc::new(Semaphore::new(64)),
            stop,
        }
    }
}
impl axum::serve::Listener for Listener {
    type Io = Stream;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Stream, SocketAddr) {
        loop {
            let (socket, address) = match self.socket.accept().await {
                Ok(value) => value,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
                continue;
            };
            return (
                Stream {
                    socket,
                    cancelled: Box::pin(self.stop.clone().cancelled_owned()),
                    _permit: permit,
                },
                address,
            );
        }
    }
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}
pub(super) struct Stream {
    socket: TcpStream,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
    _permit: OwnedSemaphorePermit,
}
fn aborted() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "TV serving stopped")
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(aborted()));
        }
        Pin::new(&mut self.socket).poll_read(cx, buffer)
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(aborted()));
        }
        Pin::new(&mut self.socket).poll_write(cx, buffer)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(aborted()));
        }
        Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(aborted()));
        }
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}
#[derive(Clone, Copy)]
pub(super) struct Peer(pub SocketAddr);
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, Listener>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, Listener>) -> Self {
        Self(*stream.remote_addr())
    }
}
