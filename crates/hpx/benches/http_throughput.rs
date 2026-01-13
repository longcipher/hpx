//! Benchmarks for HTTP request/response throughput.
//!
//! These benchmarks measure the overhead of request building, response parsing,
//! and connection handling.

use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use http::{HeaderMap, HeaderValue, Request, Response, StatusCode, Version};

/// Benchmark request building overhead
fn bench_request_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_building");

    // Simple GET request
    group.bench_function("simple_get", |b| {
        b.iter(|| {
            let _request = Request::builder()
                .method("GET")
                .uri("https://api.example.com/users/123")
                .body(())
                .unwrap();
        });
    });

    // GET request with headers
    group.bench_function("get_with_headers", |b| {
        b.iter(|| {
            let _request = Request::builder()
                .method("GET")
                .uri("https://api.example.com/users/123")
                .header("Accept", "application/json")
                .header("Authorization", "Bearer token123456789")
                .header("User-Agent", "hpx-benchmark/1.0")
                .header("X-Request-Id", "req-12345-67890")
                .body(())
                .unwrap();
        });
    });

    // POST request with JSON body
    let json_body = r#"{"name":"test","email":"test@example.com","age":30}"#;
    group.bench_function("post_with_json", |b| {
        b.iter(|| {
            let _request = Request::builder()
                .method("POST")
                .uri("https://api.example.com/users")
                .header("Content-Type", "application/json")
                .header("Content-Length", json_body.len().to_string())
                .body(json_body)
                .unwrap();
        });
    });

    group.finish();
}

/// Benchmark response parsing overhead
fn bench_response_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("response_parsing");

    // Small response
    let small_body = Bytes::from_static(b"Hello, World!");
    group.throughput(Throughput::Bytes(small_body.len() as u64));

    group.bench_with_input(
        BenchmarkId::new("parse", "small_response"),
        &small_body,
        |b, body| {
            b.iter(|| {
                let _response = Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/plain")
                    .header("Content-Length", body.len().to_string())
                    .body(body.clone())
                    .unwrap();
            });
        },
    );

    // JSON response
    let json_body = Bytes::from_static(
        br#"{"id":1,"name":"test","email":"test@example.com","created_at":"2024-01-01T00:00:00Z"}"#,
    );
    group.throughput(Throughput::Bytes(json_body.len() as u64));

    group.bench_with_input(
        BenchmarkId::new("parse", "json_response"),
        &json_body,
        |b, body| {
            b.iter(|| {
                let _response = Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/json")
                    .header("Content-Length", body.len().to_string())
                    .body(body.clone())
                    .unwrap();
            });
        },
    );

    // Large JSON response
    let large_body: Bytes = {
        let mut s = String::from("[");
        for i in 0..1000 {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                r#"{{"id":{},"name":"item{}","value":{}.5}}"#,
                i, i, i
            ));
        }
        s.push(']');
        Bytes::from(s)
    };
    group.throughput(Throughput::Bytes(large_body.len() as u64));

    group.bench_with_input(
        BenchmarkId::new("parse", "large_json_response"),
        &large_body,
        |b, body| {
            b.iter(|| {
                let _response = Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/json")
                    .header("Content-Length", body.len().to_string())
                    .body(body.clone())
                    .unwrap();
            });
        },
    );

    group.finish();
}

/// Benchmark header map operations
fn bench_header_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("header_operations");

    // Insert headers
    group.bench_function("insert_10_headers", |b| {
        b.iter(|| {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", HeaderValue::from_static("application/json"));
            headers.insert("content-length", HeaderValue::from_static("1024"));
            headers.insert("accept", HeaderValue::from_static("application/json"));
            headers.insert("authorization", HeaderValue::from_static("Bearer token"));
            headers.insert("user-agent", HeaderValue::from_static("hpx/1.0"));
            headers.insert("x-request-id", HeaderValue::from_static("req-123"));
            headers.insert("x-api-version", HeaderValue::from_static("v2"));
            headers.insert("cache-control", HeaderValue::from_static("no-cache"));
            headers.insert("accept-encoding", HeaderValue::from_static("gzip, br"));
            headers.insert("connection", HeaderValue::from_static("keep-alive"));
            headers
        });
    });

    // Lookup headers
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("content-length", HeaderValue::from_static("1024"));
    headers.insert("accept", HeaderValue::from_static("application/json"));
    headers.insert("authorization", HeaderValue::from_static("Bearer token"));
    headers.insert("user-agent", HeaderValue::from_static("hpx/1.0"));
    headers.insert("x-request-id", HeaderValue::from_static("req-123"));

    group.bench_function("lookup_existing_header", |b| {
        b.iter(|| headers.get("content-type"));
    });

    group.bench_function("lookup_missing_header", |b| {
        b.iter(|| headers.get("x-nonexistent"));
    });

    // Clone headers
    group.bench_function("clone_headers", |b| {
        b.iter(|| headers.clone());
    });

    group.finish();
}

/// Benchmark version checking
fn bench_version_handling(c: &mut Criterion) {
    let mut group = c.benchmark_group("version_handling");

    group.bench_function("create_http11_request", |b| {
        b.iter(|| {
            Request::builder()
                .version(Version::HTTP_11)
                .method("GET")
                .uri("https://example.com")
                .body(())
                .unwrap()
        });
    });

    group.bench_function("create_http2_request", |b| {
        b.iter(|| {
            Request::builder()
                .version(Version::HTTP_2)
                .method("GET")
                .uri("https://example.com")
                .body(())
                .unwrap()
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_request_building,
    bench_response_parsing,
    bench_header_operations,
    bench_version_handling
);
criterion_main!(benches);
