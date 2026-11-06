#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::unwrap_used,
    dead_code
)]
//! Minimal tunnelling proxies: HTTP `CONNECT` and SOCKS5 `CONNECT`.
//!
//! Each proxy records the destination a client asks for, then relays bytes
//! to one fixed target whatever the destination was. A test can therefore
//! point a client at a host name that does not resolve and still reach a
//! local server, which proves the connection went through the proxy.

use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

/// The protocol a [`TunnelProxy`] speaks to its clients.
#[derive(Clone, Copy)]
enum Kind {
    HttpConnect,
    Socks5,
}

/// A running tunnelling proxy on an ephemeral loopback port.
pub(crate) struct TunnelProxy {
    addr: SocketAddr,
    destinations: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl TunnelProxy {
    /// Starts an HTTP proxy that accepts only `CONNECT` requests.
    pub(crate) async fn http(target: SocketAddr) -> Self {
        Self::start(Kind::HttpConnect, target).await
    }

    /// Starts a SOCKS5 proxy (RFC 1928) that accepts only `CONNECT`, without
    /// authentication.
    pub(crate) async fn socks5(target: SocketAddr) -> Self {
        Self::start(Kind::Socks5, target).await
    }

    /// The address clients connect to.
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The `host:port` destinations clients asked for, in arrival order.
    pub(crate) fn destinations(&self) -> Vec<String> {
        self.destinations.lock().unwrap().clone()
    }

    async fn start(kind: Kind, target: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let destinations = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&destinations);

        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    break;
                };
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move {
                    let _ = relay(kind, client, target, recorded).await;
                });
            }
        });

        Self {
            addr,
            destinations,
            task,
        }
    }
}

impl Drop for TunnelProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn relay(
    kind: Kind,
    mut client: TcpStream,
    target: SocketAddr,
    recorded: Arc<Mutex<Vec<String>>>,
) -> io::Result<()> {
    let destination = match kind {
        Kind::HttpConnect => http_connect_handshake(&mut client).await?,
        Kind::Socks5 => socks5_handshake(&mut client).await?,
    };
    recorded.lock().unwrap().push(destination);

    let mut upstream = TcpStream::connect(target).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Reads a `CONNECT` request head, answers `200`, and returns its target.
async fn http_connect_handshake(client: &mut TcpStream) -> io::Result<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        client.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    let target = head
        .strip_prefix("CONNECT ")
        .and_then(|rest| rest.split(' ').next())
        .ok_or_else(|| io::Error::other(format!("not a CONNECT request: {head}")))?
        .to_owned();

    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    Ok(target)
}

/// Runs the SOCKS5 greeting and `CONNECT` request, and returns the requested
/// destination.
async fn socks5_handshake(client: &mut TcpStream) -> io::Result<String> {
    // Greeting: VER, NMETHODS, METHODS. Answer "no authentication required".
    let mut greeting = [0u8; 2];
    client.read_exact(&mut greeting).await?;
    let mut methods = vec![0u8; usize::from(greeting[1])];
    client.read_exact(&mut methods).await?;
    client.write_all(&[0x05, 0x00]).await?;

    // Request: VER, CMD, RSV, ATYP, DST.ADDR, DST.PORT.
    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    if request[1] != 0x01 {
        return Err(io::Error::other("only CONNECT is supported"));
    }
    let host = match request[3] {
        0x01 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip).await?;
            Ipv4Addr::from(ip).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut name = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        0x04 => {
            let mut ip = [0u8; 16];
            client.read_exact(&mut ip).await?;
            format!("[{}]", Ipv6Addr::from(ip))
        }
        other => return Err(io::Error::other(format!("unknown address type {other}"))),
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);

    // Reply: succeeded, bound to 0.0.0.0:0.
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;
    Ok(format!("{host}:{port}"))
}
