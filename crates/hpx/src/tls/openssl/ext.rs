use openssl::{
    ssl::{SslConnectorBuilder, SslVerifyMode},
    x509::store::X509StoreBuilder,
};

use crate::{Error, tls::x509::CertStore};

/// SslConnectorBuilderExt trait for `SslConnectorBuilder`.
pub trait SslConnectorBuilderExt {
    /// Configure the CertStore for the given `SslConnectorBuilder`.
    fn configure_cert_store(self, store: Option<&CertStore>) -> crate::Result<SslConnectorBuilder>;

    /// Configure the certificate verification for the given `SslConnectorBuilder`.
    fn set_cert_verification(self, enable: bool) -> crate::Result<SslConnectorBuilder>;
}

impl SslConnectorBuilderExt for SslConnectorBuilder {
    #[inline]
    fn configure_cert_store(
        mut self,
        store: Option<&CertStore>,
    ) -> crate::Result<SslConnectorBuilder> {
        if let Some(store) = store {
            // OpenSSL's X509Store does not implement Clone, so we must rebuild
            // from the certificate list. This is O(n) per connector build but
            // unavoidable with the current openssl crate API.
            let mut builder = X509StoreBuilder::new().map_err(Error::tls)?;
            for cert in store.0.all_certificates() {
                builder.add_cert(cert).map_err(Error::tls)?;
            }
            self.set_verify_cert_store(builder.build())
                .map_err(Error::tls)?;
        } else {
            self.set_default_verify_paths().map_err(Error::tls)?;
        }

        Ok(self)
    }

    #[inline]
    fn set_cert_verification(mut self, enable: bool) -> crate::Result<SslConnectorBuilder> {
        if enable {
            self.set_verify(SslVerifyMode::PEER);
        } else {
            self.set_verify(SslVerifyMode::NONE);
        }
        Ok(self)
    }
}
