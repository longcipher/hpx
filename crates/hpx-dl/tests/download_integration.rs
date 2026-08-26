//! Integration tests for segmented downloads against a local mock server.
//!
//! Covers the range-request correctness invariants that pure unit tests
//! cannot reach: status-code handling (206 vs. ignored `Range`), segment
//! length validation (short/over-long bodies), and progress accounting
//! across mid-stream connection failures and retries.

#![cfg(feature = "http")]

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::stream;
use hpx_dl::{
    DownloadEngine, DownloadError, DownloadRecord, DownloadRequest, SegmentDownloader,
    SegmentRange, Storage,
};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Mock server
// ---------------------------------------------------------------------------

const FILE_LEN: usize = 1000;

/// Deterministic test payload: byte at offset `i` is `(i % 251) as u8`.
fn file_bytes() -> Vec<u8> {
    (0..FILE_LEN).map(|i| (i % 251) as u8).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Honor `Range` with correct 206 responses.
    Ok,
    /// Ignore `Range` entirely; always answer 200 with the full body.
    IgnoreRange,
    /// Claim the full requested slice but send only half of it.
    Short,
    /// Send the requested slice plus extra trailing bytes beyond the end.
    Long,
    /// First request aborts mid-stream after half the bytes; later requests
    /// succeed normally.
    Flaky,
}

#[derive(Clone)]
struct MockState {
    mode: Mode,
    hits: Arc<AtomicU32>,
}

fn parse_range(value: &HeaderValue) -> Option<(usize, usize)> {
    let raw = value.to_str().ok()?;
    let rest = raw.strip_prefix("bytes=")?;
    let (start, end) = rest.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?))
}

fn partial_response(data: &[u8], start: usize, end: usize, body: Vec<u8>) -> Response {
    let mut resp = (StatusCode::PARTIAL_CONTENT, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes {start}-{end}/{}", data.len()))
            .expect("valid content-range"),
    );
    resp
}

fn full_206(data: &[u8], start: usize, end: usize) -> Response {
    partial_response(data, start, end, data[start..=end].to_vec())
}

async fn get_file(State(state): State<MockState>, headers: HeaderMap) -> Response {
    let data = file_bytes();
    let requested = headers.get(header::RANGE).and_then(parse_range);

    match state.mode {
        Mode::Ok => match requested {
            Some((start, end)) => full_206(&data, start, end),
            None => (StatusCode::OK, data).into_response(),
        },
        Mode::IgnoreRange => (StatusCode::OK, data).into_response(),
        Mode::Short => {
            let (start, end) = requested.unwrap_or((0, FILE_LEN - 1));
            let half_len = (end - start + 1) / 2;
            partial_response(&data, start, end, data[start..start + half_len].to_vec())
        }
        Mode::Long => {
            let (start, end) = requested.unwrap_or((0, FILE_LEN - 1));
            let mut body = data[start..=end].to_vec();
            body.extend(std::iter::repeat_n(0xFF_u8, 64));
            partial_response(&data, start, end, body)
        }
        Mode::Flaky => {
            let (start, end) = requested.unwrap_or((0, FILE_LEN - 1));
            if state.hits.fetch_add(1, Ordering::SeqCst) == 0 {
                // Abort mid-stream after roughly half the segment.
                let half = (end - start + 1) / 2;
                let chunk = data[start..start + half].to_vec();
                let err_stream = stream::iter(vec![
                    Ok::<_, std::io::Error>(axum::body::Bytes::from(chunk)),
                    Err(std::io::Error::other("connection dropped mid-stream")),
                ]);
                let mut resp = Body::from_stream(err_stream).into_response();
                *resp.status_mut() = StatusCode::PARTIAL_CONTENT;
                resp.headers_mut().insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes {start}-{end}/{}", data.len()))
                        .expect("valid content-range"),
                );
                resp
            } else {
                full_206(&data, start, end)
            }
        }
    }
}

async fn head_file() -> Response {
    let mut resp = StatusCode::OK.into_response();
    let headers = resp.headers_mut();
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(FILE_LEN as u64));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(header::ETAG, HeaderValue::from_static("\"mock-etag-v1\""));
    resp
}

async fn spawn_server(mode: Mode) -> SocketAddr {
    let state = MockState {
        mode,
        hits: Arc::new(AtomicU32::new(0)),
    };
    let app = Router::new()
        .route("/file.bin", get(get_file).head(head_file))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock server");
    });
    addr
}

fn downloader_for(
    addr: SocketAddr,
    segments: Vec<SegmentRange>,
    dest: std::path::PathBuf,
) -> SegmentDownloader {
    let client = hpx::Client::new();
    SegmentDownloader::new(client, format!("http://{addr}/file.bin"), segments, dest)
}

/// Unwrap one level of retry wrapping to inspect the root cause.
fn root_cause(err: DownloadError) -> DownloadError {
    match err {
        DownloadError::SegmentRetryExhausted { source, .. } => *source,
        other => other,
    }
}

// ---------------------------------------------------------------------------
// In-memory storage stub for engine-level tests
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct TestStorage {
    records: Mutex<std::collections::HashMap<hpx_dl::DownloadId, DownloadRecord>>,
}

#[async_trait]
impl Storage for TestStorage {
    async fn save(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
        let mut records = self.records.lock().await;
        records.insert(download.id, download.clone());
        Ok(())
    }

    async fn load(&self, id: hpx_dl::DownloadId) -> Result<Option<DownloadRecord>, DownloadError> {
        Ok(self.records.lock().await.get(&id).cloned())
    }

    async fn list(&self) -> Result<Vec<DownloadRecord>, DownloadError> {
        Ok(self.records.lock().await.values().cloned().collect())
    }

    async fn delete(&self, id: hpx_dl::DownloadId) -> Result<(), DownloadError> {
        self.records.lock().await.remove(&id);
        Ok(())
    }

    async fn update_progress(
        &self,
        _id: hpx_dl::DownloadId,
        _segments: &[hpx_dl::SegmentState],
    ) -> Result<(), DownloadError> {
        Ok(())
    }

    async fn upsert(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
        self.records
            .lock()
            .await
            .insert(download.id, download.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_segment_download_assembles_exact_content() {
    let addr = spawn_server(Mode::Ok).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dest = tmp.path().join("out.bin");

    let dl = downloader_for(
        addr,
        vec![
            SegmentRange::new(0, 249),
            SegmentRange::new(250, 499),
            SegmentRange::new(500, 749),
            SegmentRange::new(750, 999),
        ],
        dest.clone(),
    );

    let total = dl.download(None).await.expect("download succeeds");
    assert_eq!(total, FILE_LEN as u64);

    let written = tokio::fs::read(&dest).await.expect("read destination");
    assert_eq!(
        written,
        file_bytes(),
        "assembled content must match exactly"
    );
}

#[tokio::test]
async fn server_ignoring_range_fails_nonzero_segment() {
    let addr = spawn_server(Mode::IgnoreRange).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dest = tmp.path().join("out.bin");

    // A nonzero-start segment must refuse a 200 whole-file body instead of
    // writing it at `range.start` (which would corrupt the destination).
    let dl = downloader_for(addr, vec![SegmentRange::new(500, 999)], dest);
    let err = dl
        .download(None)
        .await
        .expect_err("must fail on ignored Range");
    assert!(
        matches!(root_cause(err), DownloadError::NoRangeSupport),
        "expected NoRangeSupport, got error"
    );
}

#[tokio::test]
async fn short_body_reports_length_mismatch() {
    let addr = spawn_server(Mode::Short).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dest = tmp.path().join("out.bin");

    let dl = downloader_for(addr, vec![SegmentRange::new(0, 999)], dest);
    let err = dl.download(None).await.expect_err("short body must fail");
    match root_cause(err) {
        DownloadError::LengthMismatch { expected, actual } => {
            assert_eq!(expected, 1000);
            assert_eq!(actual, 500);
        }
        other => panic!("expected LengthMismatch, got: {other:?}"),
    }
}

#[tokio::test]
async fn overlong_body_reports_length_mismatch() {
    let addr = spawn_server(Mode::Long).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dest = tmp.path().join("out.bin");

    let dl = downloader_for(addr, vec![SegmentRange::new(0, 999)], dest);
    let err = dl
        .download(None)
        .await
        .expect_err("over-long body must fail");
    match root_cause(err) {
        DownloadError::LengthMismatch { expected, actual } => {
            assert_eq!(expected, 1000);
            assert_eq!(actual, 1000, "writes stop at the boundary before failing");
        }
        other => panic!("expected LengthMismatch, got: {other:?}"),
    }
}

#[tokio::test]
async fn progress_is_not_double_counted_across_retries() {
    let addr = spawn_server(Mode::Flaky).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dest = tmp.path().join("out.bin");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<u64>(64);
    let dl = downloader_for(addr, vec![SegmentRange::new(0, 999)], dest);

    let handle = tokio::spawn(async move { dl.download(Some(tx)).await });

    let mut reported_total = 0u64;
    while let Some(delta) = rx.recv().await {
        reported_total += delta;
    }

    let result = handle
        .await
        .expect("task joins")
        .expect("download succeeds");
    assert_eq!(result, 1000);
    // The first attempt reported ~half the segment before dying; the retry
    // re-downloaded everything. Monotonic progress must report each byte once.
    assert_eq!(
        reported_total, 1000,
        "progress deltas must sum to exactly the segment length"
    );
}

#[tokio::test]
async fn engine_fails_when_content_length_missing() {
    // A raw TCP server is required here: hyper-based mocks always synthesize
    // `content-length: 0` on HEAD responses, which the engine legitimately
    // treats as a valid zero-byte file. Only a hand-written response can omit
    // the header entirely to simulate chunked/unknown-size remotes.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock listener");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let is_head = String::from_utf8_lossy(&buf[..n]).starts_with("HEAD");
                let resp = if is_head {
                    // No Content-Length at all — the scenario under test.
                    "HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{}",
                        "x".repeat(FILE_LEN)
                    )
                };
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });

    let tmp = tempfile::tempdir().expect("tempdir");
    let dest = tmp.path().join("out.bin");

    let engine = DownloadEngine::builder()
        .client(hpx::Client::new())
        .storage(Arc::new(TestStorage::default()))
        .build()
        .expect("build engine");

    let request = DownloadRequest::builder(format!("http://{addr}/file.bin"), &dest)
        .build()
        .expect("build request");
    let id = engine.add(request).expect("add download");

    // Wait for the scheduler to probe and fail.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let status = engine.status(id).expect("status");
        if status.state == hpx_dl::DownloadState::Failed {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "engine did not fail within deadline; state: {:?}",
            status.state
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // The engine must not have produced an empty "completed" file.
    assert!(
        !dest.exists(),
        "no destination file should be created for unknown-size downloads"
    );
}
