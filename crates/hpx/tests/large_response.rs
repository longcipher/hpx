#![allow(missing_docs)]
//! Regression tests for large (~120 KB) response bodies.
//!
//! A user reported that responses around 120 KB could not be handled by
//! hpx. These tests pin down the expected behaviour across the transfer
//! encodings and compression codecs a real client may encounter.

mod support;

use std::io::Write;

use flate2::{Compression, write::GzEncoder};
use futures_util::stream::StreamExt;
use support::server;

/// ~120 KB payload size used across every scenario.
const PAYLOAD_LEN: usize = 120 * 1024;
const CHUNK_SIZE: usize = 4096;

fn make_payload(len: usize) -> Vec<u8> {
    // Deterministic pseudo-random bytes so compression cannot collapse them.
    let mut payload = Vec::with_capacity(len);
    let mut seed = 0x1234_5678_u32;
    while payload.len() < len {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        payload.push((seed >> 24) as u8);
    }
    payload
}

fn assert_payload_matches(actual: &[u8]) {
    let expected = make_payload(PAYLOAD_LEN);
    assert_eq!(
        actual.len(),
        expected.len(),
        "body length mismatch: got {}, want {}",
        actual.len(),
        expected.len()
    );
    assert_eq!(actual, expected, "body content mismatch");
}

async fn collect_bytes(uri: String) -> bytes::Bytes {
    let res = hpx::get(uri).send().await.expect("response");
    assert_eq!(res.status(), 200);
    res.bytes().await.expect("body")
}

/// Serve `payload` in fixed-size chunks as a streaming body.
///
/// With a `content-length` header this exercises the framed decoder; without
/// one hyper switches to chunked transfer encoding.
fn chunked_body(payload: Vec<u8>, with_content_length: bool) -> http::Response<hpx::Body> {
    let stream = futures_util::stream::unfold((payload, 0), move |(payload, pos)| async move {
        let chunk = payload.chunks(CHUNK_SIZE).nth(pos)?.to_vec();
        Some((chunk, (payload, pos + 1)))
    })
    .map(Ok::<_, std::convert::Infallible>);

    let mut builder = http::Response::builder();
    if with_content_length {
        builder = builder.header("content-length", PAYLOAD_LEN.to_string());
    } else {
        builder = builder.header("transfer-encoding", "chunked");
    }
    builder.body(hpx::Body::wrap_stream(stream)).unwrap()
}

#[tokio::test]
async fn h1_content_length_large_body() {
    let server = server::http(move |_req| {
        let body = chunked_body(make_payload(PAYLOAD_LEN), true);
        async move { body }
    });

    let body = collect_bytes(format!("http://{}/big", server.addr())).await;
    assert_payload_matches(&body);
}

#[tokio::test]
async fn h1_chunked_large_body() {
    let server = server::http(move |_req| {
        let body = chunked_body(make_payload(PAYLOAD_LEN), false);
        async move { body }
    });

    let body = collect_bytes(format!("http://{}/chunked", server.addr())).await;
    assert_payload_matches(&body);
}

#[tokio::test]
async fn gzip_large_body() {
    let server = server::http(move |req| async move {
        assert!(
            req.headers()["accept-encoding"]
                .to_str()
                .unwrap()
                .contains("gzip"),
            "client should advertise gzip support"
        );

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&make_payload(PAYLOAD_LEN)).unwrap();
        let compressed = encoder.finish().unwrap();

        let stream = futures_util::stream::unfold((compressed, 0), move |(c, pos)| async move {
            let chunk = c.chunks(CHUNK_SIZE).nth(pos)?.to_vec();
            Some((chunk, (c, pos + 1)))
        })
        .map(Ok::<_, std::convert::Infallible>);

        http::Response::builder()
            .header("content-encoding", "gzip")
            .body(hpx::Body::wrap_stream(stream))
            .unwrap()
    });

    let body = collect_bytes(format!("http://{}/gzipped", server.addr())).await;
    assert_payload_matches(&body);
}

#[tokio::test]
async fn h2_prior_knowledge_large_body() {
    // hyper-util's auto builder serves h2c when the client speaks the
    // preface; `http2_only` makes hpx open the connection with prior
    // knowledge instead of upgrading from HTTP/1.
    let server = server::http(move |_req| {
        let body = chunked_body(make_payload(PAYLOAD_LEN), true);
        async move { body }
    });

    let res = hpx::Client::builder()
        .http2_only()
        .build()
        .expect("client")
        .get(format!("http://{}/big", server.addr()))
        .send()
        .await
        .expect("response");
    assert_eq!(res.version(), hpx::Version::HTTP_2);
    let body = res.bytes().await.expect("body");
    assert_payload_matches(&body);
}

#[tokio::test]
async fn keep_alive_reuse_after_large_body() {
    // A large response followed by a second request on the same connection:
    // leftover buffered bytes must not corrupt the next response.
    let server = server::http(move |req| {
        let path = req.uri().path().to_string();
        async move {
            if path == "/big" {
                chunked_body(make_payload(PAYLOAD_LEN), true)
            } else {
                http::Response::builder()
                    .header("content-length", 5)
                    .body(hpx::Body::from("small"))
                    .unwrap()
            }
        }
    });

    let base = format!("http://{}", server.addr());
    let big = collect_bytes(format!("{base}/big")).await;
    assert_payload_matches(&big);

    // Same client => same pooled connection.
    let small = hpx::get(format!("{base}/small"))
        .send()
        .await
        .expect("second response");
    assert_eq!(small.status(), 200);
    assert_eq!(small.bytes().await.expect("body").as_ref(), b"small");
}

#[tokio::test]
async fn bytes_stream_large_body() {
    use futures_util::StreamExt;

    let server = server::http(move |_req| {
        let body = chunked_body(make_payload(PAYLOAD_LEN), true);
        async move { body }
    });

    let res = hpx::get(format!("http://{}/stream", server.addr()))
        .send()
        .await
        .expect("response");
    let mut stream = res.bytes_stream();
    let mut collected = Vec::with_capacity(PAYLOAD_LEN);
    while let Some(chunk) = stream.next().await {
        collected.extend_from_slice(&chunk.expect("chunk"));
    }
    assert_payload_matches(&collected);
}

#[tokio::test]
async fn large_response_headers() {
    // ~120 KB of response headers spread over FEW headers (under the 100
    // header count limit): a single huge value must still parse.
    let server = server::http(move |_req| async move {
        let big_value = "x".repeat(120 * 1024);
        http::Response::builder()
            .header("content-length", 5)
            .header("x-big-data", big_value.as_str())
            .body(hpx::Body::from("small"))
            .unwrap()
    });

    let res = hpx::get(format!("http://{}/big-header", server.addr()))
        .send()
        .await
        .expect("response with ~120KB single header");
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()["x-big-data"].len(),
        120 * 1024,
        "large single header value must survive"
    );
}

#[tokio::test]
async fn many_response_headers_over_default_limit() {
    // More than the parser's default 100-header budget: servers behind CDNs
    // or auth gateways routinely emit hundreds of Set-Cookie lines. The
    // parser must escalate its header budget automatically instead of
    // failing with `Parse(TooLarge)`.
    let cookie_count = 600;
    let server = server::http(move |_req| async move {
        let mut builder = http::Response::builder().header("content-length", 5);
        for i in 0..cookie_count {
            builder = builder.header(
                "set-cookie",
                format!("session_{i:03}=abcdef0123456789abcdef0123456789; Path=/; HttpOnly"),
            );
        }
        builder.body(hpx::Body::from("small")).unwrap()
    });

    // Default client: the parser escalates its budget transparently.
    let res = hpx::get(format!("http://{}/cookies", server.addr()))
        .send()
        .await
        .expect("header-heavy response must parse without extra configuration");
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers().get_all("set-cookie").iter().count(),
        cookie_count
    );

    // An explicitly pinned cap still works and stays respected.
    let client = hpx::Client::builder()
        .http1_options(
            hpx::http1::Http1Options::builder()
                .max_headers(cookie_count * 2)
                .build(),
        )
        .build()
        .expect("client");
    let res = client
        .get(format!("http://{}/cookies", server.addr()))
        .send()
        .await
        .expect("response with raised max_headers");
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers().get_all("set-cookie").iter().count(),
        cookie_count,
        "all cookies must survive"
    );
}
