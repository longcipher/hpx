use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(feature = "boring")]
use tokio_boring2::SslStream;
#[cfg(feature = "rustls-tls")]
use tokio_rustls::client::TlsStream;

use crate::tls::{TlsInfo, conn::MaybeHttpsStream};

/// A trait for extracting TLS information from a connection.
///
/// Implementors can provide access to peer certificate data or other TLS-related metadata.
/// For non-TLS connections, this typically returns `None`.
pub trait TlsInfoFactory {
    fn tls_info(&self) -> Option<TlsInfo>;
}

// ===== impl TcpStream =====

impl TlsInfoFactory for TcpStream {
    fn tls_info(&self) -> Option<TlsInfo> {
        None
    }
}

#[cfg(feature = "boring")]
impl TlsInfoFactory for SslStream<TcpStream> {
    fn tls_info(&self) -> Option<TlsInfo> {
        self.ssl().peer_certificate().map(|c| TlsInfo {
            peer_certificate: c.to_der().ok(),
        })
    }
}

#[cfg(feature = "rustls-tls")]
impl TlsInfoFactory for TlsStream<TcpStream> {
    fn tls_info(&self) -> Option<TlsInfo> {
        self.get_ref().1.peer_certificates().and_then(|certs| {
            certs.first().map(|c| TlsInfo {
                peer_certificate: Some(c.to_vec()),
            })
        })
    }
}

impl TlsInfoFactory for MaybeHttpsStream<TcpStream> {
    fn tls_info(&self) -> Option<TlsInfo> {
        match self {
            MaybeHttpsStream::Https(tls) => tls.tls_info(),
            MaybeHttpsStream::Http(_) => None,
        }
    }
}

#[cfg(feature = "boring")]
impl TlsInfoFactory for SslStream<MaybeHttpsStream<TcpStream>> {
    fn tls_info(&self) -> Option<TlsInfo> {
        self.ssl().peer_certificate().map(|c| TlsInfo {
            peer_certificate: c.to_der().ok(),
        })
    }
}

#[cfg(feature = "rustls-tls")]
impl TlsInfoFactory for TlsStream<MaybeHttpsStream<TcpStream>> {
    fn tls_info(&self) -> Option<TlsInfo> {
        self.get_ref().1.peer_certificates().and_then(|certs| {
            certs.first().map(|c| TlsInfo {
                peer_certificate: Some(c.to_vec()),
            })
        })
    }
}

// ===== impl UnixStream =====

#[cfg(unix)]
impl TlsInfoFactory for UnixStream {
    fn tls_info(&self) -> Option<TlsInfo> {
        None
    }
}

#[cfg(all(unix, feature = "boring"))]
impl TlsInfoFactory for SslStream<UnixStream> {
    fn tls_info(&self) -> Option<TlsInfo> {
        self.ssl().peer_certificate().map(|c| TlsInfo {
            peer_certificate: c.to_der().ok(),
        })
    }
}

#[cfg(all(unix, feature = "rustls-tls"))]
impl TlsInfoFactory for TlsStream<UnixStream> {
    fn tls_info(&self) -> Option<TlsInfo> {
        self.get_ref().1.peer_certificates().and_then(|certs| {
            certs.first().map(|c| TlsInfo {
                peer_certificate: Some(c.to_vec()),
            })
        })
    }
}

#[cfg(unix)]
impl TlsInfoFactory for MaybeHttpsStream<UnixStream> {
    fn tls_info(&self) -> Option<TlsInfo> {
        match self {
            MaybeHttpsStream::Https(tls) => tls.tls_info(),
            MaybeHttpsStream::Http(_) => None,
        }
    }
}

#[cfg(all(unix, feature = "boring"))]
impl TlsInfoFactory for SslStream<MaybeHttpsStream<UnixStream>> {
    fn tls_info(&self) -> Option<TlsInfo> {
        self.ssl().peer_certificate().map(|c| TlsInfo {
            peer_certificate: c.to_der().ok(),
        })
    }
}

#[cfg(all(unix, feature = "rustls-tls"))]
impl TlsInfoFactory for TlsStream<MaybeHttpsStream<UnixStream>> {
    fn tls_info(&self) -> Option<TlsInfo> {
        self.get_ref().1.peer_certificates().and_then(|certs| {
            certs.first().map(|c| TlsInfo {
                peer_certificate: Some(c.to_vec()),
            })
        })
    }
}
