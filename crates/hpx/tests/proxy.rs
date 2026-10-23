#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]
mod support;
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
use std::sync::Arc;
#[cfg(feature = "boring-tls")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "rustls-tls")]
use std::{convert::Infallible, io::Cursor};
use std::{env, sync::LazyLock};

#[cfg(feature = "boring-tls")]
use boring::ssl::{SslAcceptor, SslFiletype, SslMethod, SslVerifyMode};
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
use bytes::Bytes;
use hpx::Client;
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
use hpx::tls::{CertStore, Identity};
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
use http_body_util::Full;
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
use hyper::{Response, server::conn::http1, service::service_fn};
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
use hyper_util::rt::TokioIo;
use support::server;
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
use tokio::net::TcpListener;
use tokio::sync::Mutex;
#[cfg(feature = "rustls-tls")]
use tokio_rustls::{TlsAcceptor, rustls};

// serialize tests that read from / write to environment variables
static HTTP_PROXY_ENV_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[tokio::test]
async fn http_proxy() {
    let url = "http://hyper.rs.local/prox";
    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), url);
        assert_eq!(req.headers()["host"], "hyper.rs.local");

        async { http::Response::default() }
    });

    let proxy = format!("http://{}", server.addr());

    let res = Client::builder()
        .proxy(hpx::Proxy::http(&proxy).unwrap())
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

#[tokio::test]
async fn http_proxy_basic_auth() {
    let url = "http://hyper.rs.local/prox";
    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), url);
        assert_eq!(req.headers()["host"], "hyper.rs.local");
        assert_eq!(
            req.headers()["proxy-authorization"],
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );

        async { http::Response::default() }
    });

    let proxy = format!("http://{}", server.addr());

    let res = Client::builder()
        .proxy(
            hpx::Proxy::http(&proxy)
                .unwrap()
                .basic_auth("Aladdin", "open sesame"),
        )
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

#[tokio::test]
async fn http_proxy_basic_auth_parsed() {
    let url = "http://hyper.rs.local/prox";
    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), url);
        assert_eq!(req.headers()["host"], "hyper.rs.local");
        assert_eq!(
            req.headers()["proxy-authorization"],
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );

        async { http::Response::default() }
    });

    let proxy = format!("http://Aladdin:open%20sesame@{}", server.addr());

    let res = Client::builder()
        .proxy(hpx::Proxy::http(&proxy).unwrap())
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);

    let res = hpx::get(url)
        .proxy(hpx::Proxy::http(&proxy).unwrap())
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

// This test fails with hickory DNS resolver because it tries to resolve the fake domain
// before connecting through the proxy. Works with the GAI resolver.
#[ignore = "fails with hickory DNS resolver; works with GAI resolver"]
#[tokio::test]
async fn system_http_proxy_basic_auth_parsed() {
    let url = "http://hyper.rs.local/prox";
    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), url);
        assert_eq!(req.headers()["host"], "hyper.rs.local");
        assert_eq!(
            req.headers()["proxy-authorization"],
            "Basic QWxhZGRpbjpvcGVuc2VzYW1l"
        );

        async { http::Response::default() }
    });

    // avoid races with other tests that change "http_proxy"
    let _env_lock = HTTP_PROXY_ENV_MUTEX.lock().await;

    // save system setting first.
    let system_proxy = env::var("http_proxy");

    // set-up http proxy.
    unsafe {
        env::set_var(
            "http_proxy",
            format!("http://Aladdin:opensesame@{}", server.addr()),
        );
    }

    let res = Client::builder()
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);

    // reset user setting.
    unsafe {
        match system_proxy {
            Err(_) => env::remove_var("http_proxy"),
            Ok(proxy) => env::set_var("http_proxy", proxy),
        }
    }
}

#[tokio::test]
async fn test_no_proxy() {
    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), "/4");

        async { http::Response::default() }
    });
    let proxy = format!("http://{}", server.addr());
    let url = format!("http://{}/4", server.addr());

    // set up proxy and use no_proxy to clear up client builder proxies.
    let res = Client::builder()
        .proxy(hpx::Proxy::http(&proxy).unwrap())
        .no_proxy()
        .build()
        .unwrap()
        .get(&url)
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url.as_str());
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

// This test fails with hickory DNS resolver because it tries to resolve the fake domain
// before connecting through the proxy. Works with the GAI resolver.
#[ignore = "fails with hickory DNS resolver; works with GAI resolver"]
#[tokio::test]
async fn test_using_system_proxy() {
    let url = "http://not.a.real.sub.hyper.rs.local/prox";
    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), url);
        assert_eq!(req.headers()["host"], "not.a.real.sub.hyper.rs.local");

        async { http::Response::default() }
    });

    // avoid races with other tests that change "http_proxy"
    let _env_lock = HTTP_PROXY_ENV_MUTEX.lock().await;

    // save system setting first.
    let system_proxy = env::var("http_proxy");
    // set-up http proxy.
    unsafe {
        env::set_var("http_proxy", format!("http://{}", server.addr()));
    }
    // system proxy is used by default
    let res = hpx::get(url).send().await.unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);

    // reset user setting.
    unsafe {
        match system_proxy {
            Err(_) => env::remove_var("http_proxy"),
            Ok(proxy) => env::set_var("http_proxy", proxy),
        }
    }
}

#[tokio::test]
async fn all_proxy_with_custom_auth() {
    let url = "http://hyper.rs.local/prox";
    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), url);
        assert_eq!(req.headers()["host"], "hyper.rs.local");
        assert_eq!(
            req.headers()["proxy-authorization"],
            "Bearer custom-proxy-token"
        );

        async { http::Response::default() }
    });

    let proxy = format!("http://{}", server.addr());

    let res = Client::builder()
        .proxy(
            hpx::Proxy::all(&proxy)
                .unwrap()
                .custom_http_auth(http::HeaderValue::from_static("Bearer custom-proxy-token")),
        )
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

#[tokio::test]
async fn http_over_http() {
    let url = "http://hyper.rs.local/prox";

    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), url);
        assert_eq!(req.headers()["host"], "hyper.rs.local");

        async { http::Response::default() }
    });

    let proxy = format!("http://{}", server.addr());

    let res = Client::builder()
        .proxy(hpx::Proxy::http(&proxy).unwrap())
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

#[tokio::test]
async fn http_proxy_custom_headers() {
    let url = "http://hyper.rs.local/prox";
    let server = server::http(move |req| {
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri(), url);
        assert_eq!(req.headers()["host"], "hyper.rs.local");
        assert_eq!(
            req.headers()["proxy-authorization"],
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(req.headers()["x-custom-header"], "value");

        async { http::Response::default() }
    });

    let proxy = format!("http://Aladdin:open%20sesame@{}", server.addr());

    let proxy = hpx::Proxy::http(&proxy).unwrap().custom_http_headers({
        let mut headers = http::HeaderMap::new();
        headers.insert("x-custom-header", "value".parse().unwrap());
        headers
    });

    let res = Client::builder()
        .proxy(proxy.clone())
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);

    let res = hpx::get(url).proxy(proxy).send().await.unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

#[tokio::test]
async fn tunnel_detects_auth_required() {
    let url = "https://hyper.rs.local/prox";

    let server = server::http(move |req| {
        assert_eq!(req.method(), "CONNECT");
        assert_eq!(req.uri(), "hyper.rs.local:443");
        assert!(
            !req.headers()
                .contains_key(http::header::PROXY_AUTHORIZATION)
        );

        async {
            let mut res = http::Response::default();
            *res.status_mut() = http::StatusCode::PROXY_AUTHENTICATION_REQUIRED;
            res
        }
    });

    let proxy = format!("http://{}", server.addr());

    let err = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::https(&proxy).unwrap())
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap_err();

    let err = support::error::inspect(err).pop().unwrap();
    assert!(
        err.contains("auth"),
        "proxy auth err expected, got: {err:?}"
    );
}

#[tokio::test]
async fn tunnel_includes_proxy_auth() {
    let url = "https://hyper.rs.local/prox";

    let server = server::http(move |req| {
        assert_eq!(req.method(), "CONNECT");
        assert_eq!(req.uri(), "hyper.rs.local:443");
        assert_eq!(
            req.headers()["proxy-authorization"],
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );

        async {
            // return 400 to not actually deal with TLS tunneling
            let mut res = http::Response::default();
            *res.status_mut() = http::StatusCode::BAD_REQUEST;
            res
        }
    });

    let proxy = format!("http://Aladdin:open%20sesame@{}", server.addr());

    let err = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::https(&proxy).unwrap())
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap_err();

    let err = support::error::inspect(err).pop().unwrap();
    assert!(
        err.contains("unsuccessful"),
        "tunnel unsuccessful expected, got: {err:?}"
    );
}

#[tokio::test]
async fn tunnel_includes_user_agent() {
    let url = "https://hyper.rs.local/prox";

    let server = server::http(move |req| {
        assert_eq!(req.method(), "CONNECT");
        assert_eq!(req.uri(), "hyper.rs.local:443");
        assert_eq!(req.headers()["user-agent"], "hpx-test");

        async {
            // return 400 to not actually deal with TLS tunneling
            let mut res = http::Response::default();
            *res.status_mut() = http::StatusCode::BAD_REQUEST;
            res
        }
    });

    let proxy = format!("http://{}", server.addr());

    let err = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::https(&proxy).unwrap().custom_http_headers({
            let mut headers = http::HeaderMap::new();
            headers.insert("user-agent", "hpx-test".parse().unwrap());
            headers
        }))
        .user_agent("hpx-test")
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap_err();

    let err = support::error::inspect(err).pop().unwrap();
    assert!(
        err.contains("unsuccessful"),
        "tunnel unsuccessful expected, got: {err:?}"
    );
}

#[tokio::test]
async fn proxy_tunnel_connect_error() {
    let client = Client::builder()
        .cert_verification(false)
        .no_proxy()
        .build()
        .unwrap();

    let invalid_proxies = vec![
        "http://invalid.proxy:8080",
        "https://invalid.proxy:8080",
        "socks4://invalid.proxy:8080",
        "socks4a://invalid.proxy:8080",
        "socks5://invalid.proxy:8080",
        "socks5h://invalid.proxy:8080",
    ];

    let target_urls = ["https://example.com", "http://example.com"];

    for proxy in invalid_proxies {
        for url in target_urls {
            let err = client
                .get(url)
                .proxy(hpx::Proxy::all(proxy).unwrap())
                .send()
                .await
                .unwrap_err();

            assert!(
                err.is_proxy_connect(),
                "proxy connect error expected, got: {err:?}"
            );
        }
    }
}

#[tokio::test]
async fn custom_proxy_routes_per_request() {
    let url = "http://hyper.rs.local/prox";

    let server = server::http(move |req| {
        assert_eq!(req.uri(), url);

        async { http::Response::default() }
    });

    let proxy = format!("http://{}", server.addr());

    let proxy_addr = proxy.clone();
    let client = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::custom(move |uri: &http::Uri| {
            (uri.host() == Some("hyper.rs.local")).then(|| proxy_addr.clone())
        }))
        .build()
        .unwrap();

    let res = client.get(url).send().await.unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

#[tokio::test]
async fn custom_proxy_can_skip_a_request() {
    // The proxy would fail the assertion below if it were ever contacted, so
    // reaching the origin proves the custom rule returned `None`.
    let server = server::http(move |_req| async {
        let mut res = http::Response::default();
        *res.status_mut() = http::StatusCode::IM_A_TEAPOT;
        res
    });

    let origin = format!("http://{}", server.addr());

    let client = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::custom(move |_: &http::Uri| -> Option<&str> {
            None
        }))
        .build()
        .unwrap();

    // The custom rule never matches, so the client connects directly. The
    // origin is addressed by IP, so no DNS lookup is required.
    let res = client.get(&origin).send().await.unwrap();

    assert_eq!(res.status(), hpx::StatusCode::IM_A_TEAPOT);
}

#[tokio::test]
async fn request_no_proxy_bypasses_client_proxy() {
    // The proxy must never be contacted for this request.
    let proxy_server = server::http(move |_req| async {
        let mut res = http::Response::default();
        *res.status_mut() = http::StatusCode::BAD_GATEWAY;
        res
    });

    let proxy = format!("http://{}", proxy_server.addr());

    let origin_server = server::http(move |req| {
        assert_eq!(req.uri().path(), "/");

        async { http::Response::default() }
    });

    let origin = format!("http://{}", origin_server.addr());

    let client = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::all(&proxy).unwrap())
        .build()
        .unwrap();

    // Without the bypass the request would be answered by the proxy with
    // 502. With the bypass it reaches the origin.
    let res = client.get(&origin).no_proxy().send().await.unwrap();

    assert_eq!(res.status(), hpx::StatusCode::OK);
}

#[tokio::test]
async fn request_proxy_is_authoritative() {
    let url = "http://hyper.rs.local/prox";

    let client_proxy_server = server::http(move |_req| async {
        let mut res = http::Response::default();
        *res.status_mut() = http::StatusCode::BAD_GATEWAY;
        res
    });

    let client_proxy = format!("http://{}", client_proxy_server.addr());

    let request_proxy_server = server::http(move |req| {
        assert_eq!(req.uri(), url);

        async { http::Response::default() }
    });

    let request_proxy = format!("http://{}", request_proxy_server.addr());

    let client = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::all(&client_proxy).unwrap())
        .build()
        .unwrap();

    // The per-request proxy replaces the client-level one; it must not fall
    // back to the client-level proxy when it does not match.
    let res = client
        .get(url)
        .proxy(hpx::Proxy::http(&request_proxy).unwrap())
        .send()
        .await
        .unwrap();

    assert_eq!(res.uri(), url);
    assert_eq!(res.status(), hpx::StatusCode::OK);
}

#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
const CA_CERT_PEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/mtls/ca.crt"
));
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
const CLIENT_CERT_PEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/mtls/client.crt"
));
#[cfg(any(feature = "boring-tls", feature = "rustls-tls"))]
const CLIENT_KEY_PEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/mtls/client.key"
));
#[cfg(feature = "boring-tls")]
const SERVER_CERT_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/server.crt");
#[cfg(feature = "boring-tls")]
const SERVER_KEY_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/server.key");
#[cfg(feature = "boring-tls")]
const CA_CERT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/ca.crt");
#[cfg(feature = "rustls-tls")]
const SERVER_CERT_PEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/mtls/server.crt"
));
#[cfg(feature = "rustls-tls")]
const SERVER_KEY_PEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/mtls/server.key"
));

#[cfg(feature = "boring-tls")]
fn mtls_tls_acceptor() -> SslAcceptor {
    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor
        .set_certificate_chain_file(SERVER_CERT_PATH)
        .unwrap();
    acceptor
        .set_private_key_file(SERVER_KEY_PATH, SslFiletype::PEM)
        .unwrap();
    acceptor.set_ca_file(CA_CERT_PATH).unwrap();
    acceptor.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
    acceptor.check_private_key().unwrap();
    acceptor.build()
}

#[cfg(feature = "boring-tls")]
fn proxy_tls_acceptor() -> SslAcceptor {
    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor
        .set_certificate_chain_file(SERVER_CERT_PATH)
        .unwrap();
    acceptor
        .set_private_key_file(SERVER_KEY_PATH, SslFiletype::PEM)
        .unwrap();
    acceptor.set_ca_file(CA_CERT_PATH).unwrap();
    acceptor.set_verify(SslVerifyMode::PEER);
    acceptor.check_private_key().unwrap();
    acceptor.build()
}

#[cfg(feature = "boring-tls")]
async fn spawn_origin_mtls_server() -> (tokio::task::JoinHandle<()>, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = mtls_tls_acceptor();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let stream = tokio_boring::accept(&acceptor, stream).await.unwrap();
        let service = service_fn(|_request| async {
            let mut response = Response::new(Full::new(Bytes::from_static(b"mtls-ok")));
            response.headers_mut().insert(
                http::header::CONNECTION,
                http::HeaderValue::from_static("close"),
            );
            Ok::<_, std::convert::Infallible>(response)
        });

        http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await
            .unwrap();
    });

    (server, port)
}

#[cfg(feature = "boring-tls")]
async fn spawn_https_proxy(
    proxy_saw_client_cert: Arc<AtomicBool>,
) -> (tokio::task::JoinHandle<()>, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = proxy_tls_acceptor();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let stream = tokio_boring::accept(&acceptor, stream).await.unwrap();
        proxy_saw_client_cert.store(stream.ssl().peer_certificate().is_some(), Ordering::SeqCst);

        let service = service_fn(|req| async move {
            assert_eq!(req.method(), http::Method::CONNECT);

            let authority = req.uri().authority().cloned().unwrap();
            tokio::spawn(async move {
                let upgraded = hyper::upgrade::on(req).await.unwrap();
                let mut upgraded = TokioIo::new(upgraded);
                let mut origin = tokio::net::TcpStream::connect(authority.to_string())
                    .await
                    .unwrap();

                tokio::io::copy_bidirectional(&mut upgraded, &mut origin)
                    .await
                    .unwrap();
            });

            Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::new())))
        });

        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .with_upgrades()
            .await
            .unwrap();
    });

    (server, port)
}

#[cfg(feature = "rustls-tls")]
fn parse_certs(pem: &[u8]) -> Vec<rustls::pki_types::CertificateDer<'static>> {
    rustls_pemfile::certs(&mut Cursor::new(pem))
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[cfg(feature = "rustls-tls")]
fn parse_key(pem: &[u8]) -> rustls::pki_types::PrivateKeyDer<'static> {
    rustls_pemfile::private_key(&mut Cursor::new(pem))
        .unwrap()
        .unwrap()
}

#[cfg(feature = "rustls-tls")]
fn rustls_mtls_tls_acceptor() -> TlsAcceptor {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    let (added, ignored) = roots.add_parsable_certificates(parse_certs(CA_CERT_PEM));
    assert_eq!(added, 1);
    assert_eq!(ignored, 0);

    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        provider.clone(),
    )
    .build()
    .unwrap();

    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(parse_certs(SERVER_CERT_PEM), parse_key(SERVER_KEY_PEM))
        .unwrap();

    TlsAcceptor::from(Arc::new(config))
}

#[cfg(feature = "rustls-tls")]
fn rustls_proxy_tls_acceptor() -> TlsAcceptor {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(parse_certs(SERVER_CERT_PEM), parse_key(SERVER_KEY_PEM))
        .unwrap();

    TlsAcceptor::from(Arc::new(config))
}

#[cfg(feature = "rustls-tls")]
async fn spawn_rustls_origin_mtls_server() -> (tokio::task::JoinHandle<()>, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = rustls_mtls_tls_acceptor();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let stream = acceptor.accept(stream).await.unwrap();
        let service = service_fn(|_request| async {
            let mut response = Response::new(Full::new(Bytes::from_static(b"mtls-ok")));
            response.headers_mut().insert(
                http::header::CONNECTION,
                http::HeaderValue::from_static("close"),
            );
            Ok::<_, Infallible>(response)
        });

        http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await
            .unwrap();
    });

    (server, port)
}

#[cfg(feature = "rustls-tls")]
async fn spawn_rustls_https_proxy() -> (tokio::task::JoinHandle<()>, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = rustls_proxy_tls_acceptor();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let stream = acceptor.accept(stream).await.unwrap();

        let service = service_fn(|req| async move {
            assert_eq!(req.method(), http::Method::CONNECT);

            let authority = req.uri().authority().cloned().unwrap();
            tokio::spawn(async move {
                let upgraded = hyper::upgrade::on(req).await.unwrap();
                let mut upgraded = TokioIo::new(upgraded);
                let mut origin = tokio::net::TcpStream::connect(authority.to_string())
                    .await
                    .unwrap();

                tokio::io::copy_bidirectional(&mut upgraded, &mut origin)
                    .await
                    .unwrap();
            });

            Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::new())))
        });

        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .with_upgrades()
            .await
            .unwrap();
    });

    (server, port)
}

#[cfg(feature = "boring-tls")]
#[tokio::test]
async fn https_proxy_does_not_consume_origin_mtls_identity() {
    let (origin_server, origin_port) = spawn_origin_mtls_server().await;
    let proxy_saw_client_cert = Arc::new(AtomicBool::new(false));
    let (proxy_server, proxy_port) = spawn_https_proxy(proxy_saw_client_cert.clone()).await;

    let mut pem = CLIENT_CERT_PEM.to_vec();
    pem.extend_from_slice(CLIENT_KEY_PEM);

    let cert_store = CertStore::builder()
        .add_pem_cert(CA_CERT_PEM)
        .build()
        .unwrap();
    let identity = Identity::from_pem(&pem).unwrap();
    let proxy = hpx::Proxy::https(format!("https://localhost:{proxy_port}")).unwrap();

    let client = Client::builder()
        .no_proxy()
        .proxy(proxy)
        .cert_store(cert_store)
        .identity(identity)
        .build()
        .unwrap();

    let response = client
        .get(format!("https://localhost:{origin_port}/"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.text().await.unwrap(), "mtls-ok");
    assert!(
        !proxy_saw_client_cert.load(Ordering::SeqCst),
        "origin client certificate must only be presented inside the CONNECT tunnel"
    );

    proxy_server.await.unwrap();
    origin_server.await.unwrap();
}

#[cfg(feature = "rustls-tls")]
#[tokio::test]
async fn https_proxy_supports_origin_mtls_with_rustls() {
    let (origin_server, origin_port) = spawn_rustls_origin_mtls_server().await;
    let (proxy_server, proxy_port) = spawn_rustls_https_proxy().await;

    let cert_store = CertStore::builder()
        .add_pem_cert(CA_CERT_PEM)
        .build()
        .unwrap();
    let identity = Identity::from_pkcs8_pem(CLIENT_CERT_PEM, CLIENT_KEY_PEM).unwrap();
    let proxy = hpx::Proxy::https(format!("https://localhost:{proxy_port}")).unwrap();

    let client = Client::builder()
        .no_proxy()
        .proxy(proxy)
        .cert_store(cert_store)
        .identity(identity)
        .build()
        .unwrap();

    let response = client
        .get(format!("https://localhost:{origin_port}/"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.text().await.unwrap(), "mtls-ok");

    proxy_server.await.unwrap();
    origin_server.await.unwrap();
}

/// A proxy that answers the first `CONNECT` with a `407` challenge and
/// accepts the second one when it carries the computed credentials.
struct ChallengeProxy {
    /// The `Proxy-Authorization` values the proxy received, in order.
    received: Arc<std::sync::Mutex<Vec<String>>>,
    /// The number of `CONNECT` requests the proxy answered.
    connects: Arc<std::sync::atomic::AtomicUsize>,
}

impl ChallengeProxy {
    fn new() -> Self {
        Self {
            received: Arc::new(std::sync::Mutex::new(Vec::new())),
            connects: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Records the `Proxy-Authorization` header of a request.
    fn record(&self, req: &http::Request<hyper::body::Incoming>) {
        let value = req
            .headers()
            .get("proxy-authorization")
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        if let Some(value) = value {
            self.received
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(value);
        }
    }
}

/// The client answers a `Digest` challenge and retries the `CONNECT`.
#[tokio::test]
async fn tunnel_answers_a_digest_challenge() {
    let state = Arc::new(ChallengeProxy::new());

    let server = {
        let state = state.clone();
        server::http(move |req| {
            let state = state.clone();
            async move {
                state.record(&req);
                let leg = state
                    .connects
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut res = http::Response::default();
                if leg == 0 {
                    *res.status_mut() = http::StatusCode::PROXY_AUTHENTICATION_REQUIRED;
                    res.headers_mut().insert(
                        "proxy-authenticate",
                        "Digest realm=\"proxy\", nonce=\"abc123\", qop=\"auth\""
                            .parse()
                            .unwrap(),
                    );
                    res.headers_mut()
                        .insert("content-length", "0".parse().unwrap());
                } else {
                    *res.status_mut() = http::StatusCode::OK;
                }
                res
            }
        })
    };

    let proxy = format!("http://{}", server.addr());

    let err = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::https(&proxy).unwrap().auth(
            "Aladdin",
            "open sesame",
            hpx::ProxyAuthMethod::Digest,
        ))
        .build()
        .unwrap()
        .get("https://hyper.rs.local/prox")
        .send()
        .await
        .unwrap_err();

    // The tunnel was established (the second CONNECT returned 200) and the
    // TLS handshake then failed, because the proxy is not an origin server.
    assert!(
        err.is_connect() || err.is_request(),
        "unexpected error: {err:?}"
    );

    // The first CONNECT carries no credentials, the second one carries the
    // computed Digest response.
    let received = state
        .received
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    assert_eq!(
        received.len(),
        1,
        "the challenge must be answered: {received:?}"
    );
    assert!(
        received[0].starts_with("Digest "),
        "unexpected authorization: {}",
        received[0]
    );
    assert!(
        received[0].contains("username=\"Aladdin\""),
        "{}",
        received[0]
    );
    assert!(received[0].contains("nonce=\"abc123\""), "{}", received[0]);
    assert!(received[0].contains("response=\""), "{}", received[0]);
}

/// The client answers an NTLM challenge with the three-leg handshake.
#[tokio::test]
async fn tunnel_answers_an_ntlm_challenge() {
    let state = Arc::new(ChallengeProxy::new());

    // A minimal NTLM type-2 message: the signature, the message type, empty
    // target name fields, the negotiate flags, the server challenge and the
    // reserved and target info fields.
    let mut type2 = Vec::new();
    type2.extend_from_slice(b"NTLMSSP\0");
    type2.extend_from_slice(&2u32.to_le_bytes());
    type2.extend_from_slice(&0u16.to_le_bytes());
    type2.extend_from_slice(&0u16.to_le_bytes());
    type2.extend_from_slice(&48u32.to_le_bytes());
    type2.extend_from_slice(&0x0800_0000u32.to_le_bytes());
    type2.extend_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    type2.extend_from_slice(&[0u8; 8]);
    type2.extend_from_slice(&0u16.to_le_bytes());
    type2.extend_from_slice(&0u16.to_le_bytes());
    type2.extend_from_slice(&48u32.to_le_bytes());
    let token = base64_simd::STANDARD.encode_to_string(&type2);

    let server = {
        let state = state.clone();
        server::http(move |req| {
            let state = state.clone();
            let token = token.clone();
            async move {
                state.record(&req);
                let leg = state
                    .connects
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut res = http::Response::default();
                match leg {
                    0 => {
                        *res.status_mut() = http::StatusCode::PROXY_AUTHENTICATION_REQUIRED;
                        res.headers_mut()
                            .insert("proxy-authenticate", "NTLM".parse().unwrap());
                    }
                    1 => {
                        *res.status_mut() = http::StatusCode::PROXY_AUTHENTICATION_REQUIRED;
                        res.headers_mut().insert(
                            "proxy-authenticate",
                            format!("Negotiate {token}").parse().unwrap(),
                        );
                    }
                    _ => {
                        *res.status_mut() = http::StatusCode::OK;
                    }
                }
                res.headers_mut()
                    .insert("content-length", "0".parse().unwrap());
                res
            }
        })
    };

    let proxy = format!("http://{}", server.addr());

    let err = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::https(&proxy).unwrap().auth(
            "Aladdin",
            "open sesame",
            hpx::ProxyAuthMethod::Ntlm,
        ))
        .build()
        .unwrap()
        .get("https://hyper.rs.local/prox")
        .send()
        .await
        .unwrap_err();

    assert!(
        err.is_connect() || err.is_request(),
        "unexpected error: {err:?}"
    );

    // The first CONNECT carries no credentials, the second one the NTLM
    // negotiate message and the third one the NTLMv2 response.
    let received = state
        .received
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    assert_eq!(received.len(), 2, "the NTLM handshake needs three legs");
    assert!(received[0].starts_with("NTLM "), "{}", received[0]);
    assert!(received[1].starts_with("Negotiate "), "{}", received[1]);
    assert_eq!(
        state.connects.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "the handshake needs three CONNECT requests"
    );
}

/// A `407` without matching credentials is reported as an error.
#[tokio::test]
async fn tunnel_reports_a_challenge_without_credentials() {
    let server = server::http(move |_req| async {
        let mut res = http::Response::default();
        *res.status_mut() = http::StatusCode::PROXY_AUTHENTICATION_REQUIRED;
        res.headers_mut().insert(
            "proxy-authenticate",
            "Basic realm=\"proxy\"".parse().unwrap(),
        );
        res.headers_mut()
            .insert("content-length", "0".parse().unwrap());
        res
    });

    let proxy = format!("http://{}", server.addr());

    let err = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::https(&proxy).unwrap())
        .build()
        .unwrap()
        .get("https://hyper.rs.local/prox")
        .send()
        .await
        .unwrap_err();

    let err = support::error::inspect(err).pop().unwrap();
    assert!(
        err.contains("auth"),
        "proxy auth error expected, got: {err:?}"
    );
}

/// A plain HTTP request through a proxy that challenges is retried with the
/// computed credentials.
#[tokio::test]
async fn plain_http_answers_a_digest_challenge() {
    let state = Arc::new(ChallengeProxy::new());

    let server = {
        let state = state.clone();
        server::http(move |req| {
            let state = state.clone();
            async move {
                state.record(&req);
                let leg = state
                    .connects
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut res = http::Response::default();
                if leg == 0 {
                    *res.status_mut() = http::StatusCode::PROXY_AUTHENTICATION_REQUIRED;
                    res.headers_mut().insert(
                        "proxy-authenticate",
                        "Digest realm=\"proxy\", nonce=\"abc123\", qop=\"auth\""
                            .parse()
                            .unwrap(),
                    );
                    res.headers_mut()
                        .insert("content-length", "0".parse().unwrap());
                } else {
                    *res.status_mut() = http::StatusCode::OK;
                }
                res
            }
        })
    };

    let proxy = format!("http://{}", server.addr());

    let res = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::http(&proxy).unwrap().auth(
            "Aladdin",
            "open sesame",
            hpx::ProxyAuthMethod::Digest,
        ))
        .build()
        .unwrap()
        .get("http://hyper.rs.local/prox")
        .send()
        .await
        .expect("the request should be retried with the credentials");

    assert_eq!(res.status(), hpx::StatusCode::OK);

    let received = state
        .received
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    assert_eq!(
        received.len(),
        1,
        "the challenge must be answered: {received:?}"
    );
    assert!(received[0].starts_with("Digest "), "{}", received[0]);
    assert!(
        received[0].contains("username=\"Aladdin\""),
        "{}",
        received[0]
    );
}

/// A plain HTTP request through a proxy that challenges is returned to the
/// caller when the body cannot be replayed.
#[tokio::test]
async fn plain_http_challenge_without_replayable_body_is_returned() {
    let server = server::http(move |_req| async {
        let mut res = http::Response::default();
        *res.status_mut() = http::StatusCode::PROXY_AUTHENTICATION_REQUIRED;
        res.headers_mut().insert(
            "proxy-authenticate",
            "Digest realm=\"proxy\", nonce=\"abc123\", qop=\"auth\""
                .parse()
                .unwrap(),
        );
        res.headers_mut()
            .insert("content-length", "0".parse().unwrap());
        res
    });

    let proxy = format!("http://{}", server.addr());

    let res = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::http(&proxy).unwrap().auth(
            "Aladdin",
            "open sesame",
            hpx::ProxyAuthMethod::Digest,
        ))
        .build()
        .unwrap()
        .get("http://hyper.rs.local/prox")
        .send()
        .await
        .expect("the challenge is returned to the caller");

    assert_eq!(res.status(), hpx::StatusCode::PROXY_AUTHENTICATION_REQUIRED);
}

/// A Proxy Auto-Config script selects the proxy for each request.
#[cfg(feature = "pac")]
#[tokio::test]
async fn pac_script_selects_the_proxy() {
    let url = "http://hyper.rs.local/prox";
    let server = server::http(move |req| {
        assert_eq!(req.uri(), url);
        async { http::Response::default() }
    });

    let proxy = format!("http://{}", server.addr());

    let script = format!(
        r#"
        function FindProxyForURL(url, host) {{
            if (dnsDomainIs(host, "hyper.rs.local")) {{
                return "PROXY {proxy}";
            }}
            return "DIRECT";
        }}
        "#
    );

    let res = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::pac(&script).unwrap())
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .expect("the PAC script should select the proxy");

    assert_eq!(res.status(), hpx::StatusCode::OK);
}

/// A PAC script that selects `DIRECT` sends the request without a proxy.
#[cfg(feature = "pac")]
#[tokio::test]
async fn pac_script_can_select_direct() {
    let server = server::http(move |_req| async { http::Response::default() });

    let origin = format!("http://{}", server.addr());

    let script = r#"
        function FindProxyForURL(url, host) {
            return "DIRECT";
        }
    "#;

    let res = Client::builder()
        .no_proxy()
        .proxy(hpx::Proxy::pac(script).unwrap())
        .build()
        .unwrap()
        .get(&origin)
        .send()
        .await
        .expect("the request should go directly to the origin");

    assert_eq!(res.status(), hpx::StatusCode::OK);
}

/// A response whose header block exceeds the configured limit is rejected.
#[tokio::test]
async fn response_header_size_limit_is_enforced() {
    let server = server::http(move |_req| async {
        let mut res = http::Response::default();
        // A single very large header value.
        res.headers_mut().insert(
            "x-large",
            http::HeaderValue::from_str(&"a".repeat(4096)).unwrap(),
        );
        res
    });

    let res = Client::builder()
        .no_proxy()
        .max_response_header_size(Some(1024))
        .build()
        .unwrap()
        .get(format!("http://{}", server.addr()))
        .send()
        .await;

    assert!(
        res.is_err(),
        "a response with an oversized header block must be rejected"
    );
}

/// The default limit still allows a normal response.
#[tokio::test]
async fn response_header_size_limit_allows_normal_responses() {
    let server = server::http(move |_req| async { http::Response::default() });

    let res = Client::builder()
        .no_proxy()
        .max_response_header_size(Some(64 * 1024))
        .build()
        .unwrap()
        .get(format!("http://{}", server.addr()))
        .send()
        .await
        .expect("a normal response must be accepted");

    assert_eq!(res.status(), hpx::StatusCode::OK);
}
