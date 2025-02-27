//! Listener trait and it's implements.
use std::fs::File;
use std::io::{self, Error as IoError, ErrorKind, Read};
use std::net::{IpAddr, SocketAddr as StdSocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::server::accept::Accept;
use hyper::server::conn::AddrIncoming;
use hyper::server::conn::AddrStream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::addr::SocketAddr;
use crate::transport::Transport;

cfg_feature! {
    #![feature = "acme"]
    pub mod acme;
}
cfg_feature! {
    #![feature = "native-tls"]
    pub mod native_tls;
}
cfg_feature! {
    #![feature = "rustls"]
    pub mod rustls;
}
cfg_feature! {
    #![unix]
    pub mod unix;
}

cfg_feature! {
    #![feature = "acme"]
    pub use acme::AcmeListener;
}
cfg_feature! {
    #![feature = "native-tls"]
pub use native_tls::NativeTlsListener;
}
cfg_feature! {
    #![feature = "rustls"]
pub use rustls::RustlsListener;
}
cfg_feature! {
    #![unix]
pub use unix::UnixListener;
}

/// Listener trait
pub trait Listener: Accept {
    /// Join current Listener with the other.
    #[inline]
    fn join<T>(self, other: T) -> JoinedListener<Self, T>
    where
        Self: Sized,
    {
        JoinedListener::new(self, other)
    }
}

/// A I/O stream for JoinedListener.
pub enum JoinedStream<A, B> {
    #[allow(missing_docs)]
    A(A),
    #[allow(missing_docs)]
    B(B),
}

impl<A, B> AsyncRead for JoinedStream<A, B>
where
    A: AsyncRead + Send + Unpin + 'static,
    B: AsyncRead + Send + Unpin + 'static,
{
    #[inline]
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut() {
            JoinedStream::A(a) => Pin::new(a).poll_read(cx, buf),
            JoinedStream::B(b) => Pin::new(b).poll_read(cx, buf),
        }
    }
}

impl<A, B> AsyncWrite for JoinedStream<A, B>
where
    A: AsyncWrite + Send + Unpin + 'static,
    B: AsyncWrite + Send + Unpin + 'static,
{
    #[inline]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match &mut self.get_mut() {
            JoinedStream::A(a) => Pin::new(a).poll_write(cx, buf),
            JoinedStream::B(b) => Pin::new(b).poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut() {
            JoinedStream::A(a) => Pin::new(a).poll_flush(cx),
            JoinedStream::B(b) => Pin::new(b).poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut() {
            JoinedStream::A(a) => Pin::new(a).poll_shutdown(cx),
            JoinedStream::B(b) => Pin::new(b).poll_shutdown(cx),
        }
    }
}
impl<A, B> Transport for JoinedStream<A, B>
where
    A: Transport + Send + Unpin + 'static,
    B: Transport + Send + Unpin + 'static,
{
    #[inline]
    fn remote_addr(&self) -> Option<SocketAddr> {
        match self {
            JoinedStream::A(stream) => stream.remote_addr(),
            JoinedStream::B(stream) => stream.remote_addr(),
        }
    }
}

/// JoinedListener
pub struct JoinedListener<A, B> {
    a: A,
    b: B,
}

impl<A, B> JoinedListener<A, B> {
    #[inline]
    pub(crate) fn new(a: A, b: B) -> Self {
        JoinedListener { a, b }
    }
}
impl<A, B> Listener for JoinedListener<A, B>
where
    A: Accept + Send + Unpin + 'static,
    B: Accept + Send + Unpin + 'static,
    A::Conn: Transport,
    B::Conn: Transport,
{
}
impl<A, B> Accept for JoinedListener<A, B>
where
    A: Accept + Send + Unpin + 'static,
    B: Accept + Send + Unpin + 'static,
    A::Conn: Transport,
    B::Conn: Transport,
{
    type Conn = JoinedStream<A::Conn, B::Conn>;
    type Error = IoError;

    #[inline]
    fn poll_accept(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let pin = self.get_mut();
        if fastrand::bool() {
            match Pin::new(&mut pin.a).poll_accept(cx) {
                Poll::Ready(Some(result)) => Poll::Ready(Some(
                    result.map(JoinedStream::A).map_err(|_| IoError::from(ErrorKind::Other)),
                )),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => match Pin::new(&mut pin.b).poll_accept(cx) {
                    Poll::Ready(Some(result)) => Poll::Ready(Some(
                        result.map(JoinedStream::B).map_err(|_| IoError::from(ErrorKind::Other)),
                    )),
                    Poll::Ready(None) => Poll::Ready(None),
                    Poll::Pending => Poll::Pending,
                },
            }
        } else {
            match Pin::new(&mut pin.b).poll_accept(cx) {
                Poll::Ready(Some(result)) => Poll::Ready(Some(
                    result.map(JoinedStream::B).map_err(|_| IoError::from(ErrorKind::Other)),
                )),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => match Pin::new(&mut pin.a).poll_accept(cx) {
                    Poll::Ready(Some(result)) => Poll::Ready(Some(
                        result.map(JoinedStream::A).map_err(|_| IoError::from(ErrorKind::Other)),
                    )),
                    Poll::Ready(None) => Poll::Ready(None),
                    Poll::Pending => Poll::Pending,
                },
            }
        }
    }
}

/// TcpListener
pub struct TcpListener {
    incoming: AddrIncoming,
}
impl TcpListener {
    /// Get the [`AddrIncoming] of this listener.
    #[inline]
    pub fn incoming(&self) -> &AddrIncoming {
        &self.incoming
    }

    /// Get the local address bound to this listener.
    #[inline]
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.incoming.local_addr()
    }

    /// Bind to socket address.
    #[inline]
    pub fn bind(incoming: impl IntoAddrIncoming) -> Self {
        Self::try_bind(incoming).unwrap()
    }

    /// Try to bind to socket address.
    #[inline]
    pub fn try_bind(incoming: impl IntoAddrIncoming) -> Result<Self, hyper::Error> {
        Ok(TcpListener {
            incoming: incoming.into_incoming(),
        })
    }
}
impl Listener for TcpListener {}
impl Accept for TcpListener {
    type Conn = AddrStream;
    type Error = IoError;

    #[inline]
    fn poll_accept(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        Pin::new(&mut self.get_mut().incoming).poll_accept(cx)
    }
}

/// IntoAddrIncoming
pub trait IntoAddrIncoming {
    /// Convert into AddrIncoming
    fn into_incoming(self) -> AddrIncoming;
}

impl IntoAddrIncoming for StdSocketAddr {
    #[inline]
    fn into_incoming(self) -> AddrIncoming {
        let mut incoming = AddrIncoming::bind(&self).unwrap();
        incoming.set_nodelay(true);
        incoming
    }
}

impl IntoAddrIncoming for AddrIncoming {
    #[inline]
    fn into_incoming(self) -> AddrIncoming {
        self
    }
}

impl<T: ToSocketAddrs + ?Sized> IntoAddrIncoming for &T {
    #[inline]
    fn into_incoming(self) -> AddrIncoming {
        for addr in self.to_socket_addrs().expect("failed to create AddrIncoming") {
            if let Ok(mut incoming) = AddrIncoming::bind(&addr) {
                incoming.set_nodelay(true);
                return incoming;
            }
        }
        panic!("failed to create AddrIncoming");
    }
}

impl<I: Into<IpAddr>> IntoAddrIncoming for (I, u16) {
    #[inline]
    fn into_incoming(self) -> AddrIncoming {
        let mut incoming = AddrIncoming::bind(&self.into()).expect("failed to create AddrIncoming");
        incoming.set_nodelay(true);
        incoming
    }
}

pub(crate) struct LazyFile {
    path: PathBuf,
    file: Option<File>,
}

impl LazyFile {
    #[inline]
    fn lazy_read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.file.is_none() {
            self.file = Some(File::open(&self.path)?);
        }

        self.file.as_mut().unwrap().read(buf)
    }
}
impl Read for LazyFile {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.lazy_read(buf).map_err(|e| {
            let kind = e.kind();
            tracing::error!(path = ?self.path, error = ?e, "error reading file");
            IoError::new(kind, format!("error reading file ({:?}): {}", self.path.display(), e))
        })
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{Stream, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use super::*;

    impl Stream for TcpListener {
        type Item = Result<AddrStream, IoError>;
        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.poll_accept(cx)
        }
    }

    impl<A, B> Stream for JoinedListener<A, B>
    where
        A: Accept + Send + Unpin + 'static,
        B: Accept + Send + Unpin + 'static,
        A::Conn: Transport,
        B::Conn: Transport,
    {
        type Item = Result<JoinedStream<A::Conn, B::Conn>, IoError>;
        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.poll_accept(cx)
        }
    }

    #[tokio::test]
    async fn test_tcp_listener() {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 6878));

        let mut listener = TcpListener::bind(addr);
        tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream.write_i32(150).await.unwrap();
        });

        let mut stream = listener.next().await.unwrap().unwrap();
        assert_eq!(stream.read_i32().await.unwrap(), 150);
    }

    #[tokio::test]
    async fn test_joined_listener() {
        let addr1 = std::net::SocketAddr::from(([127, 0, 0, 1], 6978));
        let addr2 = std::net::SocketAddr::from(([127, 0, 0, 1], 6979));

        let mut listener = TcpListener::bind(addr1).join(TcpListener::bind(addr2));
        tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr1).await.unwrap();
            stream.write_i32(50).await.unwrap();

            let mut stream = TcpStream::connect(addr2).await.unwrap();
            stream.write_i32(100).await.unwrap();
        });
        let mut stream = listener.next().await.unwrap().unwrap();
        let first = stream.read_i32().await.unwrap();
        let mut stream = listener.next().await.unwrap().unwrap();
        let second = stream.read_i32().await.unwrap();
        assert_eq!(first + second, 150);
    }
}
