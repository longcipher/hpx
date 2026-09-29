//! Integration tests for browser CLI subcommands: `fetch`, `scrape`, `serve`.
//!
//! Uses a local axum fixture server. Requires `--allow-private-network` because
//! the browser engine treats loopback as a private network.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration tests exercise CLI error paths with unwrap/panic"
)]

use axum::{Router, routing::get};
use tokio::net::TcpListener;

async fn start_server() -> String {
    let app = Router::new()
        .route(
            "/",
            get(|| async {
                axum::response::Html(
                    r#"<!doctype html><html><head><title>T</title></head>
                       <body><h1>Hello Fixture</h1><a href="/next">next</a></body></html>"#,
                )
            }),
        )
        .route(
            "/next",
            get(|| async { axum::response::Html("<html><body>next page</body></html>") }),
        )
        .route("/plain.txt", get(|| async { "plain text body" }));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn cmd() -> tokio::process::Command {
    let mut c = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"));
    // Keep the child env clean of ambient proxies so local fixtures are reachable.
    c.env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY");
    c
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_dump_original_returns_body() {
    let base = start_server().await;
    let output = cmd()
        .args([
            "fetch",
            &format!("{base}/plain.txt"),
            "--dump",
            "original",
            "--allow-private-network",
            "--quiet",
            "--wait",
            "0",
            "--timeout",
            "15",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "fetch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "plain text body");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_dump_html_contains_markup() {
    let base = start_server().await;
    let output = cmd()
        .args([
            "fetch",
            &format!("{base}/"),
            "--dump",
            "html",
            "--allow-private-network",
            "--quiet",
            "--wait",
            "0",
            "--timeout",
            "20",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "fetch html failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Hello Fixture") || stdout.contains("<html"),
        "unexpected html dump: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_dump_text_contains_visible_words() {
    let base = start_server().await;
    let output = cmd()
        .args([
            "fetch",
            &format!("{base}/"),
            "--dump",
            "text",
            "--allow-private-network",
            "--quiet",
            "--wait",
            "0",
            "--timeout",
            "20",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "fetch text failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Hello Fixture"),
        "text dump missing content: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_writes_output_file() {
    let base = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("page.html");
    let output = cmd()
        .args([
            "fetch",
            &format!("{base}/"),
            "--dump",
            "html",
            "--allow-private-network",
            "--quiet",
            "--wait",
            "0",
            "--timeout",
            "20",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "fetch -o failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(written.contains("Hello Fixture") || written.contains("<html"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scrape_multiple_urls_json() {
    let base = start_server().await;
    let output = cmd()
        .args([
            "scrape",
            &format!("{base}/"),
            &format!("{base}/next"),
            "--format",
            "json",
            "--allow-private-network",
            "--quiet",
            "--concurrency",
            "2",
            "--timeout",
            "30",
        ])
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() || stdout.contains('{') || stdout.contains('['),
        "scrape failed status={:?} stdout={stdout} stderr={stderr}",
        output.status
    );
    assert!(
        stdout.contains(base.as_str()) || stdout.contains("url") || stdout.contains('['),
        "scrape produced no JSON-ish output: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_starts_cdp_endpoint() {
    // Pick a free port.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut child = cmd()
        .args([
            "serve",
            "--port",
            &port.to_string(),
            "--quiet",
            "--workers",
            "1",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    // Wait until a TCP connect succeeds (server is listening).
    let mut up = false;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            up = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(up, "CDP server did not start on port {port}");

    child.kill().await.ok();
    let _ = child.wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_rejects_private_network_without_flag() {
    let base = start_server().await;
    let output = cmd()
        .args([
            "fetch",
            &format!("{base}/plain.txt"),
            "--dump",
            "original",
            "--quiet",
            "--wait",
            "0",
            "--timeout",
            "10",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        !output.status.success(),
        "expected SSRF rejection without --allow-private-network"
    );
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        err.contains("SSRF") || err.contains("forbidden") || err.contains("private"),
        "expected SSRF error, got: {err}"
    );
}
