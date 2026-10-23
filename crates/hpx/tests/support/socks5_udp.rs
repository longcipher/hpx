#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used,
    dead_code
)]
//! A minimal SOCKS5 proxy that relays UDP datagrams.
//!
//! The proxy supports the `UDP ASSOCIATE` command (RFC 1928): the client
//! opens a TCP control connection, asks for a UDP relay, and then sends its
//! datagrams to the relay address the proxy answers with. Every datagram is
//! wrapped in the SOCKS5 UDP header.

use std::net::SocketAddr;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
};

/// A running SOCKS5 UDP relay proxy.
pub(crate) struct Socks5UdpProxy {
    /// The address the client connects to.
    addr: SocketAddr,
    /// The task that serves the proxy.
    task: tokio::task::JoinHandle<()>,
}

impl Socks5UdpProxy {
    /// Starts the proxy on an ephemeral loopback port.
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let task = tokio::spawn(async move {
            let (mut control, _) = listener.accept().await.unwrap();

            // Greeting: VER, NMETHODS, METHODS.
            let mut greeting = [0u8; 2];
            control.read_exact(&mut greeting).await.unwrap();
            let mut methods = vec![0u8; greeting[1] as usize];
            control.read_exact(&mut methods).await.unwrap();

            // Method selection: no authentication.
            control.write_all(&[0x05, 0x00]).await.unwrap();

            // Request: VER, CMD, RSV, ATYP, ADDR, PORT.
            let mut head = [0u8; 4];
            control.read_exact(&mut head).await.unwrap();
            assert_eq!(head[0], 0x05);
            assert_eq!(head[1], 0x03, "the UDP ASSOCIATE command is expected");

            let relay_addr = match head[3] {
                0x01 => {
                    let mut octets = [0u8; 4];
                    control.read_exact(&mut octets).await.unwrap();
                    std::net::IpAddr::V4(std::net::Ipv4Addr::from(octets))
                }
                0x03 => {
                    let mut length = [0u8; 1];
                    control.read_exact(&mut length).await.unwrap();
                    let mut name = vec![0u8; length[0] as usize];
                    control.read_exact(&mut name).await.unwrap();
                    String::from_utf8_lossy(&name).parse().unwrap()
                }
                // An unknown address type ends the proxy.
                _ => return,
            };
            let mut port = [0u8; 2];
            control.read_exact(&mut port).await.unwrap();
            let _ = (relay_addr, u16::from_be_bytes(port));

            // The relay sockets. The client sends its wrapped datagrams to
            // `client_socket`; the destination answers on `relay_socket`.
            let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let client_addr = client_socket.local_addr().unwrap();
            let relay_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

            // Reply: VER, REP, RSV, ATYP=IPv4, BND.ADDR, BND.PORT.
            let mut reply = vec![0x05, 0x00, 0x00, 0x01];
            reply.extend_from_slice(&[127, 0, 0, 1]);
            reply.extend_from_slice(&client_addr.port().to_be_bytes());
            control.write_all(&reply).await.unwrap();
            control.flush().await.unwrap();

            // Relay the datagrams until the control connection is closed.
            let mut client_buf = vec![0u8; 65_535];
            let mut relay_buf = vec![0u8; 65_535];
            let mut client_peer: Option<SocketAddr> = None;
            let mut destination: Option<SocketAddr> = None;

            loop {
                tokio::select! {
                    // A datagram from the client: unwrap it and forward the
                    // payload to the destination.
                    received = client_socket.recv_from(&mut client_buf) => {
                        let Ok((len, peer)) = received else { break };
                        client_peer = Some(peer);
                        let Some((target, payload)) = decode_udp_header(&client_buf[..len]) else {
                            continue;
                        };
                        destination = Some(target);
                        if relay_socket.send_to(payload, target).await.is_err() {
                            break;
                        }
                    }
                    // A datagram from the destination: wrap it and send it
                    // back to the client. The header carries the address of
                    // the sender, so the client can tell where the datagram
                    // came from.
                    received = relay_socket.recv_from(&mut relay_buf) => {
                        let Ok((len, _)) = received else { break };
                        let Some(peer) = client_peer else { continue };
                        let Some(target) = destination else { continue };
                        let mut wrapped = Vec::with_capacity(len + 32);
                        encode_udp_header(&mut wrapped, target);
                        wrapped.extend_from_slice(&relay_buf[..len]);
                        if client_socket.send_to(&wrapped, peer).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self { addr, task }
    }

    /// The proxy URI.
    pub(crate) fn url(&self) -> String {
        format!("socks5://{}", self.addr)
    }
}

impl Drop for Socks5UdpProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Writes the SOCKS5 UDP header for `destination` into `buf`.
fn encode_udp_header(buf: &mut Vec<u8>, destination: SocketAddr) {
    // RSV (2 bytes) and FRAG (1 byte, always zero).
    buf.extend_from_slice(&[0x00, 0x00, 0x00]);
    match destination {
        SocketAddr::V4(addr) => {
            buf.push(0x01);
            buf.extend_from_slice(&addr.ip().octets());
        }
        SocketAddr::V6(addr) => {
            buf.push(0x04);
            buf.extend_from_slice(&addr.ip().octets());
        }
    }
    buf.extend_from_slice(&destination.port().to_be_bytes());
}

/// Reads the SOCKS5 UDP header, returning the destination and the payload.
fn decode_udp_header(datagram: &[u8]) -> Option<(SocketAddr, &[u8])> {
    assert_eq!(datagram[2], 0x00, "fragmented datagrams are not supported");
    let address = match datagram[3] {
        0x01 => {
            let mut octets = [0u8; 4];
            octets.copy_from_slice(&datagram[4..8]);
            std::net::IpAddr::V4(std::net::Ipv4Addr::from(octets))
        }
        0x04 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&datagram[4..20]);
            std::net::IpAddr::V6(std::net::Ipv6Addr::from(octets))
        }
        // An unknown address type cannot be forwarded.
        _ => return None,
    };
    let port_offset = match address {
        std::net::IpAddr::V4(_) => 8,
        std::net::IpAddr::V6(_) => 20,
    };
    let port = u16::from_be_bytes([datagram[port_offset], datagram[port_offset + 1]]);
    Some((SocketAddr::new(address, port), &datagram[port_offset + 2..]))
}
