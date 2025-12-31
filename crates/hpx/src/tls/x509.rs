mod identity;
mod parser;
mod store;

#[cfg(feature = "boring")]
use boring2::x509::X509;
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use rustls_pki_types::CertificateDer;

pub use self::{
    identity::Identity,
    store::{CertStore, CertStoreBuilder},
};
use crate::Error;

/// A certificate input.
pub enum CertificateInput<'c> {
    /// Raw DER or PEM data.
    Raw(&'c [u8]),
    /// An already parsed certificate.
    Parsed(Certificate),
}

impl<'a> CertificateInput<'a> {
    pub(crate) fn with_parser<F>(self, parser: F) -> crate::Result<Certificate>
    where
        F: Fn(&'a [u8]) -> crate::Result<Certificate>,
    {
        match self {
            CertificateInput::Raw(data) => parser(data),
            CertificateInput::Parsed(cert) => Ok(cert),
        }
    }
}

impl From<Certificate> for CertificateInput<'_> {
    fn from(cert: Certificate) -> Self {
        CertificateInput::Parsed(cert)
    }
}

impl<'c, T: AsRef<[u8]> + ?Sized + 'c> From<&'c T> for CertificateInput<'c> {
    fn from(value: &'c T) -> CertificateInput<'c> {
        CertificateInput::Raw(value.as_ref())
    }
}

/// A certificate.
#[cfg(feature = "boring")]
/// An X509 certificate.
#[derive(Clone)]
pub struct Certificate(X509);

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
/// An X509 certificate.
#[derive(Clone, Debug)]
pub struct Certificate(pub(crate) CertificateDer<'static>);

impl Certificate {
    /// Parse a certificate from DER data.
    #[inline]
    pub fn from_der<C: AsRef<[u8]>>(cert: C) -> crate::Result<Self> {
        #[cfg(feature = "boring")]
        {
            X509::from_der(cert.as_ref()).map(Self).map_err(Error::tls)
        }
        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        {
            Ok(Self(CertificateDer::from(cert.as_ref().to_vec())))
        }
    }

    /// Parse a certificate from PEM data.
    #[inline]
    pub fn from_pem<C: AsRef<[u8]>>(cert: C) -> crate::Result<Self> {
        #[cfg(feature = "boring")]
        {
            X509::from_pem(cert.as_ref()).map(Self).map_err(Error::tls)
        }
        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        {
            use std::io::Cursor;
            let mut reader = Cursor::new(cert.as_ref());
            let certs = rustls_pemfile::certs(&mut reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::tls(Box::new(e)))?;

            certs
                .into_iter()
                .next()
                .map(Self)
                .ok_or_else(|| Error::tls("No certificate found in PEM"))
        }
    }

    /// Parse a stack of certificates from DER data.
    #[inline]
    pub fn stack_from_pem<C: AsRef<[u8]>>(cert: C) -> crate::Result<Vec<Self>> {
        #[cfg(feature = "boring")]
        {
            let certs = X509::stack_from_pem(cert.as_ref()).map_err(Error::tls)?;
            Ok(certs.into_iter().map(Self).collect())
        }
        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        {
            use std::io::Cursor;
            let mut reader = Cursor::new(cert.as_ref());
            let certs = rustls_pemfile::certs(&mut reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::tls(Box::new(e)))?;

            Ok(certs.into_iter().map(Self).collect())
        }
    }
}
