//! HTTP proxy support for WebSocket connections.
//!
//! This module provides support for connecting to WebSocket servers through
//! HTTP CONNECT proxies and SOCKS5 proxies.

use std::io;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use url::Url;

/// Configuration for connecting through a proxy.
#[derive(Debug, Clone)]
pub enum ProxyConfig {
    /// HTTP CONNECT tunnel proxy.
    Http(Url),
    /// SOCKS5 proxy (requires the `socks` feature).
    #[cfg(feature = "socks")]
    Socks5(Url),
}

/// Establishes a TCP connection to the target through the given proxy.
///
/// For HTTP proxies, performs an HTTP CONNECT tunnel.
/// For SOCKS5 proxies, uses the tokio-socks library.
pub async fn connect_through_proxy(
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> io::Result<TcpStream> {
    match proxy {
        ProxyConfig::Http(proxy_url) => {
            let proxy_addr = format!(
                "{}:{}",
                proxy_url.host_str().unwrap_or("127.0.0.1"),
                proxy_url.port().unwrap_or(8080)
            );
            let proxy_stream = TcpStream::connect(&proxy_addr).await?;
            http_connect_tunnel(proxy_stream, target_host, target_port).await
        }
        #[cfg(feature = "socks")]
        ProxyConfig::Socks5(proxy_url) => {
            let proxy_addr = format!(
                "{}:{}",
                proxy_url.host_str().unwrap_or("127.0.0.1"),
                proxy_url.port().unwrap_or(1080)
            );
            let target = format!("{target_host}:{target_port}");
            let stream = tokio_socks::tcp::Socks5Stream::connect(&*proxy_addr, target)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;
            Ok(stream.into_inner())
        }
    }
}

/// Performs an HTTP CONNECT tunnel through a proxy.
///
/// Sends a CONNECT request to the proxy and reads the response. If the proxy
/// responds with a 2xx status code, the connection is established and the
/// proxy stream is returned.
async fn http_connect_tunnel(
    mut proxy_stream: TcpStream,
    target_host: &str,
    target_port: u16,
) -> io::Result<TcpStream> {
    let authority = if target_host.contains(':') {
        format!("[{target_host}]:{target_port}")
    } else {
        format!("{target_host}:{target_port}")
    };

    let connect_req = format!(
        "CONNECT {authority} HTTP/1.1\r\n\
         Host: {authority}\r\n\
         \r\n"
    );

    proxy_stream.write_all(connect_req.as_bytes()).await?;

    let mut buf = Vec::with_capacity(1024);
    let mut temp = [0u8; 1024];

    loop {
        let n = proxy_stream.read(&mut temp).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before receiving CONNECT response",
            ));
        }
        buf.extend_from_slice(&temp[..n]);

        if let Some(end_pos) = find_header_end(&buf) {
            let status_line = parse_status_line(&buf[..end_pos])?;

            if !status_line.starts_with("HTTP/") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid HTTP response: {status_line}"),
                ));
            }

            let status_code = parse_status_code(&status_line)?;
            if !(200..300).contains(&status_code) {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("proxy tunnel failed with status {status_code}"),
                ));
            }

            return Ok(proxy_stream);
        }

        if buf.len() > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CONNECT response too long",
            ));
        }
    }
}

/// Find the end of HTTP headers (`\r\n\r\n`) in a buffer.
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

/// Parse the status line from an HTTP response (e.g., "HTTP/1.1 200 OK").
fn parse_status_line(headers: &[u8]) -> io::Result<String> {
    let line_end = headers
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(headers.len());

    let line = &headers[..line_end];
    String::from_utf8(line.to_vec()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Parse the status code from an HTTP status line.
fn parse_status_code(status_line: &str) -> io::Result<u16> {
    let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid status line: {status_line}"),
        ));
    }

    parts[1]
        .parse::<u16>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_http_connect_receives_complete_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf).await;
            let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
            stream.write_all(response).await.unwrap();
        });

        let proxy_stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(proxy_stream, "example.com", 443).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_http_connect_rejects_non_2xx() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf).await;
            let response = b"HTTP/1.1 403 Forbidden\r\n\r\n";
            stream.write_all(response).await.unwrap();
        });

        let proxy_stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(proxy_stream, "example.com", 443).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_http_connect_chunked_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf).await;
            // Send response in two chunks
            stream.write_all(b"HTTP/1.1 200 ").await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            stream
                .write_all(b"Connection Established\r\n\r\n")
                .await
                .unwrap();
        });

        let proxy_stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(proxy_stream, "example.com", 443).await;
        assert!(result.is_ok());
    }
}
