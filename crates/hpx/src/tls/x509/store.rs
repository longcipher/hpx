#![allow(unused)]
use std::{fmt::Debug, sync::Arc};

#[cfg(feature = "boring")]
use boring2::{
    ssl::SslConnectorBuilder,
    x509::store::{X509Store, X509StoreBuilder},
};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use rustls::RootCertStore;

#[cfg(feature = "boring")]
use super::parser::{parse_certs, parse_certs_with_stack};
use super::{
    Certificate, CertificateInput,
    parser::{filter_map_certs, process_certs},
};
use crate::{Error, Result};

/// A builder for constructing a `CertStore`.
pub struct CertStoreBuilder {
    #[cfg(feature = "boring")]
    builder: Result<X509StoreBuilder>,
    #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
    builder: Result<RootCertStore>,
}

// ====== impl CertStoreBuilder ======

impl CertStoreBuilder {
    /// Adds a DER-encoded certificate to the certificate store.
    #[inline]
    pub fn add_der_cert<'c, C>(self, cert: C) -> Self
    where
        C: Into<CertificateInput<'c>>,
    {
        self.parse_cert(cert, Certificate::from_der)
    }

    /// Adds a PEM-encoded certificate to the certificate store.
    #[inline]
    pub fn add_pem_cert<'c, C>(self, cert: C) -> Self
    where
        C: Into<CertificateInput<'c>>,
    {
        self.parse_cert(cert, Certificate::from_pem)
    }

    /// Adds multiple DER-encoded certificates to the certificate store.
    #[inline]
    pub fn add_der_certs<'c, I>(self, certs: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<CertificateInput<'c>>,
    {
        self.parse_certs(certs, Certificate::from_der)
    }

    /// Adds multiple PEM-encoded certificates to the certificate store.
    #[inline]
    pub fn add_pem_certs<'c, I>(self, certs: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<CertificateInput<'c>>,
    {
        self.parse_certs(certs, Certificate::from_pem)
    }

    /// Adds a PEM-encoded certificate stack to the certificate store.
    pub fn add_stack_pem_certs<C>(mut self, certs: C) -> Self
    where
        C: AsRef<[u8]>,
    {
        if let Ok(ref mut builder) = self.builder {
            #[cfg(feature = "boring")]
            {
                let result = Certificate::stack_from_pem(certs.as_ref())
                    .and_then(|certs| process_certs(certs.into_iter(), builder));

                if let Err(err) = result {
                    self.builder = Err(err);
                }
            }
            #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
            {
                let result = Certificate::stack_from_pem(certs.as_ref())
                    .and_then(|certs| process_certs(certs.into_iter(), builder));

                if let Err(err) = result {
                    self.builder = Err(err);
                }
            }
        }
        self
    }

    /// Load certificates from their default locations.
    ///
    /// These locations are read from the `SSL_CERT_FILE` and `SSL_CERT_DIR`
    /// environment variables if present, or defaults specified at OpenSSL
    /// build time otherwise.
    pub fn set_default_paths(mut self) -> Self {
        if let Ok(ref mut builder) = self.builder {
            #[cfg(feature = "boring")]
            if let Err(err) = builder.set_default_paths() {
                self.builder = Err(Error::tls(err));
            }
            #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
            {
                // Rustls doesn't have a direct equivalent to set_default_paths that loads from env vars automatically
                // in the same way OpenSSL does, but we can use rustls-native-certs or webpki-roots.
                // For now, we might just ignore or use webpki-roots if enabled.
                // The user prompt mentioned webpki-roots.
                builder.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }
        }
        self
    }

    /// Constructs the `CertStore`.
    ///
    /// This method finalizes the builder and constructs the `CertStore`
    /// containing all the added certificates.
    #[inline]
    pub fn build(self) -> Result<CertStore> {
        #[cfg(feature = "boring")]
        return self
            .builder
            .map(X509StoreBuilder::build)
            .map(Arc::new)
            .map(CertStore);

        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        return self.builder.map(Arc::new).map(CertStore);
    }

    fn parse_cert<'c, C, F>(mut self, cert: C, parser: F) -> Self
    where
        C: Into<CertificateInput<'c>>,
        F: Fn(&'c [u8]) -> Result<Certificate>,
    {
        if let Ok(ref mut builder) = self.builder {
            let cert = cert.into().with_parser(parser);
            let result = cert.and_then(|cert| process_certs(std::iter::once(cert), builder));

            if let Err(err) = result {
                self.builder = Err(err);
            }
        }
        self
    }

    fn parse_certs<'c, I, F>(mut self, certs: I, parser: F) -> Self
    where
        I: IntoIterator,
        I::Item: Into<CertificateInput<'c>>,
        F: Fn(&'c [u8]) -> Result<Certificate>,
    {
        if let Ok(ref mut builder) = self.builder {
            let certs = certs
                .into_iter()
                .map(|cert| cert.into().with_parser(&parser));
            let certs_iter = filter_map_certs(certs);
            let result = process_certs(certs_iter, builder);

            if let Err(err) = result {
                self.builder = Err(err);
            }
        }
        self
    }
}

impl CertStore {
    /// Creates a new `CertStoreBuilder`.
    pub fn builder() -> CertStoreBuilder {
        #[cfg(feature = "boring")]
        let builder = X509StoreBuilder::new().map_err(Error::tls);
        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        let builder = Ok(RootCertStore::empty());

        CertStoreBuilder { builder }
    }
}

impl Default for CertStore {
    fn default() -> Self {
        CertStore::builder().set_default_paths().build().unwrap()
    }
}

/// A certificate store.
#[derive(Clone)]
pub struct CertStore(
    #[cfg(feature = "boring")] pub(crate) Arc<X509Store>,
    #[cfg(all(feature = "rustls-tls", not(feature = "boring")))] pub(crate) Arc<RootCertStore>,
);

impl Debug for CertStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertStore").finish()
    }
}
