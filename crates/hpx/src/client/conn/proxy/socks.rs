use std::{
    borrow::Cow,
    net::{IpAddr, SocketAddr},
    task::{Context, Poll},
};

use bytes::Bytes;
use http::Uri;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_socks::{
    TargetAddr,
    tcp::{Socks4Stream, Socks5Stream},
};
use tower::Service;

use super::Tunneling;
use crate::{
    dns::{GaiResolver, InternalResolve, Name},
    error::BoxError,
    ext::UriExt,
};

#[derive(Debug)]
pub(crate) enum SocksError {
    ConnectFailed(BoxError),
    DnsResolveFailure(BoxError),
    Socks(tokio_socks::Error),
    Io(std::io::Error),
    Utf8(std::str::Utf8Error),
    DnsFailure,
    MissingHost,
}

impl std::fmt::Display for SocksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SOCKS error: ")?;

        match self {
            Self::ConnectFailed(e) => {
                f.write_fmt(format_args!("failed to create underlying connection: {e}"))
            }
            Self::Socks(e) => f.write_fmt(format_args!("error during SOCKS handshake: {e}")),
            Self::Io(e) => f.write_fmt(format_args!("io error during SOCKS handshake: {e}")),
            Self::Utf8(e) => f.write_fmt(format_args!(
                "invalid UTF-8 during SOCKS authentication: {e}"
            )),
            Self::DnsResolveFailure(e) => {
                f.write_fmt(format_args!("failed to resolve DNS for SOCKS target: {e}"))
            }
            Self::DnsFailure => f.write_str("could not resolve to acceptable address type"),
            Self::MissingHost => f.write_str("missing destination host"),
        }
    }
}

impl std::error::Error for SocksError {}

impl From<std::io::Error> for SocksError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<std::str::Utf8Error> for SocksError {
    fn from(err: std::str::Utf8Error) -> Self {
        Self::Utf8(err)
    }
}

impl From<tokio_socks::Error> for SocksError {
    fn from(err: tokio_socks::Error) -> Self {
        Self::Socks(err)
    }
}

/// Represents the SOCKS protocol version.
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Version {
    V4,
    V5,
}

/// Represents the DNS resolution strategy for SOCKS connections.
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum DnsResolve {
    Local,
    Remote,
}

/// A connector that establishes connections through a SOCKS proxy.
pub(crate) struct SocksConnector<C, R = GaiResolver> {
    inner: C,
    resolver: R,
    proxy_dst: Uri,
    auth: Option<(Bytes, Bytes)>,
    version: Version,
    dns_resolve: DnsResolve,
}

impl<C, R> SocksConnector<C, R>
where
    R: InternalResolve + Clone,
{
    /// Create a new SOCKS connector with the given inner service.
    ///
    /// This wraps an underlying connector, and stores the address of a
    /// SOCKS proxy server.
    ///
    /// A `SocksConnector` can then be called with any destination. The `proxy_dst` passed to
    /// `call` will not be used to create the underlying connection, but will
    /// be used in a SOCKS handshake sent to the proxy destination.
    pub(crate) const fn new_with_resolver(proxy_dst: Uri, inner: C, resolver: R) -> Self {
        Self {
            inner,
            resolver,
            proxy_dst,
            version: Version::V5,
            dns_resolve: DnsResolve::Local,
            auth: None,
        }
    }

    /// Sets the authentication credentials for the SOCKS proxy connection.
    #[inline]
    pub(crate) fn set_auth(&mut self, auth: Option<(Bytes, Bytes)>) {
        self.auth = auth;
    }

    /// Sets whether to use the SOCKS5 protocol for the proxy connection.
    #[inline]
    pub(crate) const fn set_version(&mut self, version: Version) {
        self.version = version;
    }

    /// Sets whether to resolve DNS locally or let the proxy handle DNS resolution.
    #[inline]
    pub(crate) const fn set_dns_mode(&mut self, dns_resolve: DnsResolve) {
        self.dns_resolve = dns_resolve;
    }
}

impl<C, R> Service<Uri> for SocksConnector<C, R>
where
    C: Service<Uri>,
    C::Future: Send + 'static,
    C::Response: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C::Error: Into<BoxError>,
    R: InternalResolve + Clone + Send + 'static,
    <R as InternalResolve>::Future: Send + 'static,
{
    type Response = C::Response;
    type Error = SocksError;
    type Future = Tunneling<C::Future, C::Response, Self::Error>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner
            .poll_ready(cx)
            .map_err(Into::into)
            .map_err(SocksError::ConnectFailed)
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let connecting = self.inner.call(self.proxy_dst.clone());

        let version = self.version;
        let dns_resolve = self.dns_resolve;
        let auth = self.auth.clone();
        let mut resolver = self.resolver.clone();

        let fut = async move {
            let host = dst.host().ok_or(SocksError::MissingHost)?;
            // `Uri::host` keeps the brackets of an IPv6 literal, which neither
            // a resolver nor a SOCKS proxy accepts as a name.
            let host = host
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or(host);
            let port = dst.port_or_default();

            // Attempt to tcp connect to the proxy server.
            // This will return a `tokio::net::TcpStream` if successful.
            let socket = connecting
                .await
                .map_err(Into::into)
                .map_err(SocksError::ConnectFailed)?;

            // Resolve the target address using the provided resolver. An IP
            // literal is sent as an address: there is nothing to resolve,
            // locally or by the proxy.
            let target_addr = if let Ok(ip) = host.parse::<IpAddr>() {
                TargetAddr::Ip(SocketAddr::new(ip, port))
            } else {
                match dns_resolve {
                    DnsResolve::Local => {
                        let mut addrs = resolver
                            .resolve(Name::new(host.into()))
                            .await
                            .map_err(Into::into)
                            .map_err(SocksError::DnsResolveFailure)?;
                        // SOCKS4 carries only IPv4 addresses, so an IPv6
                        // address listed first must not hide an IPv4 one.
                        let mut socket_addr = match version {
                            Version::V4 => addrs.find(SocketAddr::is_ipv4),
                            Version::V5 => addrs.next(),
                        }
                        .ok_or(SocksError::DnsFailure)?;
                        socket_addr.set_port(port);
                        TargetAddr::Ip(socket_addr)
                    }
                    DnsResolve::Remote => TargetAddr::Domain(Cow::Borrowed(host), port),
                }
            };

            match version {
                Version::V4 => {
                    // For SOCKS4, we connect directly to the target address.
                    let stream = Socks4Stream::connect_with_socket(socket, target_addr).await?;
                    Ok(stream.into_inner())
                }
                Version::V5 => {
                    // For SOCKS5, we need to handle authentication if provided.
                    // The `auth` is an optional tuple of (username, password).
                    let stream = match auth {
                        Some((username, password)) => {
                            let username = std::str::from_utf8(&username)?;
                            let password = std::str::from_utf8(&password)?;
                            Socks5Stream::connect_with_password_and_socket(
                                socket,
                                target_addr,
                                username,
                                password,
                            )
                            .await?
                        }
                        None => Socks5Stream::connect_with_socket(socket, target_addr).await?,
                    };
                    Ok(stream.into_inner())
                }
            }
        };

        Tunneling {
            fut: Box::pin(fut),
            _marker: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use super::*;

    /// The destination a mock proxy was asked to connect to.
    #[derive(Debug, PartialEq, Eq)]
    enum Requested {
        Ip(SocketAddr),
        Domain(String, u16),
    }

    async fn read_port(stream: &mut TcpStream) -> u16 {
        let mut port = [0u8; 2];
        stream.read_exact(&mut port).await.unwrap();
        u16::from_be_bytes(port)
    }

    /// Reads a NUL-terminated SOCKS4 string.
    async fn read_nul_terminated(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        loop {
            let byte = stream.read_u8().await.unwrap();
            if byte == 0 {
                return String::from_utf8(bytes).unwrap();
            }
            bytes.push(byte);
        }
    }

    /// Accepts one SOCKS CONNECT request, grants it, and reports its target.
    async fn mock_proxy(listener: TcpListener, version: Version) -> Requested {
        let (mut stream, _) = listener.accept().await.unwrap();

        match version {
            Version::V4 => {
                let mut head = [0u8; 4];
                stream.read_exact(&mut head).await.unwrap();
                assert_eq!(head[..2], [0x04, 0x01]);
                let port = u16::from_be_bytes([head[2], head[3]]);
                let mut ip = [0u8; 4];
                stream.read_exact(&mut ip).await.unwrap();
                let _user_id = read_nul_terminated(&mut stream).await;
                // SOCKS4a: `0.0.0.x` announces a domain name.
                let requested = if ip[..3] == [0, 0, 0] && ip[3] != 0 {
                    Requested::Domain(read_nul_terminated(&mut stream).await, port)
                } else {
                    Requested::Ip(SocketAddr::new(Ipv4Addr::from(ip).into(), port))
                };
                stream
                    .write_all(&[0x00, 0x5A, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
                requested
            }
            Version::V5 => {
                let mut greeting = [0u8; 2];
                stream.read_exact(&mut greeting).await.unwrap();
                let mut methods = vec![0u8; usize::from(greeting[1])];
                stream.read_exact(&mut methods).await.unwrap();
                stream.write_all(&[0x05, 0x00]).await.unwrap();

                let mut head = [0u8; 4];
                stream.read_exact(&mut head).await.unwrap();
                assert_eq!(head[..3], [0x05, 0x01, 0x00]);
                let requested = match head[3] {
                    0x01 => {
                        let mut ip = [0u8; 4];
                        stream.read_exact(&mut ip).await.unwrap();
                        let port = read_port(&mut stream).await;
                        Requested::Ip(SocketAddr::new(Ipv4Addr::from(ip).into(), port))
                    }
                    0x04 => {
                        let mut ip = [0u8; 16];
                        stream.read_exact(&mut ip).await.unwrap();
                        let port = read_port(&mut stream).await;
                        Requested::Ip(SocketAddr::new(Ipv6Addr::from(ip).into(), port))
                    }
                    0x03 => {
                        let len = stream.read_u8().await.unwrap();
                        let mut name = vec![0u8; usize::from(len)];
                        stream.read_exact(&mut name).await.unwrap();
                        let port = read_port(&mut stream).await;
                        Requested::Domain(String::from_utf8(name).unwrap(), port)
                    }
                    other => panic!("unexpected address type {other}"),
                };
                stream
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
                requested
            }
        }
    }

    /// Connects to `dst` through a mock proxy and returns what it was asked
    /// to connect to.
    ///
    /// The resolver only knows `example.test`, for which it lists an IPv6
    /// address before an IPv4 one; any other name fails to resolve, the way
    /// a bracketed IPv6 literal fails with a real resolver.
    async fn requested_target(dst: &str, version: Version, dns_resolve: DnsResolve) -> Requested {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let proxy = tokio::spawn(mock_proxy(listener, version));

        let inner = tower::service_fn(move |_: Uri| TcpStream::connect(proxy_addr));
        let resolver = tower::service_fn(|name: Name| async move {
            if name.as_str() == "example.test" {
                Ok(vec![
                    SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                ]
                .into_iter())
            } else {
                Err(BoxError::from(format!("cannot resolve {name}")))
            }
        });

        let proxy_uri = format!("socks5://{proxy_addr}").parse().unwrap();
        let mut connector = SocksConnector::new_with_resolver(proxy_uri, inner, resolver);
        connector.set_version(version);
        connector.set_dns_mode(dns_resolve);
        connector.call(dst.parse::<Uri>().unwrap()).await.unwrap();

        proxy.await.unwrap()
    }

    #[tokio::test]
    async fn ipv6_literal_is_sent_as_an_address_with_local_dns() {
        let requested =
            requested_target("http://[2001:db8::1]:8080/", Version::V5, DnsResolve::Local).await;
        assert_eq!(
            requested,
            Requested::Ip("[2001:db8::1]:8080".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn ipv6_literal_is_sent_as_an_address_with_remote_dns() {
        let requested =
            requested_target("https://[2001:db8::1]/", Version::V5, DnsResolve::Remote).await;
        assert_eq!(
            requested,
            Requested::Ip("[2001:db8::1]:443".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn ipv4_literal_is_sent_as_an_address_with_remote_dns() {
        let requested =
            requested_target("http://192.0.2.1:8080/", Version::V4, DnsResolve::Remote).await;
        assert_eq!(requested, Requested::Ip("192.0.2.1:8080".parse().unwrap()));
    }

    #[tokio::test]
    async fn socks4_uses_an_ipv4_address_from_local_dns() {
        let requested =
            requested_target("http://example.test:8080/", Version::V4, DnsResolve::Local).await;
        assert_eq!(requested, Requested::Ip("127.0.0.1:8080".parse().unwrap()));
    }

    #[tokio::test]
    async fn socks5_uses_the_first_address_from_local_dns() {
        let requested =
            requested_target("http://example.test:8080/", Version::V5, DnsResolve::Local).await;
        assert_eq!(requested, Requested::Ip("[::1]:8080".parse().unwrap()));
    }

    #[tokio::test]
    async fn domain_is_sent_by_name_with_remote_dns() {
        let requested =
            requested_target("http://example.test:8080/", Version::V5, DnsResolve::Remote).await;
        assert_eq!(
            requested,
            Requested::Domain("example.test".to_owned(), 8080)
        );
    }
}
