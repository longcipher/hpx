use bytes::Bytes;
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(feature = "boring")]
use tokio_boring::SslStream;
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

#[cfg(feature = "boring")]
fn extract_tls_info<S>(ssl_stream: &SslStream<S>) -> TlsInfo {
    let ssl = ssl_stream.ssl();
    TlsInfo {
        peer_certificate: ssl
            .peer_certificate()
            .and_then(|cert| cert.to_der().ok())
            .map(Bytes::from),
        peer_certificate_chain: ssl.peer_cert_chain().map(|chain| {
            chain
                .iter()
                .filter_map(|cert| cert.to_der().ok())
                .map(Bytes::from)
                .collect()
        }),
    }
}

// ===== impl TcpStream =====

impl TlsInfoFactory for TcpStream {
    fn tls_info(&self) -> Option<TlsInfo> {
        None
    }
}

#[cfg(feature = "boring")]
impl TlsInfoFactory for SslStream<TcpStream> {
    #[inline]
    fn tls_info(&self) -> Option<TlsInfo> {
        Some(extract_tls_info(self))
    }
}

#[cfg(feature = "rustls-tls")]
impl TlsInfoFactory for TlsStream<TcpStream> {
    #[inline]
    fn tls_info(&self) -> Option<TlsInfo> {
        self.get_ref().1.peer_certificates().map(|certs| TlsInfo {
            peer_certificate: certs.first().map(|c| Bytes::from(c.to_vec())),
            peer_certificate_chain: Some(certs.iter().map(|c| Bytes::from(c.to_vec())).collect()),
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
    #[inline]
    fn tls_info(&self) -> Option<TlsInfo> {
        Some(extract_tls_info(self))
    }
}

#[cfg(feature = "rustls-tls")]
impl TlsInfoFactory for TlsStream<MaybeHttpsStream<TcpStream>> {
    #[inline]
    fn tls_info(&self) -> Option<TlsInfo> {
        self.get_ref().1.peer_certificates().map(|certs| TlsInfo {
            peer_certificate: certs.first().map(|c| Bytes::from(c.to_vec())),
            peer_certificate_chain: Some(certs.iter().map(|c| Bytes::from(c.to_vec())).collect()),
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
    #[inline]
    fn tls_info(&self) -> Option<TlsInfo> {
        Some(extract_tls_info(self))
    }
}

#[cfg(all(unix, feature = "rustls-tls"))]
impl TlsInfoFactory for TlsStream<UnixStream> {
    #[inline]
    fn tls_info(&self) -> Option<TlsInfo> {
        self.get_ref().1.peer_certificates().map(|certs| TlsInfo {
            peer_certificate: certs.first().map(|c| Bytes::from(c.to_vec())),
            peer_certificate_chain: Some(certs.iter().map(|c| Bytes::from(c.to_vec())).collect()),
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
    #[inline]
    fn tls_info(&self) -> Option<TlsInfo> {
        Some(extract_tls_info(self))
    }
}

#[cfg(all(unix, feature = "rustls-tls"))]
impl TlsInfoFactory for TlsStream<MaybeHttpsStream<UnixStream>> {
    #[inline]
    fn tls_info(&self) -> Option<TlsInfo> {
        self.get_ref().1.peer_certificates().map(|certs| TlsInfo {
            peer_certificate: certs.first().map(|c| Bytes::from(c.to_vec())),
            peer_certificate_chain: Some(certs.iter().map(|c| Bytes::from(c.to_vec())).collect()),
        })
    }
}
