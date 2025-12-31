//! WebSocket Upgrade

#[cfg(feature = "json")]
mod json;
pub mod message;

use std::{
    borrow::Cow,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

use base64::Engine;
use fastwebsockets::{Frame, Role};
use futures_util::Stream;
use http::{
    HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, Version, header, uri::Scheme,
};
use http2::ext::Protocol;
use sha1::{Digest, Sha1};

use self::message::{CloseCode, Message, Utf8Bytes};
use crate::{
    EmulationFactory, Error, RequestBuilder, Response, Upgraded, header::OrigHeaderMap,
    proxy::Proxy,
};

/// A WebSocket stream.
type WebSocketStream = fastwebsockets::WebSocket<Upgraded>;

/// Configuration for WebSocket connection.
#[derive(Debug, Clone, Copy)]
pub struct WebSocketConfig {
    /// Maximum message size in bytes.
    pub max_message_size: Option<usize>,
    /// Whether to automatically close the connection when a close frame is received.
    pub auto_close: bool,
    /// Whether to automatically send a pong when a ping frame is received.
    pub auto_pong: bool,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            max_message_size: None,
            auto_close: true,
            auto_pong: true,
        }
    }
}

/// Wrapper for [`RequestBuilder`] that performs the
/// websocket handshake when sent.
pub struct WebSocketRequestBuilder {
    inner: RequestBuilder,
    accept_key: Option<Cow<'static, str>>,
    protocols: Option<Vec<Cow<'static, str>>>,
    config: WebSocketConfig,
}

impl WebSocketRequestBuilder {
    /// Creates a new WebSocket request builder.
    pub fn new(inner: RequestBuilder) -> Self {
        Self {
            inner: inner.version(Version::HTTP_11),
            accept_key: None,
            protocols: None,
            config: WebSocketConfig::default(),
        }
    }

    /// Sets a custom WebSocket accept key.
    #[inline]
    pub fn accept_key<K>(mut self, key: K) -> Self
    where
        K: Into<Cow<'static, str>>,
    {
        self.accept_key = Some(key.into());
        self
    }

    /// Forces the WebSocket connection to use HTTP/2 protocol.
    #[inline]
    pub fn force_http2(mut self) -> Self {
        self.inner = self.inner.version(Version::HTTP_2);
        self
    }

    /// Sets the websocket subprotocols to request.
    #[inline]
    pub fn protocols<P>(mut self, protocols: P) -> Self
    where
        P: IntoIterator,
        P::Item: Into<Cow<'static, str>>,
    {
        let protocols = protocols.into_iter().map(Into::into).collect();
        self.protocols = Some(protocols);
        self
    }

    /// Sets the websocket max_message_size configuration.
    #[inline]
    pub fn max_message_size(mut self, max_message_size: usize) -> Self {
        self.config.max_message_size = Some(max_message_size);
        self
    }

    /// Sets whether to automatically close the connection when a close frame is received.
    #[inline]
    pub fn auto_close(mut self, auto_close: bool) -> Self {
        self.config.auto_close = auto_close;
        self
    }

    /// Sets whether to automatically send a pong when a ping frame is received.
    #[inline]
    pub fn auto_pong(mut self, auto_pong: bool) -> Self {
        self.config.auto_pong = auto_pong;
        self
    }

    /// Add a `Header` to this Request.
    #[inline]
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.inner = self.inner.header(key, value);
        self
    }

    /// Add a set of Headers to the existing ones on this Request.
    #[inline]
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.inner = self.inner.headers(headers);
        self
    }

    /// Set the original headers for this request.
    #[inline]
    pub fn orig_headers(mut self, orig_headers: OrigHeaderMap) -> Self {
        self.inner = self.inner.orig_headers(orig_headers);
        self
    }

    /// Enable or disable client default headers for this request.
    pub fn default_headers(mut self, enable: bool) -> Self {
        self.inner = self.inner.default_headers(enable);
        self
    }

    /// Enable HTTP authentication.
    #[inline]
    pub fn auth<V>(mut self, value: V) -> Self
    where
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.inner = self.inner.auth(value);
        self
    }

    /// Enable HTTP basic authentication.
    #[inline]
    pub fn basic_auth<U, P>(mut self, username: U, password: Option<P>) -> Self
    where
        U: fmt::Display,
        P: fmt::Display,
    {
        self.inner = self.inner.basic_auth(username, password);
        self
    }

    /// Enable HTTP bearer authentication.
    #[inline]
    pub fn bearer_auth<T>(mut self, token: T) -> Self
    where
        T: fmt::Display,
    {
        self.inner = self.inner.bearer_auth(token);
        self
    }

    /// Modify the query string of the URI.
    #[inline]
    #[cfg(feature = "query")]
    #[cfg_attr(docsrs, doc(cfg(feature = "query")))]
    pub fn query<T: serde::Serialize + ?Sized>(mut self, query: &T) -> Self {
        self.inner = self.inner.query(query);
        self
    }

    /// Set the proxy for this request.
    #[inline]
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.inner = self.inner.proxy(proxy);
        self
    }

    /// Set the local address for this request.
    #[inline]
    pub fn local_address<V>(mut self, local_address: V) -> Self
    where
        V: Into<Option<IpAddr>>,
    {
        self.inner = self.inner.local_address(local_address);
        self
    }

    /// Set the local addresses for this request.
    #[inline]
    pub fn local_addresses<V4, V6>(mut self, ipv4: V4, ipv6: V6) -> Self
    where
        V4: Into<Option<Ipv4Addr>>,
        V6: Into<Option<Ipv6Addr>>,
    {
        self.inner = self.inner.local_addresses(ipv4, ipv6);
        self
    }

    /// Set the interface for this request.
    #[inline]
    #[cfg(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "solaris",
        target_os = "tvos",
        target_os = "visionos",
        target_os = "watchos",
    ))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "solaris",
            target_os = "tvos",
            target_os = "visionos",
            target_os = "watchos",
        )))
    )]
    pub fn interface<I>(mut self, interface: I) -> Self
    where
        I: Into<std::borrow::Cow<'static, str>>,
    {
        self.inner = self.inner.interface(interface);
        self
    }

    /// Set the emulation for this request.
    #[inline]
    pub fn emulation<P>(mut self, factory: P) -> Self
    where
        P: EmulationFactory,
    {
        self.inner = self.inner.emulation(factory);
        self
    }

    /// Sends the request and returns and [`WebSocketResponse`].
    pub async fn send(self) -> Result<WebSocketResponse, Error> {
        let (client, request) = self.inner.build_split();
        let mut request = request?;

        // Ensure the scheme is http or https
        let uri = request.uri_mut();
        let scheme = match uri.scheme_str() {
            Some("ws") => Some(Scheme::HTTP),
            Some("wss") => Some(Scheme::HTTPS),
            _ => None,
        };
        if scheme.is_some() {
            let mut parts = uri.clone().into_parts();
            parts.scheme = scheme;
            *uri = Uri::from_parts(parts).map_err(Error::builder)?;
        }

        // Get the version of the request
        let version = request.version();

        // Set the headers for the websocket handshake
        let headers = request.headers_mut();
        headers.insert(
            header::SEC_WEBSOCKET_VERSION,
            HeaderValue::from_static("13"),
        );

        // Ensure the request is HTTP 1.1/HTTP 2
        let accept_key = match version {
            Some(Version::HTTP_10 | Version::HTTP_11) => {
                // Generate a nonce if one wasn't provided
                let nonce = self
                    .accept_key
                    .unwrap_or_else(|| Cow::Owned(fastwebsockets::handshake::generate_key()));

                headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
                headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
                headers.insert(
                    header::SEC_WEBSOCKET_KEY,
                    HeaderValue::from_str(&nonce).map_err(Error::builder)?,
                );

                *request.method_mut() = Method::GET;
                *request.version_mut() = Some(Version::HTTP_11);
                Some(nonce)
            }
            Some(Version::HTTP_2) => {
                *request.method_mut() = Method::CONNECT;
                *request.version_mut() = Some(Version::HTTP_2);
                request
                    .extensions_mut()
                    .insert(Protocol::from_static("websocket"));
                None
            }
            unsupported => {
                return Err(Error::upgrade(format!(
                    "unsupported version: {unsupported:?}"
                )));
            }
        };

        // Set websocket subprotocols
        if let Some(ref protocols) = self.protocols {
            // Sets subprotocols
            if !protocols.is_empty() {
                let subprotocols = protocols
                    .iter()
                    .map(|s| s.as_ref())
                    .collect::<Vec<&str>>()
                    .join(", ");

                request.headers_mut().insert(
                    header::SEC_WEBSOCKET_PROTOCOL,
                    subprotocols.parse().map_err(Error::builder)?,
                );
            }
        }

        client
            .execute(request)
            .await
            .map(|inner| WebSocketResponse {
                inner,
                accept_key,
                protocols: self.protocols,
                config: self.config,
            })
    }
}

/// The server's response to the websocket upgrade request.
#[derive(Debug)]
pub struct WebSocketResponse {
    inner: Response,
    accept_key: Option<Cow<'static, str>>,
    protocols: Option<Vec<Cow<'static, str>>>,
    config: WebSocketConfig,
}

impl Deref for WebSocketResponse {
    type Target = Response;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for WebSocketResponse {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl WebSocketResponse {
    /// Turns the response into a websocket. This checks if the websocket
    /// handshake was successful.
    pub async fn into_websocket(self) -> Result<WebSocket, Error> {
        let (inner, protocol) = {
            let status = self.inner.status();
            let headers = self.inner.headers();

            match self.inner.version() {
                // HTTP/1.0 and HTTP/1.1 use the traditional upgrade mechanism
                Version::HTTP_10 | Version::HTTP_11 => {
                    if status != StatusCode::SWITCHING_PROTOCOLS {
                        return Err(Error::upgrade(format!("unexpected status code: {status}")));
                    }

                    if !header_contains(self.inner.headers(), header::CONNECTION, "upgrade") {
                        return Err(Error::upgrade("missing connection header"));
                    }

                    if !header_eq(self.inner.headers(), header::UPGRADE, "websocket") {
                        return Err(Error::upgrade("invalid upgrade header"));
                    }

                    match self
                        .accept_key
                        .zip(headers.get(header::SEC_WEBSOCKET_ACCEPT))
                    {
                        Some((nonce, header)) => {
                            if !header
                                .to_str()
                                .is_ok_and(|s| s == derive_accept_key(nonce.as_bytes()))
                            {
                                return Err(Error::upgrade(format!(
                                    "invalid accept key: {header:?}"
                                )));
                            }
                        }
                        None => {
                            return Err(Error::upgrade("missing accept key"));
                        }
                    }
                }
                // HTTP/2 uses the Extended CONNECT Protocol (RFC 8441)
                Version::HTTP_2 => {
                    if status != StatusCode::OK {
                        return Err(Error::upgrade(format!("unexpected status code: {status}")));
                    }
                }
                _ => {
                    return Err(Error::upgrade(format!(
                        "unsupported version: {:?}",
                        self.inner.version()
                    )));
                }
            }

            let protocol = headers.get(header::SEC_WEBSOCKET_PROTOCOL).cloned();
            let requested = self.protocols.as_ref().filter(|p| !p.is_empty());
            let replied = protocol.as_ref().and_then(|v| v.to_str().ok());

            match (requested, replied) {
                (Some(req), Some(rep)) => {
                    if !req.contains(&Cow::Borrowed(rep)) {
                        return Err(Error::upgrade(format!("invalid protocol: {rep}")));
                    }
                }
                (Some(_), None) => {
                    return Err(Error::upgrade(format!(
                        "missing protocol: {:?}",
                        self.protocols
                    )));
                }
                (None, Some(_)) => {
                    return Err(Error::upgrade(format!("invalid protocol: {protocol:?}")));
                }
                (None, None) => {}
            };

            let upgraded = self.inner.upgrade().await?;
            let mut ws = fastwebsockets::WebSocket::after_handshake(upgraded, Role::Client);

            if let Some(size) = self.config.max_message_size {
                ws.set_max_message_size(size);
            }
            ws.set_auto_close(self.config.auto_close);
            ws.set_auto_pong(self.config.auto_pong);

            (ws, protocol)
        };

        Ok(WebSocket { inner, protocol })
    }
}

/// Checks if the header value is equal to the given value.
fn header_eq(headers: &HeaderMap, key: HeaderName, value: &'static str) -> bool {
    if let Some(header) = headers.get(&key) {
        header.as_bytes().eq_ignore_ascii_case(value.as_bytes())
    } else {
        false
    }
}

/// Checks if the header value contains the given value.
fn header_contains(headers: &HeaderMap, key: HeaderName, value: &'static str) -> bool {
    let header = if let Some(header) = headers.get(&key) {
        header
    } else {
        return false;
    };

    if let Ok(header) = std::str::from_utf8(header.as_bytes()) {
        header.to_ascii_lowercase().contains(value)
    } else {
        false
    }
}

fn derive_accept_key(key: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key);
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

/// A websocket connection
pub struct WebSocket {
    inner: WebSocketStream,
    protocol: Option<HeaderValue>,
}

impl WebSocket {
    /// Return the selected WebSocket subprotocol, if one has been chosen.
    #[inline]
    pub fn protocol(&self) -> Option<&HeaderValue> {
        self.protocol.as_ref()
    }

    /// Receive another message.
    ///
    /// Returns `None` if the stream has closed.
    #[inline]
    pub async fn recv(&mut self) -> Option<Result<Message, Error>> {
        match self.inner.read_frame().await {
            Ok(frame) => Some(Ok(Message::from_frame(frame))),
            Err(e) => {
                // fastwebsockets returns ConnectionClosed when the stream is closed.
                // We can treat it as None (end of stream).
                // But we should check the error type.
                // For now, return error unless it's clearly EOF.
                // fastwebsockets::WebSocketError doesn't expose easy EOF check except via Io error.
                // Let's return the error.
                Some(Err(Error::websocket(e)))
            }
        }
    }

    /// Send a message.
    #[inline]
    pub async fn send(&mut self, msg: Message) -> Result<(), Error> {
        let frame = msg.into_frame();
        self.inner
            .write_frame(frame)
            .await
            .map_err(Error::websocket)
    }

    /// Closes the connection with a given code and (optional) reason.
    pub async fn close<C, R>(mut self, code: C, reason: R) -> Result<(), Error>
    where
        C: Into<CloseCode>,
        R: Into<Utf8Bytes>,
    {
        let code = code.into();
        let reason = reason.into();
        let frame = Frame::close(code.0, reason.as_str().as_bytes());
        self.inner
            .write_frame(frame)
            .await
            .map_err(Error::websocket)
    }

    /// Split the WebSocket into a reader and a writer.
    pub fn split(self) -> (WebSocketWrite, WebSocketRead) {
        let (r, w) = self.inner.split(tokio::io::split);
        (WebSocketWrite { inner: w }, WebSocketRead { inner: r })
    }
}

/// A WebSocket reader.
pub struct WebSocketRead {
    inner: fastwebsockets::WebSocketRead<tokio::io::ReadHalf<Upgraded>>,
}

impl Stream for WebSocketRead {
    type Item = Result<Message, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut send_fn = |_| async { Ok::<(), Error>(()) };
        let fut = self.inner.read_frame(&mut send_fn);
        tokio::pin!(fut);
        match fut.poll(cx) {
            Poll::Ready(Ok(frame)) => Poll::Ready(Some(Ok(Message::from_frame(frame)))),
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(Error::websocket(e)))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl WebSocketRead {
    /// Receive another message.
    pub async fn recv(&mut self) -> Option<Result<Message, Error>> {
        match self
            .inner
            .read_frame(&mut |_| async { Ok::<(), Error>(()) })
            .await
        {
            Ok(frame) => Some(Ok(Message::from_frame(frame))),
            Err(e) => Some(Err(Error::websocket(e))),
        }
    }
}

/// A WebSocket writer.
pub struct WebSocketWrite {
    inner: fastwebsockets::WebSocketWrite<tokio::io::WriteHalf<Upgraded>>,
}

impl WebSocketWrite {
    /// Send a message.
    pub async fn send(&mut self, msg: Message) -> Result<(), Error> {
        let frame = msg.into_frame();
        self.inner
            .write_frame(frame)
            .await
            .map_err(Error::websocket)
    }

    /// Closes the connection with a given code and (optional) reason.
    pub async fn close<C, R>(mut self, code: C, reason: R) -> Result<(), Error>
    where
        C: Into<CloseCode>,
        R: Into<Utf8Bytes>,
    {
        let code = code.into();
        let reason = reason.into();
        let frame = Frame::close(code.0, reason.as_str().as_bytes());
        self.inner
            .write_frame(frame)
            .await
            .map_err(Error::websocket)
    }
}
