#![allow(unused_imports)]
#[cfg(feature = "boring")]
pub use tokio_boring::SslStream as TlsStream;
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
pub use tokio_rustls::client::TlsStream;

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
pub use super::rustls::*;
#[cfg(feature = "boring")]
pub use crate::tls::boring::*;

#[cfg(not(any(feature = "boring", feature = "rustls-tls")))]
pub mod no_tls {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use http::Uri;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use crate::{
        client::{ConnectRequest, conn::Connection},
        error::BoxError,
    };

    pub struct TlsStream<T>(pub T);

    impl<T> TlsStream<T> {
        pub fn get_ref(&self) -> &T {
            &self.0
        }
        pub fn get_mut(&mut self) -> &mut T {
            &mut self.0
        }
    }

    pub struct HttpsConnector<T>(pub T);
    pub struct TlsConnector;
    pub struct TlsConnectorBuilder;

    pub struct EstablishedConn<IO> {
        pub io: IO,
        pub req: ConnectRequest,
    }

    impl<IO> EstablishedConn<IO> {
        pub fn new(io: IO, req: ConnectRequest) -> Self {
            Self { io, req }
        }
    }

    impl<T> Clone for HttpsConnector<T>
    where
        T: Clone,
    {
        fn clone(&self) -> Self {
            HttpsConnector(self.0.clone())
        }
    }

    impl<T> HttpsConnector<T> {
        pub fn with_connector(connector: T, _tls: TlsConnector) -> Self {
            HttpsConnector(connector)
        }
        pub fn no_alpn(&mut self) {}
    }

    // Implement Service<Uri>
    impl<T> tower::Service<Uri> for HttpsConnector<T>
    where
        T: tower::Service<Uri>,
        T::Future: Send + 'static,
        T::Error: Into<BoxError> + Send + 'static,
        T::Response: Send + 'static,
    {
        type Response = MaybeHttpsStream<T::Response>;
        type Error = BoxError;
        type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.0.poll_ready(cx).map_err(Into::into)
        }

        fn call(&mut self, req: Uri) -> Self::Future {
            let fut = self.0.call(req);
            Box::pin(async move {
                let stream = fut.await.map_err(Into::into)?;
                Ok(MaybeHttpsStream::Http(stream))
            })
        }
    }

    // Implement Service<ConnectRequest>
    impl<T> tower::Service<ConnectRequest> for HttpsConnector<T>
    where
        T: tower::Service<Uri>,
        T::Future: Send + 'static,
        T::Error: Into<BoxError> + Send + 'static,
        T::Response: Send + 'static,
    {
        type Response = MaybeHttpsStream<T::Response>;
        type Error = BoxError;
        type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.0.poll_ready(cx).map_err(Into::into)
        }

        fn call(&mut self, req: ConnectRequest) -> Self::Future {
            let fut = self.0.call(req.uri().clone());
            Box::pin(async move {
                let stream = fut.await.map_err(Into::into)?;
                Ok(MaybeHttpsStream::Http(stream))
            })
        }
    }

    // Implement Service<EstablishedConn<IO>>
    impl<T, IO> tower::Service<EstablishedConn<IO>> for HttpsConnector<T>
    where
        T: Clone + Send,
        IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        type Response = MaybeHttpsStream<IO>;
        type Error = BoxError;
        type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, conn: EstablishedConn<IO>) -> Self::Future {
            let io = conn.io;
            Box::pin(async move { Ok(MaybeHttpsStream::Http(io)) })
        }
    }

    impl TlsConnector {
        pub fn builder() -> TlsConnectorBuilder {
            TlsConnectorBuilder
        }
    }

    impl Clone for TlsConnector {
        fn clone(&self) -> Self {
            TlsConnector
        }
    }

    impl TlsConnectorBuilder {
        pub fn build(&self, _opts: &crate::tls::TlsOptions) -> Result<TlsConnector, crate::Error> {
            Ok(TlsConnector)
        }
        pub fn alpn_protocol(self, _proto: Option<crate::tls::AlpnProtocol>) -> Self {
            self
        }
        pub fn max_version(self, _version: Option<crate::tls::TlsVersion>) -> Self {
            self
        }
        pub fn min_version(self, _version: Option<crate::tls::TlsVersion>) -> Self {
            self
        }
        pub fn tls_sni(self, _enable: bool) -> Self {
            self
        }
        pub fn verify_hostname(self, _verify: bool) -> Self {
            self
        }
        pub fn cert_verification(self, _verify: bool) -> Self {
            self
        }
        pub fn cert_store(self, _store: crate::tls::CertStore) -> Self {
            self
        }
        pub fn identity(self, _identity: Option<crate::tls::Identity>) -> Self {
            self
        }
        pub fn keylog(self, _keylog: Option<crate::tls::KeyLog>) -> Self {
            self
        }
    }

    use crate::client::conn::TlsInfoFactory;
    impl<T> TlsInfoFactory for TlsStream<T> {
        fn tls_info(&self) -> Option<crate::tls::TlsInfo> {
            None
        }
    }

    impl<T: AsyncRead + Unpin> AsyncRead for TlsStream<T> {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
        }
    }

    impl<T: AsyncWrite + Unpin> AsyncWrite for TlsStream<T> {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.get_mut().0).poll_flush(cx)
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
        }
    }

    pub enum MaybeHttpsStream<T> {
        Http(T),
        Https(TlsStream<T>),
    }

    impl<T> MaybeHttpsStream<T> {
        pub fn get_ref(&self) -> &T {
            match self {
                MaybeHttpsStream::Http(s) => s,
                MaybeHttpsStream::Https(s) => &s.0,
            }
        }
        pub fn get_mut(&mut self) -> &mut T {
            match self {
                MaybeHttpsStream::Http(s) => s,
                MaybeHttpsStream::Https(s) => &mut s.0,
            }
        }
    }

    impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for MaybeHttpsStream<T> {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            match self.get_mut() {
                MaybeHttpsStream::Http(s) => Pin::new(s).poll_read(cx, buf),
                MaybeHttpsStream::Https(s) => Pin::new(&mut s.0).poll_read(cx, buf),
            }
        }
    }

    impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for MaybeHttpsStream<T> {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            match self.get_mut() {
                MaybeHttpsStream::Http(s) => Pin::new(s).poll_write(cx, buf),
                MaybeHttpsStream::Https(s) => Pin::new(&mut s.0).poll_write(cx, buf),
            }
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            match self.get_mut() {
                MaybeHttpsStream::Http(s) => Pin::new(s).poll_flush(cx),
                MaybeHttpsStream::Https(s) => Pin::new(&mut s.0).poll_flush(cx),
            }
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            match self.get_mut() {
                MaybeHttpsStream::Http(s) => Pin::new(s).poll_shutdown(cx),
                MaybeHttpsStream::Https(s) => Pin::new(&mut s.0).poll_shutdown(cx),
            }
        }
    }

    impl<T: Connection> Connection for MaybeHttpsStream<T> {
        fn connected(&self) -> crate::client::conn::Connected {
            match self {
                MaybeHttpsStream::Http(s) => s.connected(),
                MaybeHttpsStream::Https(s) => s.0.connected(),
            }
        }
    }
}

#[cfg(not(any(feature = "boring", feature = "rustls-tls")))]
pub use self::no_tls::*;
