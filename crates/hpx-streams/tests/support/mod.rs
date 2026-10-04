//! Loopback HTTP/1 server used by the `hpx-streams` integration tests.
//!
//! A real socket is used rather than an in-memory body so the tests can control
//! *where* the response body is split across TCP segments. Every codec in this
//! crate is a `tokio_util::codec::Decoder` driven by chunk arrival, so the
//! interesting bugs live in the state carried between `decode` calls — bracket
//! depth, quote/escape state, a half-read varint length prefix. Serving the
//! body in caller-specified slices is what makes those paths reachable.

#![allow(missing_docs, dead_code)]

use std::{net::SocketAddr, thread, time::Duration};

use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    runtime,
};

/// How long a server thread waits for its single connection before giving up.
///
/// Without this bound, a test whose client never connects (for example because
/// client construction failed) would leave the thread blocked forever.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to keep a served connection open while the client drains it.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait for the client's request head before giving up on it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Read bytes until the end of the HTTP request head (`\r\n\r\n`).
async fn read_request_head(socket: &mut tokio::net::TcpStream) {
    let mut seen: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 512];
    loop {
        let Ok(read) = socket.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        seen.extend_from_slice(&chunk[..read]);
        if seen.windows(4).any(|w| w == b"\r\n\r\n") {
            return;
        }
        // Guard against a peer that never terminates its head.
        if seen.len() > 64 * 1024 {
            return;
        }
    }
}

/// A loopback server that serves exactly one response.
pub(crate) struct ChunkedServer {
    addr: SocketAddr,
}

impl ChunkedServer {
    /// Serve `body` as a `200 OK` response, split into the given chunk sizes.
    ///
    /// Chunk sizes are consumed cyclically until the body is exhausted, so a
    /// short slice list can drive a long body. Pass `&[usize::MAX]` to deliver
    /// the whole body in one segment. A zero or oversized size means "the rest
    /// of the body".
    ///
    /// `content_type` is emitted verbatim; pass an empty string to omit it.
    pub(crate) fn serve_chunks(body: &[u8], chunk_sizes: &[usize], content_type: &str) -> Self {
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\r\n",
            body.len()
        );
        let mut response = head.into_bytes();

        let mut remaining = body;
        let mut index = 0usize;
        while !remaining.is_empty() {
            let requested = chunk_sizes
                .get(index % chunk_sizes.len().max(1))
                .copied()
                .unwrap_or(usize::MAX);
            let take = requested.max(1).min(remaining.len());
            response.extend_from_slice(&remaining[..take]);
            remaining = &remaining[take..];
            index += 1;
        }

        Self::serve_raw(response)
    }

    /// Serve `body` as a single-segment `200 OK` response.
    pub(crate) fn serve(body: &[u8], content_type: &str) -> Self {
        Self::serve_chunks(body, &[usize::MAX], content_type)
    }

    /// Serve a fully assembled raw HTTP response, status line and headers
    /// included.
    ///
    /// This is how the malformed-response tests inject bad framing: a bad
    /// `Content-Length`, an early close, invalid header syntax, and so on.
    pub(crate) fn serve_raw(response: Vec<u8>) -> Self {
        // The listener is created with `std` and only adopted by the tokio
        // runtime inside the server thread. Binding through `block_on` would
        // panic when the caller is already inside a tokio runtime, which every
        // `#[tokio::test]` is.
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("loopback bind must succeed");
        let addr = listener
            .local_addr()
            .expect("bound listener must have an address");
        listener
            .set_nonblocking(true)
            .expect("a listener must be switchable to non-blocking mode");

        // The server owns its own single-threaded runtime on a dedicated
        // thread so it never contends with the test's reactor.
        thread::spawn(move || {
            let rt = runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime");
            rt.block_on(async move {
                let listener = TcpListener::from_std(listener).expect("adopt std listener");
                let accepted = tokio::time::timeout(ACCEPT_TIMEOUT, listener.accept()).await;
                let Ok(Ok((mut socket, _))) = accepted else {
                    // No client connected within the bound. Nothing to serve.
                    return;
                };
                // The request head must be consumed before the response is
                // written. An HTTP/1 client reads eagerly once the connection
                // is handed to the pool, so a server that speaks first leaves
                // unsolicited bytes on an idle connection and the client
                // rejects the connection with `UnexpectedMessage` before it has
                // even sent anything.
                let _ = tokio::time::timeout(REQUEST_TIMEOUT, read_request_head(&mut socket)).await;
                let _ = socket.write_all(&response).await;
                let _ = socket.flush().await;
                // Hold the connection open until the client hangs up. Closing
                // straight after the write races the client's read: `hpx` sees
                // the peer disappear mid-response and cancels the request,
                // which looks like a client bug rather than a server one.
                let _ = tokio::time::timeout(DRAIN_TIMEOUT, async {
                    let mut sink = [0u8; 256];
                    loop {
                        match socket.read(&mut sink).await {
                            Ok(0) | Err(_) => break,
                            Ok(_) => continue,
                        }
                    }
                })
                .await;
            });
        });

        Self { addr }
    }

    /// Base URL for this server, e.g. `http://127.0.0.1:34567/stream`.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

/// Build a client that bypasses any system proxy configuration.
///
/// # Panics
///
/// Panics if the client cannot be constructed, which indicates a broken test
/// environment rather than a defect in the code under test.
pub(crate) fn client() -> hpx::Client {
    hpx::Client::builder()
        .no_proxy()
        .build()
        .expect("an hpx client must build in the test environment")
}

/// GET `url`, panicking with context if the request fails.
///
/// # Panics
///
/// Panics when the request fails.
pub(crate) async fn get_ok(url: &str) -> hpx::Response {
    init_tracing();
    match client().get(url).send().await {
        Ok(res) => res,
        Err(err) => panic!("expected a response from {url}, got error: {err:?}"),
    }
}

/// Install a `tracing` subscriber driven by `RUST_LOG`, if one is not installed.
///
/// Opt-in diagnostics for the codec paths: run with
/// `RUST_LOG=hpx=trace cargo test -p hpx-streams` to see the framing decisions
/// the client makes while splitting a response body. `try_init` makes repeat
/// calls cheap no-ops, so this is safe to call from every request helper.
pub(crate) fn init_tracing() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init();
    });
}

/// Build a length-prefixed varint, the framing used by
/// [`hpx_streams::ProtobufStreamResponse`].
pub(crate) fn encode_varint_len(mut value: usize, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(u8::try_from(value & 0x7F).unwrap_or(0x7F) | 0x80);
        value >>= 7;
    }
    out.push(u8::try_from(value).unwrap_or(0));
}

/// Serialize `messages` as a length-prefixed protobuf stream.
pub(crate) fn encode_len_prefixed<T: prost::Message>(messages: &[T]) -> Vec<u8> {
    let mut out = Vec::new();
    for message in messages {
        let mut body = Vec::new();
        message
            .encode(&mut body)
            .expect("prost encode into a Vec cannot fail");
        encode_varint_len(body.len(), &mut out);
        out.extend_from_slice(&body);
    }
    out
}

/// Drain a stream of stream-items, panicking on the first error.
///
/// # Panics
///
/// Panics when any item is an error.
pub(crate) async fn collect_ok<S, T>(stream: S) -> Vec<T>
where
    S: futures::Stream<Item = hpx_streams::StreamBodyResult<T>> + Unpin,
    T: std::fmt::Debug,
{
    use futures::TryStreamExt as _;
    stream
        .try_collect()
        .await
        .unwrap_or_else(|err| panic!("stream yielded an unexpected error: {err}"))
}

/// Split a stream into its successful items and its error kinds.
///
/// Lets a test assert that a specific item failed without discarding the items
/// that decoded cleanly before it.
pub(crate) async fn partition_stream<S, T>(
    stream: S,
) -> (Vec<T>, Vec<hpx_streams::error::StreamBodyKind>)
where
    S: futures::Stream<Item = hpx_streams::StreamBodyResult<T>> + Unpin,
{
    use futures::StreamExt as _;
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    futures::pin_mut!(stream);
    while let Some(item) = stream.next().await {
        match item {
            Ok(value) => ok.push(value),
            Err(err) => errs.push(err.kind()),
        }
    }
    (ok, errs)
}
