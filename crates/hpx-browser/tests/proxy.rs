//! Proxy integration tests.
//!
//! Every network path a page has must honor the configured proxy:
//! main-document navigation, subresource loads, and JS `fetch()`.
//! HTTP forward proxies and SOCKS5 proxies are covered with in-process
//! recording servers — no external network dependency.

#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collection_is_never_read,
    clippy::collapsible_if,
    clippy::duration_suboptimal_units,
    clippy::explicit_iter_loop,
    clippy::expect_fun_call,
    clippy::expect_used,
    clippy::ignore_without_reason,
    clippy::ignored_unit_patterns,
    clippy::items_after_test_module,
    clippy::iter_kv_map,
    clippy::iter_with_drain,
    clippy::let_underscore_future,
    clippy::manual_div_ceil,
    clippy::manual_string_new,
    clippy::map_unwrap_or,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_continue,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_raw_string_hashes,
    clippy::nonminimal_bool,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::redundant_async_block,
    clippy::redundant_clone,
    clippy::redundant_pattern_matching,
    clippy::struct_field_names,
    clippy::type_complexity,
    clippy::unchecked_time_subtraction,
    clippy::unreadable_literal,
    clippy::unnecessary_cast,
    clippy::unnecessary_trailing_comma,
    clippy::unnested_or_patterns,
    clippy::useless_borrows_in_formatting,
    clippy::useless_vec,
    clippy::use_self,
    clippy::unwrap_used,
    unfulfilled_lint_expectations
)]

use std::{
    net::{Ipv4Addr, Ipv6Addr},
    sync::Arc,
    time::Duration,
};

use hpx_browser::{
    dom::Dom,
    js_runtime::BrowserJsRuntime,
    net::{NetConfig, SharedNetConfig},
    page::Page,
    stealth::chrome_148_windows,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

const BODY: &str =
    "<html><head><title>proxied</title></head><body><p>hello via proxy</p></body></html>";

fn http_200() -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        BODY.len(),
        BODY
    )
}

/// Recording HTTP forward proxy.
///
/// Logs the raw request head of every connection and answers with a fixed
/// 200 itself (it never opens an upstream connection).
async fn spawn_http_proxy() -> (String, mpsc::UnboundedReceiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
    let addr = listener.local_addr().expect("proxy addr");
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = stream.write_all(http_200().as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    });

    (format!("http://{addr}/"), rx)
}

/// Minimal recording SOCKS5 proxy (no-auth, CONNECT only).
///
/// Performs the greeting + CONNECT handshake, records the requested
/// (host, port), and then relays a fixed HTTP 200 over the tunnel — it never
/// opens an upstream connection.
async fn spawn_socks5_proxy() -> (String, mpsc::UnboundedReceiver<(String, u16)>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind socks proxy");
    let addr = listener.local_addr().expect("socks addr");
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                // Greeting: VER NMETHODS METHODS...
                let mut hdr = [0u8; 2];
                if stream.read_exact(&mut hdr).await.is_err() || hdr[0] != 0x05 {
                    return;
                }
                let mut methods = vec![0u8; hdr[1] as usize];
                if stream.read_exact(&mut methods).await.is_err() {
                    return;
                }
                // Select "no authentication required".
                if stream.write_all(&[0x05, 0x00]).await.is_err() {
                    return;
                }

                // Request: VER CMD RSV ATYP DST.ADDR DST.PORT
                let mut req = [0u8; 4];
                if stream.read_exact(&mut req).await.is_err() {
                    return;
                }
                let host = match req[3] {
                    1 => {
                        let mut b = [0u8; 4];
                        if stream.read_exact(&mut b).await.is_err() {
                            return;
                        }
                        Ipv4Addr::from(b).to_string()
                    }
                    3 => {
                        let mut len = [0u8; 1];
                        if stream.read_exact(&mut len).await.is_err() {
                            return;
                        }
                        let mut domain = vec![0u8; len[0] as usize];
                        if stream.read_exact(&mut domain).await.is_err() {
                            return;
                        }
                        String::from_utf8_lossy(&domain).into_owned()
                    }
                    4 => {
                        let mut b = [0u8; 16];
                        if stream.read_exact(&mut b).await.is_err() {
                            return;
                        }
                        Ipv6Addr::from(b).to_string()
                    }
                    _ => return,
                };
                let mut port_bytes = [0u8; 2];
                if stream.read_exact(&mut port_bytes).await.is_err() {
                    return;
                }
                let port = u16::from_be_bytes(port_bytes);
                let _ = tx.send((host, port));

                // Success reply, bound address 0.0.0.0:0.
                if stream
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await
                    .is_err()
                {
                    return;
                }

                // The client now speaks HTTP through the tunnel.
                let mut buf = [0u8; 16 * 1024];
                if let Ok(n) = stream.read(&mut buf).await
                    && n > 0
                {
                    let _ = stream.write_all(http_200().as_bytes()).await;
                }
                let _ = stream.flush().await;
            });
        }
    });

    (format!("socks5://{addr}"), rx)
}

async fn next_recorded(rx: &mut mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("proxy should receive a request")
        .expect("proxy channel open")
}

fn localhost_page() -> Page {
    let mut page = Page::new();
    page.set_allow_private_network(true);
    page
}

#[tokio::test]
async fn page_navigation_routes_through_http_proxy() {
    let (proxy_url, mut rec) = spawn_http_proxy().await;
    let mut page = localhost_page();
    page.set_proxy(Some(hpx::Proxy::all(&proxy_url).expect("valid proxy url")));

    page.navigate("http://localhost/proxied")
        .await
        .expect("navigate");

    let req = next_recorded(&mut rec).await;
    assert!(
        req.starts_with("GET http://localhost/proxied "),
        "proxy must receive absolute-form request line, got: {req}"
    );
    assert!(
        page.content().contains("hello via proxy"),
        "page content must come from the proxied response"
    );
}

#[tokio::test]
async fn page_subresources_route_through_http_proxy() {
    let (proxy_url, mut rec) = spawn_http_proxy().await;
    let mut page = localhost_page();
    page.set_proxy(Some(hpx::Proxy::all(&proxy_url).expect("valid proxy url")));

    // The proxy answers every request with the same HTML document, so any
    // subresource request is recorded too.
    page.navigate("http://localhost/page-with-assets")
        .await
        .expect("navigate");

    let first = next_recorded(&mut rec).await;
    assert!(
        first.starts_with("GET http://localhost/page-with-assets "),
        "{first}"
    );
}

#[tokio::test]
#[cfg(feature = "socks")]
async fn page_navigation_routes_through_socks5_proxy() {
    let (proxy_url, mut rec) = spawn_socks5_proxy().await;
    let mut page = localhost_page();
    page.set_proxy(Some(hpx::Proxy::all(&proxy_url).expect("valid socks url")));

    page.navigate("http://localhost/proxied")
        .await
        .expect("navigate");

    let (host, port) = tokio::time::timeout(Duration::from_secs(10), rec.recv())
        .await
        .expect("socks proxy should receive a connect")
        .expect("socks channel open");
    assert!(
        host == "127.0.0.1" || host == "::1",
        "socks5 resolves locally and connects to the loopback address, got: {host}"
    );
    assert_eq!(port, 80);
    assert!(page.content().contains("hello via proxy"));
}

#[tokio::test]
#[cfg(feature = "v8")]
async fn js_fetch_routes_through_http_proxy() {
    let (proxy_url, mut rec) = spawn_http_proxy().await;
    let net_config: SharedNetConfig = NetConfig::shared();
    net_config.store(Arc::new(NetConfig {
        proxy: Some(hpx::Proxy::all(&proxy_url).expect("valid proxy url")),
        allow_private_network: true,
    }));

    let base = url::Url::parse("http://localhost/").expect("base url");
    let mut rt = BrowserJsRuntime::with_net_config(Dom::new(), Some(base), net_config)
        .expect("runtime with net config");

    rt.execute_script(
        "globalThis.__text = 'pending'; \
         fetch('/data').then(r => r.text()).then(t => { globalThis.__text = t; }) \
         .catch(e => { globalThis.__text = 'ERR:' + e; });",
    )
    .expect("bootstrap fetch script");
    rt.run_event_loop().await.expect("event loop");

    let text = rt.execute_script("globalThis.__text").expect("read result");
    assert_eq!(text, BODY, "JS fetch must return the proxied body");

    let req = next_recorded(&mut rec).await;
    assert!(
        req.starts_with("GET http://localhost/data "),
        "JS fetch must go through the proxy with an absolute-form URI, got: {req}"
    );
}

#[tokio::test]
async fn stealth_profile_proxy_is_applied_to_navigation() {
    let (proxy_url, mut rec) = spawn_http_proxy().await;
    let mut profile = chrome_148_windows();
    profile.proxy = Some(proxy_url);

    let mut page = Page::with_profile(BODY, "http://localhost/", profile)
        .await
        .expect("page with profile");
    page.set_allow_private_network(true);

    page.navigate("http://localhost/proxied")
        .await
        .expect("navigate");

    let req = next_recorded(&mut rec).await;
    assert!(
        req.starts_with("GET http://localhost/proxied "),
        "profile proxy must be applied, got: {req}"
    );
}

#[tokio::test]
async fn explicit_proxy_wins_over_stealth_profile_proxy() {
    let (explicit_url, mut explicit_rec) = spawn_http_proxy().await;
    let (profile_url, mut profile_rec) = spawn_http_proxy().await;
    let mut profile = chrome_148_windows();
    profile.proxy = Some(profile_url);

    let mut page = Page::with_profile(BODY, "http://localhost/", profile)
        .await
        .expect("page with profile");
    page.set_allow_private_network(true);
    page.set_proxy(Some(
        hpx::Proxy::all(&explicit_url).expect("valid proxy url"),
    ));

    page.navigate("http://localhost/proxied")
        .await
        .expect("navigate");

    let req = next_recorded(&mut explicit_rec).await;
    assert!(req.starts_with("GET http://localhost/proxied "), "{req}");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), profile_rec.recv())
            .await
            .is_err(),
        "the profile proxy must not receive traffic once an explicit proxy is set"
    );
}

#[tokio::test]
async fn clearing_proxy_restores_direct_connection() {
    let (proxy_url, mut proxy_rec) = spawn_http_proxy().await;

    // Direct origin server.
    let origin = TcpListener::bind("127.0.0.1:0").await.expect("bind origin");
    let origin_addr = origin.local_addr().expect("origin addr");
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = origin.accept().await {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let _ = stream.write_all(http_200().as_bytes()).await;
            let _ = stream.flush().await;
        }
    });

    let mut page = localhost_page();
    page.set_proxy(Some(hpx::Proxy::all(&proxy_url).expect("valid proxy url")));
    page.set_proxy(None);

    page.navigate(&format!("http://{origin_addr}/direct"))
        .await
        .expect("navigate");

    assert!(
        tokio::time::timeout(Duration::from_millis(500), proxy_rec.recv())
            .await
            .is_err(),
        "no traffic may reach the proxy after it was cleared"
    );
    assert!(page.content().contains("hello via proxy"));
}
