//! HTTP/3 throughput benchmark.
//!
//! Benchmarks 1000 GET requests over a single HTTP/3 (QUIC) connection
//! vs a single HTTP/2 connection. Uses `criterion` with `Throughput::Elements(1000)`.

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use hpx::{
    http3::{__test_connect_request, Http3Options, QuicConnector},
    tls::quic::build_quinn_endpoint,
};
use quinn::{Endpoint, ServerConfig, crypto::rustls::QuicServerConfig};
use rustls::{
    RootCertStore, ServerConfig as RustlsServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use tower::Service;

const REQUEST_COUNT: u64 = 1000;

// ---------------------------------------------------------------------------
// h3 helpers
// ---------------------------------------------------------------------------

/// Spawn a local h3 echo server and return a connected `SendRequest` handle.
///
/// The server responds to every GET with `200 OK` and no body.
async fn setup_h3() -> (
    tokio::task::JoinHandle<()>,
    hpx_h3::client::SendRequest<hpx_h3::quinn::OpenStreams, Bytes>,
    u16,
) {
    // 1. Generate a self-signed certificate (SAN: 127.0.0.1).
    let certified_key = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let cert_der: CertificateDer<'static> = certified_key.cert.der().clone();
    let key_der: PrivateKeyDer<'static> =
        PrivatePkcs8KeyDer::from(certified_key.signing_key.serialize_der()).into();

    // 2. Build the server-side rustls `ServerConfig` with ALPN `[b"h3"]`.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut server_rustls_config = RustlsServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .unwrap();
    server_rustls_config.alpn_protocols = vec![b"h3".to_vec()];

    // 3. Build the quinn server endpoint bound to an ephemeral loopback port.
    let server_quic_config = QuicServerConfig::try_from(Arc::new(server_rustls_config)).unwrap();
    let server_config = ServerConfig::with_crypto(Arc::new(server_quic_config));
    let server_endpoint = Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr: SocketAddr = server_endpoint.local_addr().unwrap();
    let port = server_addr.port();

    // 4. Spawn the server task. For each accepted QUIC connection, accept
    //    requests and respond with `200 OK` (no body). Drain the request body
    //    so the underlying `quinn::RecvStream` is marked `all_data_read`
    //    before drop.
    let jh = tokio::spawn(async move {
        while let Some(incoming) = server_endpoint.accept().await {
            match incoming.await {
                Ok(quinn_conn) => {
                    tokio::spawn(async move {
                        let h3_quinn_conn = hpx_h3::quinn::Connection::new(quinn_conn);
                        let mut h3_conn: hpx_h3::server::Connection<
                            hpx_h3::quinn::Connection,
                            Bytes,
                        > = match hpx_h3::server::Connection::new(h3_quinn_conn).await {
                            Ok(c) => c,
                            Err(_) => return,
                        };
                        loop {
                            match h3_conn.accept().await {
                                Ok(Some(resolver)) => {
                                    let (_req, mut stream) = match resolver.resolve_request().await
                                    {
                                        Ok(parts) => parts,
                                        Err(_) => continue,
                                    };
                                    let resp = http::Response::builder()
                                        .status(http::StatusCode::OK)
                                        .body(())
                                        .unwrap();
                                    if stream.send_response(resp).await.is_err() {
                                        continue;
                                    }
                                    while matches!(stream.recv_data().await, Ok(Some(_))) {}
                                    let _ = stream.finish().await;
                                }
                                Ok(None) => break,
                                Err(_) => break,
                            }
                        }
                    });
                }
                Err(_) => break,
            }
        }
    });

    // 5. Build the client-side rustls `ClientConfig` with ALPN `[b"h3"]`
    //    and the self-signed cert as a trusted root.
    let mut root_store = RootCertStore::empty();
    root_store.add(cert_der.clone()).unwrap();
    let client_provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut client_tls_config = rustls::ClientConfig::builder_with_provider(client_provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    client_tls_config.alpn_protocols = vec![b"h3".to_vec()];
    let tls_config: Arc<rustls::ClientConfig> = Arc::new(client_tls_config);

    // 6. Build the client `quinn::Endpoint` and transport config.
    let client_endpoint = build_quinn_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let transport_config = Arc::new(quinn::TransportConfig::default());

    // 7. Construct the `QuicConnector`.
    let mut connector = QuicConnector::new(
        client_endpoint,
        transport_config,
        tls_config,
        Http3Options::default(),
    );

    // 8. Satisfy `tower::Service::poll_ready` contract.
    let waker = futures_util::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    match connector.poll_ready(&mut cx) {
        std::task::Poll::Ready(Ok(())) => {}
        other => panic!("poll_ready should be Ok, got {other:?}"),
    }

    // 9. Connect and obtain the `SendRequest` handle.
    let uri: http::Uri = format!("https://127.0.0.1:{port}/").parse().unwrap();
    let connect_req = __test_connect_request(uri);
    let h3_conn = tokio::time::timeout(Duration::from_secs(5), connector.call(connect_req))
        .await
        .unwrap()
        .unwrap();

    (jh, h3_conn.send_request, port)
}

// ---------------------------------------------------------------------------
// h2 helpers
// ---------------------------------------------------------------------------

/// Spawn a local h2-capable echo server using hyper.
///
/// Returns the server's join handle and port.
async fn setup_h2() -> (tokio::task::JoinHandle<()>, u16) {
    use hyper::{body::Incoming, service::service_fn};
    use hyper_util::{rt::TokioIo, server::conn::auto::Builder};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let jh = tokio::spawn(async move {
        let builder = Builder::new(hyper_util::rt::TokioExecutor::new());
        loop {
            let accept_result = listener.accept().await;
            let (stream, _) = match accept_result {
                Ok(accepted) => accepted,
                Err(_) => break,
            };
            let io = TokioIo::new(stream);
            let svc = service_fn(|_req: hyper::Request<Incoming>| async move {
                Ok::<_, std::convert::Infallible>(http::Response::new(http_body_util::Full::new(
                    Bytes::new(),
                )))
            });
            let builder = builder.clone();
            tokio::spawn(async move {
                let _ = builder.serve_connection(io, svc).await;
            });
        }
    });

    (jh, port)
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// Benchmark 1000 GET requests over a single HTTP/3 connection.
fn bench_h3_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("http3_throughput");
    group.throughput(Throughput::Elements(REQUEST_COUNT));

    group.bench_function("h3_1000_gets", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let (_jh, send_request, port) = setup_h3().await;
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    for _ in 0..REQUEST_COUNT {
                        let mut sr = send_request.clone();
                        let req = http::Request::get(format!("https://127.0.0.1:{port}/"))
                            .body(())
                            .unwrap();
                        let mut stream = sr.send_request(req).await.unwrap();
                        stream.finish().await.unwrap();
                        let _resp = stream.recv_response().await.unwrap();
                        while let Some(_) = stream.recv_data().await.unwrap() {}
                    }
                    total += start.elapsed();
                }
                total
            })
        });
    });

    group.finish();
}

/// Benchmark 1000 GET requests over a single HTTP/2 connection.
fn bench_h2_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("http2_throughput");
    group.throughput(Throughput::Elements(REQUEST_COUNT));

    group.bench_function("h2_1000_gets", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let (_jh, port) = setup_h2().await;
                let client = hpx::Client::builder().build().unwrap();
                let url = format!("http://127.0.0.1:{port}/");
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    for _ in 0..REQUEST_COUNT {
                        let resp = client
                            .get(&url)
                            .version(http::Version::HTTP_2)
                            .send()
                            .await
                            .unwrap();
                        let _ = resp.bytes().await.unwrap();
                    }
                    total += start.elapsed();
                }
                total
            })
        });
    });

    group.finish();
}

criterion_group!(benches, bench_h3_throughput, bench_h2_throughput);
criterion_main!(benches);
