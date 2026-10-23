//! SOCKS5 UDP ASSOCIATE (RFC 1928), used to tunnel QUIC through a SOCKS5
//! proxy.
//!
//! QUIC runs over UDP, so it cannot be tunnelled through an HTTP `CONNECT`
//! tunnel. A SOCKS5 proxy can relay UDP datagrams, which is the only way to
//! carry HTTP/3 through a proxy.
//!
//! The client opens a TCP control connection to the proxy and asks it to
//! relay UDP. The proxy answers with the address its relay listens on. From
//! then on, every datagram is wrapped in a SOCKS5 UDP header:
//!
//! ```text
//! +--------+--------+--------+--------+
//! | RSV    | FRAG   | ATYP   | DST.ADDR
//! +--------+--------+--------+--------+
//! | DST.PORT | DATA                |
//! +---------+----------------------+
//! ```

use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use quinn::{
    AsyncUdpSocket, UdpPoller,
    udp::{RecvMeta, Transmit},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

/// The SOCKS5 version byte.
const SOCKS5_VERSION: u8 = 0x05;
/// The `UDP ASSOCIATE` command.
const CMD_UDP_ASSOCIATE: u8 = 0x03;
/// The `no authentication required` method.
const METHOD_NONE: u8 = 0x00;
/// The `username/password` method.
const METHOD_USER_PASSWORD: u8 = 0x02;
/// The `IPv4` address type.
const ATYP_IPV4: u8 = 0x01;
/// The `domain name` address type.
const ATYP_DOMAIN: u8 = 0x03;
/// The `IPv6` address type.
const ATYP_IPV6: u8 = 0x04;

/// The largest datagram the relay handles.
const MAX_DATAGRAM: usize = 65_535;

/// A UDP socket that tunnels datagrams through a SOCKS5 UDP relay.
///
/// The datagrams are wrapped in the SOCKS5 UDP header on the way out and
/// unwrapped on the way in, so the QUIC stack above sees plain UDP.
#[derive(Debug)]
pub(crate) struct Socks5UdpSocket {
    /// The socket connected to the proxy's relay address.
    socket: Arc<UdpSocket>,
    /// The control connection.
    ///
    /// The proxy closes the relay when this connection is closed, so it has
    /// to be kept alive for as long as the socket is used.
    _control: TcpStream,
}

impl Socks5UdpSocket {
    /// Establishes a UDP relay through the SOCKS5 proxy at `proxy`.
    ///
    /// The optional credentials are used when the proxy asks for
    /// username/password authentication.
    pub(crate) async fn connect(
        proxy: SocketAddr,
        credentials: Option<(String, String)>,
    ) -> io::Result<Self> {
        let mut control = TcpStream::connect(proxy).await?;

        // Greeting: the methods the client supports.
        control.write_all(&[SOCKS5_VERSION, 1, METHOD_NONE]).await?;
        // When credentials are available, offer the password method too.
        if credentials.is_some() {
            control.write_all(&[METHOD_USER_PASSWORD]).await?;
        }
        control.flush().await?;

        let mut greeting = [0u8; 2];
        control.read_exact(&mut greeting).await?;
        if greeting[0] != SOCKS5_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid SOCKS version in the method selection",
            ));
        }

        match greeting[1] {
            METHOD_NONE => {}
            METHOD_USER_PASSWORD => {
                let (username, password) = credentials.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "the SOCKS5 proxy requires a username and password",
                    )
                })?;
                authenticate(&mut control, &username, &password).await?;
            }
            method => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("the SOCKS5 proxy requires the unsupported method {method}"),
                ));
            }
        }

        // The UDP ASSOCIATE request. The address is the one the client will
        // send the datagrams from, which the proxy relays as-is.
        let local = UdpSocket::bind("0.0.0.0:0").await?;
        let local_addr = local.local_addr()?;
        control
            .write_all(&[
                SOCKS5_VERSION,
                CMD_UDP_ASSOCIATE,
                0x00,
                ATYP_IPV4,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
            ])
            .await?;
        control.flush().await?;

        let relay = read_reply_address(&mut control).await?;

        // The relay address is often reported as 0.0.0.0, which means "the
        // address the client connected to".
        let relay = if relay.ip().is_unspecified() {
            SocketAddr::new(proxy.ip(), relay.port())
        } else {
            relay
        };

        let socket = UdpSocket::bind(SocketAddr::new(
            if local_addr.is_ipv4() {
                std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
            } else {
                std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
            },
            0,
        ))
        .await?;
        socket.connect(relay).await?;

        Ok(Self {
            socket: Arc::new(socket),
            _control: control,
        })
    }
}

/// Performs the SOCKS5 username/password sub-negotiation (RFC 1929).
async fn authenticate(control: &mut TcpStream, username: &str, password: &str) -> io::Result<()> {
    let username = username.as_bytes();
    let password = password.as_bytes();
    if username.len() > u8::MAX as usize || password.len() > u8::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the SOCKS5 credentials are too long",
        ));
    }

    let mut request = Vec::with_capacity(3 + username.len() + password.len());
    request.push(0x01);
    request.push(username.len() as u8);
    request.extend_from_slice(username);
    request.push(password.len() as u8);
    request.extend_from_slice(password);
    control.write_all(&request).await?;
    control.flush().await?;

    let mut reply = [0u8; 2];
    control.read_exact(&mut reply).await?;
    if reply[1] != 0x00 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the SOCKS5 proxy rejected the credentials",
        ));
    }

    Ok(())
}

/// Reads the bound address from a SOCKS5 reply.
async fn read_reply_address(control: &mut TcpStream) -> io::Result<SocketAddr> {
    let mut head = [0u8; 4];
    control.read_exact(&mut head).await?;
    if head[0] != SOCKS5_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid SOCKS version in the reply",
        ));
    }
    if head[1] != 0x00 {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("the SOCKS5 proxy refused the request: {}", head[1]),
        ));
    }

    let address = match head[3] {
        ATYP_IPV4 => {
            let mut octets = [0u8; 4];
            control.read_exact(&mut octets).await?;
            std::net::IpAddr::V4(std::net::Ipv4Addr::from(octets))
        }
        ATYP_IPV6 => {
            let mut octets = [0u8; 16];
            control.read_exact(&mut octets).await?;
            std::net::IpAddr::V6(std::net::Ipv6Addr::from(octets))
        }
        ATYP_DOMAIN => {
            let mut length = [0u8; 1];
            control.read_exact(&mut length).await?;
            let mut name = vec![0u8; length[0] as usize];
            control.read_exact(&mut name).await?;
            let host = String::from_utf8_lossy(&name).into_owned();
            host.parse::<std::net::IpAddr>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the SOCKS5 proxy returned an invalid host name",
                )
            })?
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the SOCKS5 proxy returned the unknown address type {other}"),
            ));
        }
    };

    let mut port = [0u8; 2];
    control.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);

    Ok(SocketAddr::new(address, port))
}

/// Writes the SOCKS5 UDP header for `destination` into `buf`.
fn encode_udp_header(buf: &mut Vec<u8>, destination: SocketAddr) {
    // RSV (2 bytes) and FRAG (1 byte, always zero: no fragmentation).
    buf.extend_from_slice(&[0x00, 0x00, 0x00]);
    match destination {
        SocketAddr::V4(addr) => {
            buf.push(ATYP_IPV4);
            buf.extend_from_slice(&addr.ip().octets());
        }
        SocketAddr::V6(addr) => {
            buf.push(ATYP_IPV6);
            buf.extend_from_slice(&addr.ip().octets());
        }
    }
    buf.extend_from_slice(&destination.port().to_be_bytes());
}

/// Reads the SOCKS5 UDP header, returning the source address and payload.
fn decode_udp_header(datagram: &[u8]) -> io::Result<(SocketAddr, &[u8])> {
    if datagram.len() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the SOCKS5 UDP datagram is too short",
        ));
    }

    let frag = datagram[2];
    if frag != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the SOCKS5 proxy returned a fragmented datagram",
        ));
    }

    let address = match datagram[3] {
        ATYP_IPV4 => {
            if datagram.len() < 8 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the SOCKS5 UDP datagram is too short",
                ));
            }
            let mut octets = [0u8; 4];
            octets.copy_from_slice(&datagram[4..8]);
            std::net::IpAddr::V4(std::net::Ipv4Addr::from(octets))
        }
        ATYP_IPV6 => {
            if datagram.len() < 20 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the SOCKS5 UDP datagram is too short",
                ));
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&datagram[4..20]);
            std::net::IpAddr::V6(std::net::Ipv6Addr::from(octets))
        }
        ATYP_DOMAIN => {
            let length = *datagram.get(4).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the SOCKS5 UDP datagram is too short",
                )
            })? as usize;
            if datagram.len() < 5 + length {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the SOCKS5 UDP datagram is too short",
                ));
            }
            let host = String::from_utf8_lossy(&datagram[5..5 + length]).into_owned();
            host.parse::<std::net::IpAddr>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the SOCKS5 proxy returned an invalid host name",
                )
            })?
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the SOCKS5 proxy returned the unknown address type {other}"),
            ));
        }
    };

    let port_offset = match address {
        std::net::IpAddr::V4(_) => 8,
        std::net::IpAddr::V6(_) => 20,
    };
    let port = datagram.get(port_offset..port_offset + 2).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the SOCKS5 UDP datagram is too short",
        )
    })?;

    Ok((
        SocketAddr::new(address, u16::from_be_bytes([port[0], port[1]])),
        &datagram[port_offset + 2..],
    ))
}

impl AsyncUdpSocket for Socks5UdpSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(Socks5UdpPoller {
            socket: self.socket.clone(),
        })
    }

    fn try_send(&self, transmit: &Transmit<'_>) -> io::Result<()> {
        let mut buf = Vec::with_capacity(transmit.contents.len() + 32);
        encode_udp_header(&mut buf, transmit.destination);
        buf.extend_from_slice(transmit.contents);
        self.socket.try_send(&buf).map(|_| ())
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let Some(buf) = bufs.first_mut() else {
            return Poll::Ready(Ok(0));
        };
        let Some(meta) = meta.first_mut() else {
            return Poll::Ready(Ok(0));
        };

        loop {
            std::task::ready!(self.socket.poll_recv_ready(cx))?;

            let mut datagram = vec![0u8; MAX_DATAGRAM];
            match self.socket.try_recv(&mut datagram) {
                Ok(n) => {
                    let (source, payload) = decode_udp_header(&datagram[..n])?;
                    let len = payload.len().min(buf.len());
                    buf[..len].copy_from_slice(&payload[..len]);
                    meta.addr = source;
                    meta.len = len;
                    meta.stride = len;
                    meta.ecn = None;
                    meta.dst_ip = None;
                    return Poll::Ready(Ok(1));
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

/// Reports the readiness of the relay socket.
#[derive(Debug)]
struct Socks5UdpPoller {
    socket: Arc<UdpSocket>,
}

impl UdpPoller for Socks5UdpPoller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.socket.poll_send_ready(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_the_udp_header() {
        let mut buf = Vec::new();
        encode_udp_header(&mut buf, "192.0.2.1:443".parse().unwrap());
        buf.extend_from_slice(b"payload");

        let (source, payload) = decode_udp_header(&buf).unwrap();
        assert_eq!(source, "192.0.2.1:443".parse().unwrap());
        assert_eq!(payload, b"payload");
    }

    #[test]
    fn encodes_and_decodes_an_ipv6_header() {
        let mut buf = Vec::new();
        encode_udp_header(&mut buf, "[2001:db8::1]:443".parse().unwrap());
        buf.extend_from_slice(b"payload");

        let (source, payload) = decode_udp_header(&buf).unwrap();
        assert_eq!(source, "[2001:db8::1]:443".parse().unwrap());
        assert_eq!(payload, b"payload");
    }

    #[test]
    fn rejects_a_truncated_datagram() {
        assert!(decode_udp_header(&[0x00, 0x00, 0x00]).is_err());
        assert!(decode_udp_header(&[0x00, 0x00, 0x00, ATYP_IPV4, 1, 2]).is_err());
    }

    #[test]
    fn rejects_a_fragmented_datagram() {
        let mut buf = Vec::new();
        encode_udp_header(&mut buf, "192.0.2.1:443".parse().unwrap());
        buf[2] = 0x01;
        assert!(decode_udp_header(&buf).is_err());
    }

    #[test]
    fn rejects_an_unknown_address_type() {
        let buf = [0x00, 0x00, 0x00, 0x09];
        assert!(decode_udp_header(&buf).is_err());
    }
}
