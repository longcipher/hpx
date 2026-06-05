#![allow(clippy::unwrap_used, clippy::panic)]

//! Integration tests for HTTP CLI commands using a mock axum server.

use axum::{
    Router,
    http::HeaderMap,
    routing::{get, post},
};
use tokio::net::TcpListener;

async fn start_server() -> String {
    let app = Router::new()
        .route("/hello", get(|| async { "hello world" }))
        .route("/echo", post(|body: String| async move { body }))
        .route(
            "/headers",
            get(|headers: HeaderMap| async move {
                let mut result = String::new();
                for (k, v) in &headers {
                    result.push_str(&format!("{}: {}\n", k, v.to_str().unwrap_or("")));
                }
                result
            }),
        )
        .route(
            "/auth/basic",
            get(|headers: HeaderMap| async move {
                match headers.get("authorization") {
                    Some(val) => val.to_str().unwrap_or("bad").to_string(),
                    None => "no auth".to_string(),
                }
            }),
        )
        .route(
            "/auth/bearer",
            get(|headers: HeaderMap| async move {
                match headers.get("authorization") {
                    Some(val) => val.to_str().unwrap_or("bad").to_string(),
                    None => "no auth".to_string(),
                }
            }),
        )
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                "slow"
            }),
        )
        .route("/file.txt", get(|| async { "file content here" }));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_get_request() {
    let base = start_server().await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args([format!("{base}/hello").as_str()])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello world");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_post_request() {
    let base = start_server().await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args(["-X", "POST", "-d", "test body", &format!("{base}/echo")])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "test body");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_custom_headers() {
    let base = start_server().await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args(["-H", "X-Custom:test-value", &format!("{base}/headers")])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("x-custom: test-value"),
        "expected custom header in response, got: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_basic_auth() {
    let base = start_server().await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args(["--basic", "user:pass", &format!("{base}/auth/basic")])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Basic "),
        "expected Basic auth header, got: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bearer_auth() {
    let base = start_server().await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args(["--bearer", "my-token", &format!("{base}/auth/bearer")])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Bearer my-token"),
        "expected Bearer auth header, got: {stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_invalid_url() {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args(["not-a-valid-url"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_timeout() {
    let base = start_server().await;
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
            .args(["--timeout", "1", &format!("{base}/slow")])
            .output()
            .await
    })
    .await;
    assert!(result.is_ok(), "test timed out waiting for CLI");
    let output = result.unwrap().unwrap();
    assert!(
        !output.status.success(),
        "expected timeout error, but command succeeded"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_connection_refused() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args([&format!("http://127.0.0.1:{port}/")])
        .output()
        .await
        .unwrap();
    assert!(
        !output.status.success(),
        "expected connection error, but command succeeded"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_download_to_file() {
    let base = start_server().await;
    let temp_dir = tempfile::tempdir().unwrap();
    let output_path = temp_dir.path().join("downloaded.txt");

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args([
            "-o",
            output_path.to_str().unwrap(),
            &format!("{base}/file.txt"),
        ])
        .output()
        .await
        .unwrap();

    assert!(
        output.status.success(),
        "download failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_path.exists(), "downloaded file not created");
    let content = std::fs::read_to_string(&output_path).unwrap();
    assert_eq!(content, "file content here");
}
