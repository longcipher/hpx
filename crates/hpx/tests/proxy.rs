mod support;
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
use std::sync::Arc;
#[cfg(feature = "boring")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "rustls-tls")]
use std::{convert::Infallible, io::Cursor};
use std::{env, sync::LazyLock};

#[cfg(feature = "boring")]
use boring::ssl::{SslAcceptor, SslFiletype, SslMethod, SslVerifyMode};
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
use bytes::Bytes;
use hpx::Client;
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
use hpx::tls::{CertStore, Identity};
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
use http_body_util::Full;
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
use hyper::{Response, server::conn::http1, service::service_fn};
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
use hyper_util::rt::TokioIo;
use support::server;
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
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
#[ignore]
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
        )
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
#[ignore]
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

#[cfg(any(feature = "boring", feature = "rustls-tls"))]
const CA_CERT_PEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/mtls/ca.crt"
));
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
const CLIENT_CERT_PEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/mtls/client.crt"
));
#[cfg(any(feature = "boring", feature = "rustls-tls"))]
const CLIENT_KEY_PEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/mtls/client.key"
));
#[cfg(feature = "boring")]
const SERVER_CERT_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/server.crt");
#[cfg(feature = "boring")]
const SERVER_KEY_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/server.key");
#[cfg(feature = "boring")]
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

#[cfg(feature = "boring")]
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

#[cfg(feature = "boring")]
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

#[cfg(feature = "boring")]
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

#[cfg(feature = "boring")]
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

#[cfg(feature = "boring")]
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
