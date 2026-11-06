#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration tests exercise error paths with unwrap/panic"
)]

//! Integration tests for HTTP CLI commands using a mock axum server.

#![allow(
    clippy::allow_attributes,
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collapsible_if,
    clippy::collection_is_never_read,
    clippy::duration_suboptimal_units,
    clippy::expect_fun_call,
    clippy::expect_used,
    clippy::explicit_iter_loop,
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
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::redundant_async_block,
    clippy::redundant_clone,
    clippy::redundant_pattern_matching,
    clippy::struct_field_names,
    clippy::type_complexity,
    clippy::unchecked_time_subtraction,
    clippy::unnecessary_cast,
    clippy::unnecessary_trailing_comma,
    clippy::unnested_or_patterns,
    clippy::unreadable_literal,
    clippy::use_self,
    clippy::useless_borrows_in_formatting,
    clippy::useless_vec,
    missing_docs,
    unfulfilled_lint_expectations
)]
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
    // Bind and immediately release so the port is almost certainly closed.
    // Re-check with a direct connect so a parallel test that races for the
    // same ephemeral port cannot make this assertion flaky.
    let port = loop {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Err(_) => break port,
            Ok(_) => continue, // stolen by another test; retry
        }
    };

    // Use std::process (not tokio) so env_clear cannot race with concurrent
    // `env::set_var` in other tests. A system/http proxy would answer 502 and
    // exit 0, which would mask the connection-refused signal.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .arg(format!("http://127.0.0.1:{port}/"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success(),
        "expected connection error for closed port {port}, but command succeeded:\nstdout={stdout}\nstderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("Connection refused")
            || combined.contains("error sending request")
            || combined.contains("Connect"),
        "expected a connect error, got:\nstdout={stdout}\nstderr={stderr}"
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

const DL_FILE_LEN: usize = 1000;

fn dl_payload() -> Vec<u8> {
    (0..DL_FILE_LEN).map(|i| (i % 251) as u8).collect()
}

/// Serve `/file.bin` like a static file server: HEAD reports the length and
/// range support, GET honors `Range` with 206.
async fn start_dl_server() -> String {
    use axum::{
        http::{HeaderValue, StatusCode, header},
        response::{IntoResponse, Response},
    };

    async fn head_file() -> Response {
        let mut resp = StatusCode::OK.into_response();
        let headers = resp.headers_mut();
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(DL_FILE_LEN));
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        resp
    }

    async fn get_file(headers: HeaderMap) -> Response {
        let data = dl_payload();
        let range = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes="))
            .and_then(|v| v.split_once('-'))
            .and_then(|(start, end)| {
                Some((start.parse::<usize>().ok()?, end.parse::<usize>().ok()?))
            });
        match range {
            Some((start, end)) => {
                let mut resp =
                    (StatusCode::PARTIAL_CONTENT, data[start..=end].to_vec()).into_response();
                resp.headers_mut().insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes {start}-{end}/{DL_FILE_LEN}")).unwrap(),
                );
                resp
            }
            None => (StatusCode::OK, data).into_response(),
        }
    }

    let app = Router::new().route("/file.bin", get(get_file).head(head_file));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dl_add_exits_after_download_completes() {
    let base = start_dl_server().await;
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = temp_dir.path().join("downloads.db");
    let output_path = temp_dir.path().join("file.bin");

    // A successful download emits `Completed`; `dl add` must stop waiting on
    // it instead of blocking forever.
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
            .args([
                "--storage-path",
                storage.to_str().unwrap(),
                "dl",
                "add",
                &format!("{base}/file.bin"),
                "-o",
                output_path.to_str().unwrap(),
            ])
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("hpx dl add did not exit after the download completed")
    .unwrap();

    assert!(
        output.status.success(),
        "dl add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(&output_path).unwrap(), dl_payload());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_redacts_proxy_password() {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args([
            "--dry-run",
            "--proxy",
            "http://user:s3cret@127.0.0.1:9",
            "http://127.0.0.1:9/",
        ])
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "dry run failed: {stderr}");
    assert!(
        stderr.contains("[proxy] http://user:***@127.0.0.1:9"),
        "expected redacted proxy, got: {stderr}"
    );
    assert!(
        !stdout.contains("s3cret") && !stderr.contains("s3cret"),
        "proxy password leaked:\nstdout={stdout}\nstderr={stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_proxy_error_redacts_password() {
    // Scheme-less, so hpx rejects it with an error that embeds the URI.
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hpx"))
        .args(["--proxy", "user:s3cret@proxy:8080", "http://127.0.0.1:9/"])
        .env("RUST_LOG", "error")
        .output()
        .await
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "expected an invalid proxy error");
    assert!(
        combined.contains("invalid proxy URL: user:***@proxy:8080"),
        "expected redacted proxy error, got: {combined}"
    );
    assert!(
        !combined.contains("s3cret"),
        "proxy password leaked: {combined}"
    );
}
