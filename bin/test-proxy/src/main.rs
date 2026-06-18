use eyre::{Result, bail, eyre};
use futures::{SinkExt, StreamExt};
use hpx_yawc::{Frame, WebSocket, close::CloseCode, proxy::ProxyConfig};
use url::Url;

const PROXY_URL: &str = "http://127.0.0.1:7890";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    println!("=== HTTP Proxy Tests (proxy={PROXY_URL}) ===\n");

    test_http_proxy().await?;
    println!("[PASS] HTTP proxy (plain HTTP)\n");

    test_https_proxy().await?;
    println!("[PASS] HTTPS proxy (CONNECT tunnel)\n");

    println!("=== WebSocket Proxy Tests (proxy={PROXY_URL}) ===\n");

    test_ws_proxy().await?;
    println!("[PASS] WebSocket proxy (ws://echo)\n");

    test_wss_proxy().await?;
    println!("[PASS] WebSocket proxy (wss://echo)\n");

    println!("=== All proxy tests passed ===");
    Ok(())
}

async fn test_http_proxy() -> Result<()> {
    let proxy = hpx::Proxy::http(PROXY_URL)?;
    let client = hpx::Client::builder().proxy(proxy).build()?;

    let resp = client
        .get("http://httpbin.org/ip")
        .send()
        .await
        .map_err(|e| eyre!("HTTP proxy request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| eyre!("failed to read response body: {e}"))?;

    if !status.is_success() {
        bail!("HTTP proxy returned status {status}: {body}");
    }

    println!("  status={status}, body={}", truncate(&body, 200));
    Ok(())
}

async fn test_https_proxy() -> Result<()> {
    let proxy = hpx::Proxy::all(PROXY_URL)?;
    let client = hpx::Client::builder()
        .proxy(proxy)
        .cert_verification(false)
        .build()?;

    let resp = client
        .get("https://httpbin.org/ip")
        .send()
        .await
        .map_err(|e| eyre!("HTTPS proxy request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| eyre!("failed to read response body: {e}"))?;

    if !status.is_success() {
        bail!("HTTPS proxy returned status {status}: {body}");
    }

    println!("  status={status}, body={}", truncate(&body, 200));
    Ok(())
}

async fn test_ws_proxy() -> Result<()> {
    let proxy_url: Url = PROXY_URL.parse()?;
    let proxy = ProxyConfig::Http(proxy_url);

    let mut ws = WebSocket::connect("wss://ws.postman-echo.com/raw".parse()?)
        .with_proxy(proxy)
        .await
        .map_err(|e| eyre!("WS proxy connect failed: {e}"))?;

    let msg = "hello via proxy";
    ws.send(Frame::text(msg)).await?;

    let frame = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
        .await
        .map_err(|_| eyre!("WS echo timeout"))?
        .ok_or_else(|| eyre!("WS stream ended before echo"))?;

    let reply = String::from_utf8_lossy(frame.payload());
    if reply != msg {
        bail!("WS echo mismatch: expected {msg:?}, got {reply:?}");
    }
    println!("  echoed: {reply}");

    ws.send(Frame::close(CloseCode::Normal, b"done")).await?;
    Ok(())
}

async fn test_wss_proxy() -> Result<()> {
    let proxy_url: Url = PROXY_URL.parse()?;
    let proxy = ProxyConfig::Http(proxy_url);

    let mut ws = WebSocket::connect("wss://ws.postman-echo.com/raw".parse()?)
        .with_proxy(proxy)
        .await
        .map_err(|e| eyre!("WSS proxy connect failed: {e}"))?;

    let msg = "hello via wss proxy";
    ws.send(Frame::text(msg)).await?;

    let frame = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
        .await
        .map_err(|_| eyre!("WSS echo timeout"))?
        .ok_or_else(|| eyre!("WSS stream ended before echo"))?;

    let reply = String::from_utf8_lossy(frame.payload());
    if reply != msg {
        bail!("WSS echo mismatch: expected {msg:?}, got {reply:?}");
    }
    println!("  echoed: {reply}");

    ws.send(Frame::close(CloseCode::Normal, b"done")).await?;
    Ok(())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() > max { &s[..max] } else { s }
}
