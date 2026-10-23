use std::{
    io,
    marker::{PhantomData, Unpin},
    pin::Pin,
    task::{self, Poll, ready},
};

use http::{HeaderMap, HeaderValue, Uri, header::PROXY_AUTHENTICATE};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tower::Service;

use super::Tunneling;
use crate::{
    error::BoxError,
    ext::UriExt,
    proxy::auth::{Authenticator, Challenge, ProxyCredentials, parse_challenges},
};

/// The maximum number of `CONNECT` attempts, including the first one.
///
/// NTLM needs three legs (negotiate, challenge, authenticate), so more than
/// one attempt is possible.
const MAX_ATTEMPTS: usize = 4;

/// The maximum size of the response head, in bytes.
const MAX_HEAD_SIZE: usize = 8192;

/// Tunnel Proxy via HTTP CONNECT
///
/// This is a connector that can be used by the `Client`. It wraps
/// another connector, and after getting an underlying connection, it creates
/// an HTTP CONNECT tunnel over it.
#[derive(Debug)]
pub(crate) struct TunnelConnector<C> {
    headers: Headers,
    /// The credentials used to answer a `407` challenge.
    credentials: Option<ProxyCredentials>,
    inner: C,
    proxy_dst: Uri,
}

#[derive(Clone, Debug)]
enum Headers {
    Empty,
    Auth(HeaderValue),
    Extra(HeaderMap),
}

#[derive(Debug)]
pub(crate) enum TunnelError {
    ConnectFailed(BoxError),
    Io(std::io::Error),
    MissingHost,
    ProxyAuthRequired,
    ProxyAuthFailed,
    ProxyHeadersTooLong,
    TunnelUnexpectedEof,
    TunnelUnsuccessful,
    MalformedResponse,
}

impl<C> TunnelConnector<C> {
    /// Create a new tunnel connector.
    ///
    /// This wraps an underlying connector, and stores the address of a
    /// tunneling proxy server.
    ///
    /// A `TunnelConnector` can then be called with any destination. The `proxy_dst` passed to
    /// `call` will not be used to create the underlying connection, but will
    /// be used in an HTTP CONNECT request sent to the proxy destination.
    pub(crate) const fn new(proxy_dst: Uri, connector: C) -> Self {
        Self {
            headers: Headers::Empty,
            credentials: None,
            inner: connector,
            proxy_dst,
        }
    }

    /// Add `proxy-authorization` header value to the CONNECT request.
    pub(crate) fn with_auth(mut self, mut auth: HeaderValue) -> Self {
        // just in case the user forgot
        auth.set_sensitive(true);
        match self.headers {
            Headers::Empty => {
                self.headers = Headers::Auth(auth);
            }
            Headers::Auth(ref mut existing) => {
                *existing = auth;
            }
            Headers::Extra(ref mut extra) => {
                extra.insert(http::header::PROXY_AUTHORIZATION, auth);
            }
        }

        self
    }

    /// Add the credentials used to answer a proxy authentication challenge.
    pub(crate) fn with_credentials(mut self, credentials: ProxyCredentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Add extra headers to be sent with the CONNECT request.
    ///
    /// If existing headers have been set, these will be merged.
    pub(crate) fn with_headers(mut self, mut headers: HeaderMap) -> Self {
        match self.headers {
            Headers::Empty => {
                self.headers = Headers::Extra(headers);
            }
            Headers::Auth(auth) => {
                headers
                    .entry(http::header::PROXY_AUTHORIZATION)
                    .or_insert(auth);
                self.headers = Headers::Extra(headers);
            }
            Headers::Extra(ref mut extra) => {
                extra.extend(headers);
            }
        }

        self
    }
}

impl<C> Service<Uri> for TunnelConnector<C>
where
    C: Service<Uri>,
    C::Future: Send + 'static,
    C::Response: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    C::Error: Into<BoxError>,
{
    type Response = C::Response;
    type Error = TunnelError;
    type Future = Tunneling<C::Future, C::Response, Self::Error>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner
            .poll_ready(cx)
            .map_err(Into::into)
            .map_err(TunnelError::ConnectFailed)
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let connecting = self.inner.call(self.proxy_dst.clone());
        let headers = self.headers.clone();
        let credentials = self.credentials.clone();

        Tunneling {
            fut: Box::pin(async move {
                let conn = connecting
                    .await
                    .map_err(Into::into)
                    .map_err(TunnelError::ConnectFailed)?;
                tunnel(
                    conn,
                    dst.host().ok_or(TunnelError::MissingHost)?,
                    dst.port_or_default(),
                    &headers,
                    credentials.as_ref(),
                )
                .await
            }),
            _marker: PhantomData,
        }
    }
}

/// Establishes the tunnel, answering proxy authentication challenges.
async fn tunnel<T>(
    mut conn: T,
    host: &str,
    port: u16,
    headers: &Headers,
    credentials: Option<&ProxyCredentials>,
) -> Result<T, TunnelError>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let mut authenticator = credentials.cloned().map(Authenticator::new);
    let mut authorization = preemptive_authorization(headers, authenticator.as_ref());

    for attempt in 0..MAX_ATTEMPTS {
        let request = connect_request(host, port, headers, authorization.as_ref());
        write_all(&mut conn, &request)
            .await
            .map_err(TunnelError::Io)?;

        let mut buf = Vec::new();
        let head = read_response_head(&mut conn, &mut buf).await?;

        match head.status {
            200 => {
                // The tunnel is established. Any bytes that arrived with the
                // response head belong to the tunnelled stream; the client
                // cannot have sent anything yet, so a well-behaved proxy
                // never sends any.
                return Ok(conn);
            }
            407 => {
                let Some(authenticator) = authenticator.as_mut() else {
                    return Err(TunnelError::ProxyAuthRequired);
                };

                if attempt + 1 == MAX_ATTEMPTS {
                    return Err(TunnelError::ProxyAuthFailed);
                }

                // Skip the challenge body so the connection is positioned at
                // the next message boundary.
                skip_body(&mut conn, &head, &buf).await?;

                let challenges = collect_challenges(&head.headers);
                let uri = format!("{host}:{port}");
                let response = authenticator
                    .respond(&challenges, "CONNECT", &uri)
                    .ok_or(TunnelError::ProxyAuthRequired)?;

                authorization = Some(response);
            }
            _ => return Err(TunnelError::TunnelUnsuccessful),
        }
    }

    Err(TunnelError::ProxyAuthFailed)
}

/// The `Proxy-Authorization` header value to send with the first request.
fn preemptive_authorization(
    headers: &Headers,
    authenticator: Option<&Authenticator>,
) -> Option<HeaderValue> {
    match headers {
        Headers::Auth(auth) => Some(auth.clone()),
        Headers::Extra(extra) => extra.get(http::header::PROXY_AUTHORIZATION).cloned(),
        Headers::Empty => authenticator.and_then(Authenticator::preemptive),
    }
}

/// Builds the CONNECT request bytes.
fn connect_request(
    host: &str,
    port: u16,
    headers: &Headers,
    authorization: Option<&HeaderValue>,
) -> Vec<u8> {
    let mut buf = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n").into_bytes();

    match headers {
        Headers::Auth(auth) => {
            buf.extend_from_slice(b"Proxy-Authorization: ");
            buf.extend_from_slice(auth.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        Headers::Extra(extra) => {
            for (name, value) in extra {
                buf.extend_from_slice(name.as_str().as_bytes());
                buf.extend_from_slice(b": ");
                buf.extend_from_slice(value.as_bytes());
                buf.extend_from_slice(b"\r\n");
            }
        }
        Headers::Empty => (),
    }

    if let Some(authorization) = authorization
        && !matches!(headers, Headers::Auth(_))
        && !headers
            .as_extra()
            .is_some_and(|extra| extra.contains_key(http::header::PROXY_AUTHORIZATION))
    {
        buf.extend_from_slice(b"Proxy-Authorization: ");
        buf.extend_from_slice(authorization.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }

    // headers end
    buf.extend_from_slice(b"\r\n");
    buf
}

/// The head of an HTTP response.
#[derive(Debug)]
struct ResponseHead {
    /// The response status code.
    status: u16,
    /// The response headers.
    headers: HeaderMap,
}

/// Reads a response head from the connection.
///
/// `buf` collects the bytes that were read; it may hold the beginning of
/// the response body.
async fn read_response_head<T>(conn: &mut T, buf: &mut Vec<u8>) -> Result<ResponseHead, TunnelError>
where
    T: AsyncRead + Unpin,
{
    let mut chunk = [0u8; 1024];

    loop {
        // The head may already be complete from a previous read.
        if let Some(end) = find_head_end(buf) {
            let head = buf[..end].to_vec();
            let rest = buf.split_off(end);
            *buf = rest;
            return parse_response_head(&head);
        }

        if buf.len() > MAX_HEAD_SIZE {
            return Err(TunnelError::ProxyHeadersTooLong);
        }

        let n = read(conn, &mut chunk).await.map_err(TunnelError::Io)?;
        if n == 0 {
            return Err(TunnelError::TunnelUnexpectedEof);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Returns the index just past the `\r\n\r\n` terminator, if present.
fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|i| i + 4)
}

/// Parses a response head into its status code and headers.
fn parse_response_head(head: &[u8]) -> Result<ResponseHead, TunnelError> {
    let text = std::str::from_utf8(head).map_err(|_| TunnelError::MalformedResponse)?;

    let mut lines = text.split("\r\n");
    let status_line = lines.next().ok_or(TunnelError::MalformedResponse)?;

    let mut parts = status_line.split_whitespace();
    let version = parts.next().ok_or(TunnelError::MalformedResponse)?;
    if !version.starts_with("HTTP/") {
        return Err(TunnelError::MalformedResponse);
    }
    let status: u16 = parts
        .next()
        .ok_or(TunnelError::MalformedResponse)?
        .parse()
        .map_err(|_| TunnelError::MalformedResponse)?;

    let mut headers = HeaderMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        let (Ok(name), Ok(value)) = (
            http::header::HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        headers.append(name, value);
    }

    Ok(ResponseHead { status, headers })
}

/// Collects the `Proxy-Authenticate` challenges from the response headers.
fn collect_challenges(headers: &HeaderMap) -> Vec<Challenge> {
    let values: Vec<&HeaderValue> = headers.get_all(PROXY_AUTHENTICATE).iter().collect();
    parse_challenges(&values)
}

/// Skips the body of a response so the connection is positioned at the next
/// message boundary.
async fn skip_body<T>(conn: &mut T, head: &ResponseHead, buf: &[u8]) -> Result<(), TunnelError>
where
    T: AsyncRead + Unpin,
{
    let chunked = head
        .headers
        .get(http::header::TRANSFER_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));

    if chunked {
        return skip_chunked_body(conn, buf).await;
    }

    let Some(length) = head
        .headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<usize>().ok())
    else {
        // A response without a length has no body.
        return Ok(());
    };

    // The body may already be in the buffer.
    let buffered = buf.len().min(length);
    let mut remaining = length - buffered;

    while remaining > 0 {
        let mut chunk = vec![0u8; remaining.min(4096)];
        let n = read(conn, &mut chunk).await.map_err(TunnelError::Io)?;
        if n == 0 {
            return Err(TunnelError::TunnelUnexpectedEof);
        }
        remaining -= n;
    }

    Ok(())
}

/// Skips a chunked response body.
async fn skip_chunked_body<T>(conn: &mut T, buf: &[u8]) -> Result<(), TunnelError>
where
    T: AsyncRead + Unpin,
{
    // The chunked body is read until the terminating zero-length chunk. The
    // bytes already buffered are consumed first.
    let mut pending: Vec<u8> = buf.to_vec();
    let mut chunk = [0u8; 1024];

    loop {
        // Find the size line in the pending bytes.
        let Some(index) = pending.windows(2).position(|window| window == b"\r\n") else {
            let n = read(conn, &mut chunk).await.map_err(TunnelError::Io)?;
            if n == 0 {
                return Err(TunnelError::TunnelUnexpectedEof);
            }
            pending.extend_from_slice(&chunk[..n]);
            continue;
        };

        let size_line = String::from_utf8_lossy(&pending[..index]).to_string();
        let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| TunnelError::MalformedResponse)?;

        // Consume the size line, the chunk data and the trailing CRLF.
        let mut consumed = index + 2;
        if size == 0 {
            return Ok(());
        }

        while pending.len() < consumed + size + 2 {
            let n = read(conn, &mut chunk).await.map_err(TunnelError::Io)?;
            if n == 0 {
                return Err(TunnelError::TunnelUnexpectedEof);
            }
            pending.extend_from_slice(&chunk[..n]);
        }

        consumed += size + 2;
        pending.drain(..consumed);
    }
}

async fn read<T>(io: &mut T, buf: &mut [u8]) -> io::Result<usize>
where
    T: AsyncRead + Unpin,
{
    std::future::poll_fn(move |cx| {
        let mut buf = ReadBuf::new(buf);
        ready!(Pin::new(&mut *io).poll_read(cx, &mut buf))?;
        Poll::Ready(Ok(buf.filled().len()))
    })
    .await
}

async fn write_all<T>(io: &mut T, buf: &[u8]) -> io::Result<()>
where
    T: AsyncWrite + Unpin,
{
    let mut n = 0;
    std::future::poll_fn(move |cx| {
        while n < buf.len() {
            n += ready!(Pin::new(&mut *io).poll_write(cx, &buf[n..])?);
        }
        Poll::Ready(Ok(()))
    })
    .await
}

impl std::fmt::Display for TunnelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel error: ")?;

        f.write_str(match self {
            Self::MissingHost => "missing destination host",
            Self::ProxyAuthRequired => "proxy authorization required",
            Self::ProxyAuthFailed => "proxy authentication failed",
            Self::ProxyHeadersTooLong => "proxy response headers too long",
            Self::TunnelUnexpectedEof => "unexpected end of file",
            Self::TunnelUnsuccessful => "unsuccessful",
            Self::MalformedResponse => "malformed proxy response",
            Self::ConnectFailed(_) => "failed to create underlying connection",
            Self::Io(_) => "io error establishing tunnel",
        })
    }
}

impl std::error::Error for TunnelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::ConnectFailed(e) => Some(&**e),
            _ => None,
        }
    }
}

/// A helper that exposes the extra headers of a [`Headers`] value.
trait HeadersExt {
    /// The extra headers, if the value carries any.
    fn as_extra(&self) -> Option<&HeaderMap>;
}

impl HeadersExt for Headers {
    fn as_extra(&self) -> Option<&HeaderMap> {
        match self {
            Self::Extra(extra) => Some(extra),
            _ => None,
        }
    }
}
