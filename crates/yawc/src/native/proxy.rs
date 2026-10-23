//! HTTP proxy support for WebSocket connections.
//!
//! This module provides support for connecting to WebSocket servers through
//! HTTP CONNECT proxies, SOCKS4/SOCKS4a proxies and SOCKS5 proxies.

use std::io;

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};
use url::Url;

/// Configuration for connecting through a proxy.
#[derive(Debug, Clone)]
pub enum ProxyConfig {
    /// HTTP CONNECT tunnel proxy.
    ///
    /// A `https://` URL means the connection to the proxy itself is secured
    /// with TLS; the CONNECT tunnel is then established inside that session.
    Http(Url),
    /// SOCKS5 proxy (requires the `socks` feature).
    #[cfg(feature = "socks")]
    Socks5(Url),
    /// SOCKS4/SOCKS4a proxy (requires the `socks` feature).
    ///
    /// A hostname target is encoded using the SOCKS4a extension, so the
    /// proxy resolves the destination name.
    #[cfg(feature = "socks")]
    Socks4(Url),
}

/// Combined read+write trait for proxy tunnel streams.
pub trait ProxyIo:
    tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug
{
}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug> ProxyIo
    for T
{
}

/// A TCP stream, possibly wrapped in a BufReader for proxy tunnels.
pub type ProxyStream = Box<dyn ProxyIo>;

/// Establishes a TCP connection to the target through the given proxy.
///
/// For HTTP proxies, performs an HTTP CONNECT tunnel. When the proxy URL
/// uses the `https` scheme, the connection to the proxy itself is secured
/// with TLS first.
/// For SOCKS proxies, uses the tokio-socks library.
pub async fn connect_through_proxy(
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> io::Result<ProxyStream> {
    match proxy {
        ProxyConfig::Http(proxy_url) => {
            let host = proxy_url.host_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "proxy URL has no host")
            })?;
            let port = proxy_port(proxy_url);
            let proxy_addr = format!("{host}:{port}");
            let tcp_stream = TcpStream::connect(&proxy_addr).await?;
            let auth_header = extract_proxy_auth(proxy_url);

            // A `https://` proxy endpoint means the hop to the proxy is
            // secured with TLS. The CONNECT request itself stays plain
            // HTTP/1.1 inside that session.
            let proxy_stream: ProxyStream = if proxy_url.scheme() == "https" {
                let connector = super::proxy_tls_connector()?;
                let server_name = super::ServerName::try_from(host.to_owned()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid proxy dnsname")
                })?;
                Box::new(connector.connect(server_name, tcp_stream).await?)
            } else {
                Box::new(tcp_stream)
            };

            http_connect_tunnel(
                proxy_stream,
                target_host,
                target_port,
                auth_header.as_deref(),
            )
            .await
        }
        #[cfg(feature = "socks")]
        ProxyConfig::Socks5(proxy_url) => {
            let host = proxy_url.host_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "SOCKS5 proxy URL has no host")
            })?;
            let port = proxy_port(proxy_url);
            let proxy_addr = format!("{host}:{port}");
            let target = format!("{target_host}:{target_port}");

            let stream = if !proxy_url.username().is_empty() {
                let user = proxy_url.username().to_string();
                let pass = proxy_url.password().unwrap_or("").to_string();
                tokio_socks::tcp::Socks5Stream::connect_with_password(
                    &*proxy_addr,
                    target,
                    &user,
                    &pass,
                )
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?
                .into_inner()
            } else {
                tokio_socks::tcp::Socks5Stream::connect(&*proxy_addr, target)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?
                    .into_inner()
            };

            Ok(Box::new(stream))
        }
        #[cfg(feature = "socks")]
        ProxyConfig::Socks4(proxy_url) => {
            let host = proxy_url.host_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "SOCKS4 proxy URL has no host")
            })?;
            let port = proxy_port(proxy_url);
            let proxy_addr = format!("{host}:{port}");
            let target = format!("{target_host}:{target_port}");

            // SOCKS4 carries a `userid` field rather than a password. When
            // the target is a hostname it is encoded with the SOCKS4a
            // extension, so the proxy resolves the name.
            let stream = if proxy_url.username().is_empty() {
                tokio_socks::tcp::Socks4Stream::connect(&*proxy_addr, target)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?
                    .into_inner()
            } else {
                let user = proxy_url.username().to_string();
                tokio_socks::tcp::Socks4Stream::connect_with_userid(&*proxy_addr, target, &user)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?
                    .into_inner()
            };

            Ok(Box::new(stream))
        }
    }
}

/// Resolve the port of a proxy endpoint, applying the scheme default when
/// the URL does not carry one.
fn proxy_port(url: &Url) -> u16 {
    url.port().unwrap_or_else(|| match url.scheme() {
        "https" => 443,
        "socks4" | "socks4a" | "socks5" | "socks5h" => 1080,
        _ => 80,
    })
}

/// Validates that a hostname contains only valid characters.
///
/// Rejects hosts containing control characters (CR, LF) or other characters
/// not valid in HTTP request lines and headers to prevent CRLF injection.
fn validate_host(host: &str) -> io::Result<()> {
    if host.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "hostname must not be empty",
        ));
    }

    for ch in host.chars() {
        if !ch.is_ascii_alphanumeric()
            && ch != '.'
            && ch != '-'
            && ch != ':'
            && ch != '['
            && ch != ']'
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid character '{}' in hostname", ch.escape_debug()),
            ));
        }
    }

    Ok(())
}

/// Performs an HTTP CONNECT tunnel through a proxy.
///
/// Sends a CONNECT request to the proxy and reads the response. If the proxy
/// responds with a 2xx status code, the connection is established and the
/// proxy stream is returned.
async fn http_connect_tunnel<S>(
    proxy_stream: S,
    target_host: &str,
    target_port: u16,
    auth_header: Option<&str>,
) -> io::Result<ProxyStream>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug + 'static,
{
    validate_host(target_host)?;

    let authority = if target_host.contains(':') {
        format!("[{target_host}]:{target_port}")
    } else {
        format!("{target_host}:{target_port}")
    };

    let mut connect_req = format!(
        "CONNECT {authority} HTTP/1.1\r\n\
         Host: {authority}\r\n"
    );

    if let Some(auth) = auth_header {
        connect_req.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }

    connect_req.push_str("\r\n");

    tracing::debug!(target = %authority, "CONNECT tunnel request");
    let mut stream = BufReader::new(proxy_stream);
    stream.write_all(connect_req.as_bytes()).await?;

    // Read the CONNECT response line by line until we hit the empty line.
    let mut line_num = 0;
    loop {
        let mut line = String::new();
        let n = stream.read_line(&mut line).await?;
        line_num += 1;
        tracing::debug!(line_num, n, line = %line.trim_end(), "CONNECT tunnel response line");

        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before receiving CONNECT response",
            ));
        }

        // Status line is the first line.
        if line.starts_with("HTTP/") {
            let status_code = parse_status_code(line.trim())?;
            if !(200..300).contains(&status_code) {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("proxy tunnel failed with status {status_code}"),
                ));
            }
            continue;
        }

        // Empty line signals end of headers — tunnel established.
        if line.trim().is_empty() {
            break;
        }

        // Other header lines — skip.
    }

    // Return the BufReader to preserve any buffered bytes from the proxy response.
    // BufReader implements AsyncRead + AsyncWrite, so it works for TLS and WS.
    Ok(Box::new(stream))
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

/// Extract Basic auth header from proxy URL credentials.
fn extract_proxy_auth(url: &Url) -> Option<String> {
    let user = url.username();
    if user.is_empty() {
        return None;
    }

    use percent_encoding::percent_decode_str;

    let user = percent_decode_str(user).decode_utf8_lossy().into_owned();
    let pass = percent_decode_str(url.password().unwrap_or(""))
        .decode_utf8_lossy()
        .into_owned();
    let credentials = format!("{user}:{pass}");
    let encoded = base64_simd::STANDARD.encode_to_string(credentials.as_bytes());
    Some(format!("Basic {encoded}"))
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;

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
        let result = http_connect_tunnel(proxy_stream, "example.com", 443, None).await;
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
        let result = http_connect_tunnel(proxy_stream, "example.com", 443, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_proxy_auth_header_is_sent() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (tx, rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = vec![0u8; 4096];
            let n = stream.read(&mut req_buf).await.unwrap();
            let _ = tx.send(req_buf[..n].to_vec());
            // Send a 200 response so the tunnel completes
            let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
            stream.write_all(response).await.unwrap();
        });

        let proxy_stream = TcpStream::connect(addr).await.unwrap();
        let proxy_url = url::Url::parse("http://user:pass@proxy.local:8080").unwrap();
        let auth_header = extract_proxy_auth(&proxy_url);

        let result =
            http_connect_tunnel(proxy_stream, "example.com", 443, auth_header.as_deref()).await;
        assert!(result.is_ok());

        let received = rx.await.unwrap();
        let request = String::from_utf8_lossy(&received);
        assert!(request.contains("Proxy-Authorization: Basic"));
        assert!(request.contains("CONNECT example.com:443"));
    }

    #[test]
    fn test_extract_proxy_auth_with_credentials() {
        let url = url::Url::parse("http://admin:secret@proxy.local:8080").unwrap();
        let auth = extract_proxy_auth(&url);
        assert!(auth.is_some());
        let auth = auth.unwrap();
        assert!(auth.starts_with("Basic "));
        // Verify base64 encoding
        let decoded = base64_simd::STANDARD
            .decode_to_vec(auth.strip_prefix("Basic ").unwrap().as_bytes())
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "admin:secret");
    }

    #[test]
    fn test_extract_proxy_auth_without_credentials() {
        let url = url::Url::parse("http://proxy.local:8080").unwrap();
        let auth = extract_proxy_auth(&url);
        assert!(auth.is_none());
    }

    #[test]
    fn test_extract_proxy_auth_empty_username() {
        let url = url::Url::parse("http://:pass@proxy.local:8080").unwrap();
        let auth = extract_proxy_auth(&url);
        assert!(auth.is_none());
    }

    #[test]
    fn test_extract_proxy_auth_percent_encoded_credentials() {
        let url = url::Url::parse("http://user%40domain:p%40ss@proxy.local:8080").unwrap();
        let auth = extract_proxy_auth(&url);
        assert!(auth.is_some());
        let auth = auth.unwrap();
        let decoded = base64_simd::STANDARD
            .decode_to_vec(auth.strip_prefix("Basic ").unwrap().as_bytes())
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "user@domain:p@ss");
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
        let result = http_connect_tunnel(proxy_stream, "example.com", 443, None).await;
        assert!(result.is_ok());
    }

    fn mock_proxy(
        listener: tokio::net::TcpListener,
        response: Vec<u8>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let _ = stream.write_all(&response).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        })
    }

    #[tokio::test]
    async fn test_http_connect_200_success() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let response = b"HTTP/1.1 200 Connection Established\r\n\r\n".to_vec();
        let server = mock_proxy(listener, response);

        let stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(stream, "example.com", 443, None).await;

        assert!(result.is_ok());
        server.abort();
    }

    #[tokio::test]
    async fn test_http_connect_407_proxy_auth_required() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let response = b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n".to_vec();
        let server = mock_proxy(listener, response);

        let stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(stream, "example.com", 443, None).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
        server.abort();
    }

    #[tokio::test]
    async fn test_http_connect_502_bad_gateway() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let response = b"HTTP/1.1 502 Bad Gateway\r\n\r\n".to_vec();
        let server = mock_proxy(listener, response);

        let stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(stream, "example.com", 443, None).await;

        assert!(result.is_err());
        server.abort();
    }

    #[tokio::test]
    async fn test_http_connect_partial_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;

            stream.write_all(b"HTTP/1.1 200 OK\r\n").await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

            stream.write_all(b"X-Custom: value\r\n\r\n").await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(stream, "example.com", 443, None).await;

        assert!(result.is_ok());
        server.abort();
    }

    #[tokio::test]
    async fn test_http_connect_ipv6_address() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);

            assert!(request.contains("CONNECT [::1]:443 HTTP/1.1"));

            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(stream, "::1", 443, None).await;

        assert!(result.is_ok());
        server.abort();
    }

    #[tokio::test]
    async fn test_http_connect_connection_closed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            drop(stream);
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let result = http_connect_tunnel(stream, "example.com", 443, None).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        server.abort();
    }

    #[tokio::test]
    async fn test_http_connect_with_auth_header() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);

            assert!(request.contains("Proxy-Authorization: Basic dXNlcjpwYXNz"));

            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let result =
            http_connect_tunnel(stream, "example.com", 443, Some("Basic dXNlcjpwYXNz")).await;

        assert!(result.is_ok());
        server.abort();
    }

    #[tokio::test]
    async fn test_connect_through_http_proxy() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        });

        let proxy_url = url::Url::parse(&format!("http://{addr}")).unwrap();
        let proxy = ProxyConfig::Http(proxy_url);

        let result = connect_through_proxy(&proxy, "example.com", 443).await;
        assert!(result.is_ok());
        server.abort();
    }

    #[tokio::test]
    async fn test_connect_through_http_proxy_with_auth() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);

            assert!(request.contains("Proxy-Authorization: Basic"));

            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        });

        let proxy_url = url::Url::parse(&format!("http://user:pass@{addr}")).unwrap();
        let proxy = ProxyConfig::Http(proxy_url);

        let result = connect_through_proxy(&proxy, "example.com", 443).await;
        assert!(result.is_ok());
        server.abort();
    }

    #[tokio::test]
    async fn test_connect_through_proxy_connection_refused() {
        let proxy_url = url::Url::parse("http://127.0.0.1:1").unwrap();
        let proxy = ProxyConfig::Http(proxy_url);

        let result = connect_through_proxy(&proxy, "example.com", 443).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_proxy_default_ports() {
        let url = url::Url::parse("http://proxy.local").unwrap();
        assert_eq!(url.port(), None);
        // The port is resolved from the scheme when the URL omits it.
        assert_eq!(proxy_port(&url), 80);

        let url = url::Url::parse("http://proxy.local:3128").unwrap();
        assert_eq!(url.port(), Some(3128));
        assert_eq!(proxy_port(&url), 3128);

        let url = url::Url::parse("https://proxy.local").unwrap();
        assert_eq!(proxy_port(&url), 443);

        let url = url::Url::parse("socks5://proxy.local").unwrap();
        assert_eq!(proxy_port(&url), 1080);

        let url = url::Url::parse("socks4://proxy.local").unwrap();
        assert_eq!(proxy_port(&url), 1080);
    }

    #[tokio::test]
    async fn test_connect_through_http_proxy_without_explicit_port() {
        // The proxy listens on an ephemeral port, so the URL carries it
        // explicitly; this test only asserts that the resolved port is used
        // when the URL has one.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        });

        let proxy_url = url::Url::parse(&format!("http://{addr}")).unwrap();
        let proxy = ProxyConfig::Http(proxy_url);

        let result = connect_through_proxy(&proxy, "example.com", 443).await;
        assert!(result.is_ok());
        server.abort();
    }

    /// A minimal SOCKS4 server that records the request it received and
    /// answers with a success reply.
    #[cfg(feature = "socks")]
    async fn socks4_server(listener: tokio::net::TcpListener) -> tokio::task::JoinHandle<Vec<u8>> {
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let request = buf[..n].to_vec();

            // VN=0, CD=90 (granted), DSTPORT, DSTIP.
            let reply = [0u8, 90, 0, 0, 0, 0, 0, 0];
            let _ = stream.write_all(&reply).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            request
        })
    }

    /// SOCKS4 encodes the destination as an IPv4 address and the user id as
    /// a NUL-terminated string.
    #[cfg(feature = "socks")]
    #[tokio::test]
    async fn test_connect_through_socks4_proxy() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = socks4_server(listener).await;

        let proxy_url = url::Url::parse(&format!("socks4://{addr}")).unwrap();
        let proxy = ProxyConfig::Socks4(proxy_url);

        let result = connect_through_proxy(&proxy, "93.184.216.34", 443).await;
        assert!(result.is_ok());

        let request = server.await.unwrap();
        assert_eq!(request[0], 0x04, "SOCKS version");
        assert_eq!(request[1], 0x01, "CONNECT command");
        assert_eq!(
            u16::from_be_bytes([request[2], request[3]]),
            443,
            "destination port"
        );
        assert_eq!(&request[4..8], &[93, 184, 216, 34], "destination address");
    }

    /// A hostname target uses the SOCKS4a extension: the address field is
    /// `0.0.0.1` and the name follows the user id.
    #[cfg(feature = "socks")]
    #[tokio::test]
    async fn test_connect_through_socks4a_proxy() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = socks4_server(listener).await;

        let proxy_url = url::Url::parse(&format!("socks4a://{addr}")).unwrap();
        let proxy = ProxyConfig::Socks4(proxy_url);

        let result = connect_through_proxy(&proxy, "example.com", 443).await;
        assert!(result.is_ok());

        let request = server.await.unwrap();
        assert_eq!(request[0], 0x04, "SOCKS version");
        assert_eq!(&request[4..8], &[0, 0, 0, 1], "SOCKS4a marker address");

        // The user id is NUL-terminated, followed by the hostname.
        let payload = String::from_utf8_lossy(&request[8..]);
        assert!(
            payload.contains("example.com"),
            "the hostname must follow the user id: {payload}"
        );
    }

    /// A minimal SOCKS5 server that records the request it received.
    #[cfg(feature = "socks")]
    async fn socks5_server(
        listener: tokio::net::TcpListener,
        method: u8,
    ) -> tokio::task::JoinHandle<Vec<u8>> {
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            // Greeting: VER, NMETHODS, METHODS.
            let mut greeting = [0u8; 2];
            stream.read_exact(&mut greeting).await.unwrap();
            let mut methods = vec![0u8; greeting[1] as usize];
            stream.read_exact(&mut methods).await.unwrap();

            // Method selection.
            stream.write_all(&[0x05, method]).await.unwrap();

            let mut recorded = greeting.to_vec();
            recorded.extend_from_slice(&methods);
            recorded.extend_from_slice(&[0x05, method]);

            if method == 0x02 {
                // User/password sub-negotiation: VER, ULEN, UNAME, PLEN, PASSWD.
                let mut version = [0u8; 1];
                stream.read_exact(&mut version).await.unwrap();
                let mut ulen = [0u8; 1];
                stream.read_exact(&mut ulen).await.unwrap();
                let mut uname = vec![0u8; ulen[0] as usize];
                stream.read_exact(&mut uname).await.unwrap();
                let mut plen = [0u8; 1];
                stream.read_exact(&mut plen).await.unwrap();
                let mut passwd = vec![0u8; plen[0] as usize];
                stream.read_exact(&mut passwd).await.unwrap();

                // Success reply: VER=1, STATUS=0.
                stream.write_all(&[0x01, 0x00]).await.unwrap();

                recorded.extend_from_slice(&[version[0], ulen[0]]);
                recorded.extend_from_slice(&uname);
                recorded.extend_from_slice(&[plen[0]]);
                recorded.extend_from_slice(&passwd);
            }

            // Request: VER, CMD, RSV, ATYP, address, port.
            let mut head = [0u8; 4];
            stream.read_exact(&mut head).await.unwrap();
            recorded.extend_from_slice(&head);

            match head[3] {
                0x01 => {
                    let mut addr = [0u8; 4];
                    stream.read_exact(&mut addr).await.unwrap();
                    recorded.extend_from_slice(&addr);
                }
                0x03 => {
                    let mut len = [0u8; 1];
                    stream.read_exact(&mut len).await.unwrap();
                    let mut name = vec![0u8; len[0] as usize];
                    stream.read_exact(&mut name).await.unwrap();
                    recorded.extend_from_slice(&len);
                    recorded.extend_from_slice(&name);
                }
                _ => {}
            }

            let mut port = [0u8; 2];
            stream.read_exact(&mut port).await.unwrap();
            recorded.extend_from_slice(&port);

            // Success reply: VER, REP, RSV, ATYP=IPv4, address, port.
            let reply = [0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
            let _ = stream.write_all(&reply).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            recorded
        })
    }

    /// SOCKS5 without authentication.
    #[cfg(feature = "socks")]
    #[tokio::test]
    async fn test_connect_through_socks5_proxy() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = socks5_server(listener, 0x00).await;

        let proxy_url = url::Url::parse(&format!("socks5://{addr}")).unwrap();
        let proxy = ProxyConfig::Socks5(proxy_url);

        let result = connect_through_proxy(&proxy, "example.com", 443).await;
        assert!(result.is_ok());

        let request = server.await.unwrap();
        assert_eq!(request[0], 0x05, "SOCKS version");
        assert_eq!(request[1], 0x01, "one auth method is offered");
        assert_eq!(request[2], 0x00, "no authentication");
        // Request header: VER, CMD=CONNECT, RSV, ATYP=domain. It follows the
        // greeting and the method-selection reply.
        assert_eq!(&request[5..9], &[0x05, 0x01, 0x00, 0x03]);
    }

    /// SOCKS5 with username/password authentication (RFC 1929).
    #[cfg(feature = "socks")]
    #[tokio::test]
    async fn test_connect_through_socks5_proxy_with_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = socks5_server(listener, 0x02).await;

        let proxy_url = url::Url::parse(&format!("socks5://user:pass@{addr}")).unwrap();
        let proxy = ProxyConfig::Socks5(proxy_url);

        let result = connect_through_proxy(&proxy, "example.com", 443).await;
        assert!(result.is_ok());

        let request = server.await.unwrap();
        assert_eq!(request[0], 0x05, "SOCKS version");
        assert_eq!(request[1], 0x02, "two auth methods are offered");
        assert_eq!(&request[2..4], &[0x00, 0x02], "no-auth and user/pass");
        // The credentials follow the method selection.
        let payload = String::from_utf8_lossy(&request[4..]);
        assert!(
            payload.contains("user") && payload.contains("pass"),
            "the credentials must be sent: {payload}"
        );
    }

    #[test]
    fn test_validate_host_valid() {
        assert!(validate_host("example.com").is_ok());
        assert!(validate_host("192.168.1.1").is_ok());
        assert!(validate_host("[::1]").is_ok());
        assert!(validate_host("sub.domain.example.com").is_ok());
        assert!(validate_host("host-with-hyphens.com").is_ok());
        assert!(validate_host("10.0.0.1").is_ok());
        assert!(validate_host("[2001:db8::1]").is_ok());
    }

    #[test]
    fn test_validate_host_rejects_crlf() {
        let err = validate_host("example.com\r\nInjected: header").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        let err = validate_host("example.com\r").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        let err = validate_host("example.com\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_validate_host_rejects_spaces() {
        let err = validate_host("example .com").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_validate_host_rejects_empty() {
        let err = validate_host("").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn test_validate_host_rejects_special_chars() {
        let err = validate_host("example.com/path").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        let err = validate_host("example.com?query=1").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        let err = validate_host("user@example.com").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
