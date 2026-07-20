#![cfg(feature = "http3")]

use hpx::Client;
use http::Version;

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Real-network HTTP/3 integration test against Cloudflare's trace endpoint.
///
/// Sends a GET request to `https://cloudflare.com/cdn-cgi/trace` using
/// `Client::builder().http3_only().build()` and asserts the response is
/// delivered over HTTP/3 with status 200 OK and the expected trace body.
///
/// This test is marked `#[ignore]` because it requires real network access.
/// Run manually with:
///
/// ```text
/// cargo test -p hpx --features http3 --test http3_integration \
///   http3_get_cloudflare_trace -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore]
async fn http3_get_cloudflare_trace() -> TestResult<()> {
    let client = Client::builder().http3_only().build()?;

    let response = client
        .get("https://cloudflare.com/cdn-cgi/trace")
        .send()
        .await?;

    assert_eq!(
        response.status(),
        http::StatusCode::OK,
        "expected 200 OK from Cloudflare trace endpoint"
    );

    assert_eq!(
        response.version(),
        Version::HTTP_3,
        "expected HTTP/3 version from Cloudflare trace endpoint"
    );

    let body = response.text().await?;
    assert!(
        body.contains("h="),
        "expected trace output to contain 'h=' (host header line), got: {body}"
    );

    Ok(())
}
