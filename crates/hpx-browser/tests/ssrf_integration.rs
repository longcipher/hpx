//! Integration tests for SSRF protection at the HttpClient boundary.
//!
//! Spins up a local HTTP server on 127.0.0.1 and verifies that:
//! - Default HttpClient rejects loopback requests (SSRF)
//! - HttpClient with `allow_private_network(true)` connects successfully

use axum::{Router, routing::get};
use hpx_browser::net::{HttpClient, RedirectPolicy};
use tokio::net::TcpListener;

/// Start a minimal HTTP server on a random loopback port, return its URL.
async fn start_test_server() -> String {
    let app = Router::new().route("/", get(|| async { "ok" }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn ssrf_blocks_loopback_by_default() {
    let server_url = start_test_server().await;
    let client = HttpClient::new(hpx::BrowserProfile::Chrome).unwrap();

    let result = client
        .request("GET", &server_url, None, &[], RedirectPolicy::Manual)
        .await;

    assert!(result.is_err(), "SSRF should block loopback by default");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("SSRF"),
        "error should be SSRF rejection, got: {err}"
    );
}

#[tokio::test]
async fn ssrf_allows_loopback_when_permitted() {
    let server_url = start_test_server().await;
    let client = HttpClient::new(hpx::BrowserProfile::Chrome)
        .unwrap()
        .allow_private_network(true);

    let result = client
        .request("GET", &server_url, None, &[], RedirectPolicy::Manual)
        .await;

    assert!(
        result.is_ok(),
        "should connect to loopback when allow_private_network is true: {:?}",
        result.err()
    );
    let resp = result.unwrap();
    assert!(resp.ok(), "server should return success status");
}
