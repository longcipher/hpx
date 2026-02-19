//! SSE connection driver implementation.
//!
//! Provides [`SseConnection`], [`SseHandle`], and [`SseStream`] for managing
//! SSE connections with auto-reconnection, protocol handler classification, and
//! optional authentication.

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use tokio::{sync::mpsc, time::timeout};
use tracing::{debug, error, info, warn};

use super::{config::SseConfig, protocol::SseProtocolHandler, types::SseEvent};
use crate::{
    auth::Authentication,
    error::{TransportError, TransportResult},
    reconnect::{BackoffConfig, calculate_backoff},
};

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

/// SSE connection state machine states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SseConnectionState {
    /// Not connected.
    Disconnected,
    /// Attempting to establish a connection.
    Connecting,
    /// Actively receiving events.
    Connected,
    /// Reconnecting after a disconnect.
    Reconnecting {
        /// Current reconnection attempt number.
        attempt: u32,
    },
    /// Fully closed — will not reconnect.
    Closed,
}

impl SseConnectionState {
    /// Returns `true` if the connection is actively streaming.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Returns `true` if the connection is in a terminal state.
    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Closed)
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Control commands sent from [`SseHandle`] to the background task.
#[derive(Debug)]
pub enum SseCommand {
    /// Gracefully close the connection.
    Close,
    /// Request a reconnection with an explanatory reason.
    Reconnect {
        /// Human-readable reason for the reconnection request.
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// Public API: SseConnection
// ---------------------------------------------------------------------------

/// Entry point for SSE connections.
///
/// Call [`connect()`](SseConnection::connect) to establish a connection and
/// then [`split()`](SseConnection::split) to obtain a [`SseHandle`]
/// (for control) and [`SseStream`] (for events).
pub struct SseConnection {
    handle: SseHandle,
    stream: SseStream,
}

impl SseConnection {
    /// Establish an SSE connection with the given configuration and handler.
    ///
    /// Spawns a background task that manages the HTTP connection, parses events
    /// via the inlined SSE parser, and forwards them to the [`SseStream`].
    ///
    /// # Errors
    ///
    /// Returns an error if configuration validation fails.
    pub async fn connect<H: SseProtocolHandler>(
        config: SseConfig,
        handler: H,
    ) -> TransportResult<Self> {
        Self::connect_inner(config, handler, None).await
    }

    /// Establish an SSE connection with authentication.
    ///
    /// Same as [`connect()`](SseConnection::connect), but signs every HTTP
    /// request (including reconnections) using the given [`Authentication`]
    /// implementation.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration validation fails.
    pub async fn connect_with_auth<H: SseProtocolHandler>(
        config: SseConfig,
        handler: H,
        auth: Box<dyn Authentication>,
    ) -> TransportResult<Self> {
        Self::connect_inner(config, handler, Some(auth)).await
    }

    /// Internal constructor shared between `connect` and `connect_with_auth`.
    async fn connect_inner<H: SseProtocolHandler>(
        config: SseConfig,
        handler: H,
        auth: Option<Box<dyn Authentication>>,
    ) -> TransportResult<Self> {
        config.validate().map_err(TransportError::config)?;

        let config = Arc::new(config);
        let (cmd_tx, cmd_rx) = mpsc::channel(config.command_channel_capacity);
        let (event_tx, event_rx) = mpsc::channel(config.event_channel_capacity);

        tokio::spawn(sse_connection_driver(
            Arc::clone(&config),
            handler,
            auth,
            cmd_rx,
            event_tx,
        ));

        let handle = SseHandle { cmd_tx };
        let stream = SseStream { rx: event_rx };

        Ok(Self { handle, stream })
    }

    /// Split the connection into a control handle and event stream.
    pub fn split(self) -> (SseHandle, SseStream) {
        (self.handle, self.stream)
    }

    /// Get a reference to the control handle.
    pub fn handle(&self) -> &SseHandle {
        &self.handle
    }
}

impl Stream for SseConnection {
    type Item = SseEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.stream).poll_next(cx)
    }
}

// ---------------------------------------------------------------------------
// SseHandle
// ---------------------------------------------------------------------------

/// Clone-able handle for controlling a running SSE connection.
#[derive(Clone)]
pub struct SseHandle {
    cmd_tx: mpsc::Sender<SseCommand>,
}

impl SseHandle {
    /// Request a graceful close.
    ///
    /// # Errors
    ///
    /// Returns an error if the background task has already shut down.
    pub async fn close(&self) -> TransportResult<()> {
        self.cmd_tx.send(SseCommand::Close).await.map_err(|_| {
            TransportError::connection_closed(Some("SSE background task shut down".to_string()))
        })
    }

    /// Request a reconnection.
    ///
    /// # Errors
    ///
    /// Returns an error if the background task has already shut down.
    pub async fn reconnect(&self, reason: &str) -> TransportResult<()> {
        self.cmd_tx
            .send(SseCommand::Reconnect {
                reason: reason.to_string(),
            })
            .await
            .map_err(|_| {
                TransportError::connection_closed(Some("SSE background task shut down".to_string()))
            })
    }

    /// Check whether the background task is still running.
    pub fn is_connected(&self) -> bool {
        !self.cmd_tx.is_closed()
    }
}

// ---------------------------------------------------------------------------
// SseStream
// ---------------------------------------------------------------------------

/// Stream of [`SseEvent`]s from an SSE connection.
///
/// Implements [`Stream`] for use with `StreamExt` combinators.
pub struct SseStream {
    rx: mpsc::Receiver<SseEvent>,
}

impl SseStream {
    /// Receive the next SSE event, blocking until one is available.
    pub async fn next_event(&mut self) -> Option<SseEvent> {
        self.rx.recv().await
    }
}

impl Stream for SseStream {
    type Item = SseEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.rx).poll_recv(cx)
    }
}

// ---------------------------------------------------------------------------
// Internal: establish a single HTTP connection and return an EventStream
// ---------------------------------------------------------------------------

/// Establish a single SSE connection via `hpx::Client`.
///
/// Validates response status and Content-Type, then wraps the response body
/// in an `EventStream`.
async fn establish_sse_connection(
    config: &SseConfig,
    auth: Option<&dyn Authentication>,
    last_event_id: Option<&str>,
) -> TransportResult<super::parse::EventStream<impl Stream<Item = Result<Bytes, hpx::Error>> + use<>>>
{
    let mut request_url = config.url.clone();
    let client = hpx::Client::builder()
        .connect_timeout(config.connect_timeout)
        .build()
        .map_err(|e| TransportError::config(format!("Failed to build HTTP client: {e}")))?;

    let mut headers = config.headers.clone();
    headers.insert(
        http::header::ACCEPT,
        http::HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-cache"),
    );

    // Last-Event-ID for resumption.
    if let Some(id) = last_event_id
        && let Ok(value) = http::HeaderValue::from_str(id)
    {
        headers.insert(
            http::header::HeaderName::from_static("last-event-id"),
            value,
        );
    }

    // Apply authentication if configured.
    if config.auth_on_connect
        && let Some(a) = auth
    {
        let path = url_path(&config.url);
        let body = config.body.as_deref();
        if let Some(qs) = a.sign(&config.method, &path, &mut headers, body).await? {
            append_query_string(&mut request_url, &qs);
            debug!(query_string = %qs, "Applied auth query string to SSE URL");
        }
    }

    // Build request.
    let mut req = match config.method {
        http::Method::POST => client.post(&request_url),
        _ => client.get(&request_url),
    };
    req = req.headers(headers);

    if let Some(body) = &config.body {
        req = req.body(body.clone());
    }

    let resp = timeout(config.connect_timeout, req.send())
        .await
        .map_err(|_| TransportError::timeout(config.connect_timeout))?
        .map_err(TransportError::Http)?;

    // Validate status.
    let status = resp.status();
    if !status.is_success() {
        return Err(TransportError::sse_invalid_status(status));
    }

    // Validate Content-Type.
    if let Some(ct) = resp.headers().get(http::header::CONTENT_TYPE) {
        let ct_str = ct.to_str().unwrap_or("");
        if !ct_str.contains("text/event-stream") {
            return Err(TransportError::sse_invalid_content_type(ct_str));
        }
    }

    // Wrap the body stream in the inlined EventStream parser.
    let body_stream = resp.bytes_stream();
    Ok(super::parse::EventStream::new(body_stream))
}

/// Extract the path component from a URL string.
fn url_path(url: &str) -> String {
    url.find("://")
        .and_then(|scheme_end| url[scheme_end + 3..].find('/'))
        .map(|path_start| {
            let offset = url.find("://").unwrap_or(0) + 3;
            url[offset + path_start..].to_string()
        })
        .unwrap_or_else(|| "/".to_string())
}

fn append_query_string(url: &mut String, query: &str) {
    let query = query.trim_start_matches('?');
    if query.is_empty() {
        return;
    }

    if url.contains('?') {
        url.push('&');
    } else {
        url.push('?');
    }
    url.push_str(query);
}

// ---------------------------------------------------------------------------
// Internal: backoff calculation
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Internal: background driver
// ---------------------------------------------------------------------------

/// The long-lived background task that drives the SSE connection.
///
/// It connects, reads events, classifies them, forwards them to the consumer,
/// and reconnects with exponential backoff on failures.
async fn sse_connection_driver<H: SseProtocolHandler>(
    config: Arc<SseConfig>,
    handler: H,
    auth: Option<Box<dyn Authentication>>,
    mut cmd_rx: mpsc::Receiver<SseCommand>,
    event_tx: mpsc::Sender<SseEvent>,
) {
    let mut attempt: u32 = 0;
    let mut last_event_id: Option<String> = None;

    loop {
        // --- Establish connection ---
        info!(url = %config.url, attempt, "SSE connecting");
        let connection =
            establish_sse_connection(&config, auth.as_deref(), last_event_id.as_deref()).await;

        let mut event_stream = match connection {
            Ok(stream) => {
                info!(url = %config.url, "SSE connection established");
                handler.on_connect();
                attempt = 0;
                stream
            }
            Err(err) => {
                error!(url = %config.url, error = %err, "SSE connection failed");
                handler.on_disconnect();

                if !handler.should_retry(&err) {
                    warn!("Handler says no retry — closing");
                    return;
                }
                if let Some(max) = config.reconnect_max_attempts
                    && attempt >= max
                {
                    error!(attempts = max, "Max SSE reconnect attempts exceeded");
                    return;
                }

                let delay = calculate_backoff(
                    BackoffConfig {
                        initial_delay: config.reconnect_initial_delay,
                        max_delay: config.reconnect_max_delay,
                        factor: config.reconnect_backoff_factor,
                        jitter: config.reconnect_jitter,
                    },
                    attempt,
                );
                attempt = attempt.saturating_add(1);
                warn!(
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    "SSE reconnecting after backoff"
                );
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        // --- Event loop ---
        let should_reconnect = loop {
            tokio::select! {
                biased;

                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(SseCommand::Close) | None => {
                            info!("SSE connection closing (requested)");
                            handler.on_disconnect();
                            return;
                        }
                        Some(SseCommand::Reconnect { reason }) => {
                            warn!(reason = %reason, "SSE reconnect requested");
                            handler.on_disconnect();
                            break true;
                        }
                    }
                }

                item = event_stream.next() => {
                    match item {
                        Some(Ok(raw_event)) => {
                            // Track last event ID for reconnection.
                            if !raw_event.id.is_empty() {
                                last_event_id = Some(raw_event.id.to_string());
                            }

                            let kind = handler.classify_event(&raw_event);
                            debug!(
                                event_type = %raw_event.event,
                                id = %raw_event.id,
                                kind = %kind,
                                "SSE event received",
                            );

                            let sse_event = SseEvent::new(raw_event, kind);
                            if event_tx.send(sse_event).await.is_err() {
                                // Consumer dropped — shut down.
                                info!("SSE consumer dropped, shutting down");
                                handler.on_disconnect();
                                return;
                            }
                        }
                        Some(Err(err)) => {
                            error!(error = %err, "SSE stream error");
                            handler.on_disconnect();

                            let transport_err = TransportError::sse_parse(err.to_string());
                            if !handler.should_retry(&transport_err) {
                                warn!("Handler says no retry after stream error — closing");
                                return;
                            }
                            break true;
                        }
                        None => {
                            // Stream ended (server closed connection).
                            warn!("SSE stream ended");
                            handler.on_disconnect();

                            let transport_err = TransportError::sse_stream_ended();
                            if !handler.should_retry(&transport_err) {
                                return;
                            }
                            break true;
                        }
                    }
                }
            }
        };

        if !should_reconnect {
            return;
        }

        // --- Reconnect with backoff ---
        if let Some(max) = config.reconnect_max_attempts
            && attempt >= max
        {
            error!(attempts = max, "Max SSE reconnect attempts exceeded");
            return;
        }

        let delay = calculate_backoff(
            BackoffConfig {
                initial_delay: config.reconnect_initial_delay,
                max_delay: config.reconnect_max_delay,
                factor: config.reconnect_backoff_factor,
                jitter: config.reconnect_jitter,
            },
            attempt,
        );
        attempt = attempt.saturating_add(1);
        warn!(
            attempt,
            delay_ms = delay.as_millis() as u64,
            "SSE reconnecting after backoff"
        );
        tokio::time::sleep(delay).await;
    }
}
