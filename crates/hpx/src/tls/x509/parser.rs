#![expect(unused)]
#[cfg(feature = "boring-tls")]
use boring::x509::store::{X509Store, X509StoreBuilder};
#[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
use openssl::x509::store::{X509Store, X509StoreBuilder};
#[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
use rustls::RootCertStore;

use super::{Certificate, CertificateInput};
use crate::{Error, Result};

#[cfg(feature = "boring-tls")]
type StoreBuilder = X509StoreBuilder;
#[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
type StoreBuilder = X509StoreBuilder;
#[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
type StoreBuilder = RootCertStore;
#[cfg(not(any(
    feature = "boring-tls",
    feature = "openssl-tls",
    feature = "rustls-tls"
)))]
type StoreBuilder = ();

#[cfg(feature = "boring-tls")]
type StoreType = X509Store;
#[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
type StoreType = X509Store;
#[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
type StoreType = RootCertStore;
#[cfg(not(any(
    feature = "boring-tls",
    feature = "openssl-tls",
    feature = "rustls-tls"
)))]
type StoreType = ();

#[cfg(any(
    feature = "boring-tls",
    feature = "openssl-tls",
    feature = "rustls-tls"
))]
pub(super) fn parse_certs<'c, I>(
    certs: I,
    parser: fn(&'c [u8]) -> crate::Result<Certificate>,
) -> Result<StoreType>
where
    I: IntoIterator,
    I::Item: Into<CertificateInput<'c>>,
{
    #[cfg(feature = "boring-tls")]
    let mut store = X509StoreBuilder::new().map_err(Error::tls)?;
    #[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
    let mut store = X509StoreBuilder::new().map_err(Error::tls)?;
    #[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
    let mut store = RootCertStore::empty();
    #[cfg(not(any(
        feature = "boring-tls",
        feature = "openssl-tls",
        feature = "rustls-tls"
    )))]
    let mut store = ();

    let certs = filter_map_certs(certs.into_iter().map(|c| c.into().with_parser(parser)));
    process_certs(certs, &mut store)?;

    #[cfg(feature = "boring-tls")]
    return Ok(store.build());
    #[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
    return Ok(store.build());
    #[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
    return Ok(store);
    #[cfg(not(any(
        feature = "boring-tls",
        feature = "openssl-tls",
        feature = "rustls-tls"
    )))]
    return Ok(());
}

#[cfg(any(
    feature = "boring-tls",
    feature = "openssl-tls",
    feature = "rustls-tls"
))]
pub(super) fn parse_certs_with_stack<C, F>(certs: C, parse: F) -> Result<StoreType>
where
    C: AsRef<[u8]>,
    F: Fn(C) -> Result<Vec<Certificate>>,
{
    #[cfg(feature = "boring-tls")]
    let mut store = X509StoreBuilder::new().map_err(Error::tls)?;
    #[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
    let mut store = X509StoreBuilder::new().map_err(Error::tls)?;
    #[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
    let mut store = RootCertStore::empty();
    #[cfg(not(any(
        feature = "boring-tls",
        feature = "openssl-tls",
        feature = "rustls-tls"
    )))]
    let mut store = ();

    let certs = parse(certs)?;
    process_certs(certs.into_iter(), &mut store)?;

    #[cfg(feature = "boring-tls")]
    return Ok(store.build());
    #[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
    return Ok(store.build());
    #[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
    return Ok(store);
    #[cfg(not(any(
        feature = "boring-tls",
        feature = "openssl-tls",
        feature = "rustls-tls"
    )))]
    return Ok(());
}

#[cfg(any(
    feature = "boring-tls",
    feature = "openssl-tls",
    feature = "rustls-tls"
))]
pub(super) fn process_certs<I>(iter: I, store: &mut StoreBuilder) -> Result<Vec<Vec<u8>>>
where
    I: Iterator<Item = Certificate>,
{
    let mut valid_count = 0;
    let mut invalid_count = 0;
    let mut der_roots = Vec::new();
    for cert in iter {
        // The DER form must be captured before `cert.0` is moved into the
        // store; `to_der()` failure only means the raw copy is lost.
        #[cfg(feature = "boring-tls")]
        let der = cert.0.to_der().ok();
        #[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
        let der = cert.0.to_der().ok();
        #[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
        let der = Some(cert.0.as_ref().to_vec());
        #[cfg(not(any(
            feature = "boring-tls",
            feature = "openssl-tls",
            feature = "rustls-tls"
        )))]
        let der: Option<Vec<u8>> = None;

        #[cfg(feature = "boring-tls")]
        let res = store.add_cert(cert.0).map_err(Error::tls).map(|()| der);
        #[cfg(all(feature = "openssl-tls", not(feature = "boring-tls")))]
        let res = store.add_cert(cert.0).map_err(Error::tls).map(|()| der);
        #[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
        let res = store
            .add(cert.0.clone())
            .map_err(|e| Error::tls(Box::new(e)))
            .map(|()| der);
        #[cfg(not(any(
            feature = "boring-tls",
            feature = "openssl-tls",
            feature = "rustls-tls"
        )))]
        let res: Result<Option<Vec<u8>>> = Err(Error::tls("TLS not supported"));

        match res {
            Ok(der) => {
                valid_count += 1;
                if let Some(der) = der {
                    der_roots.push(der);
                }
            }
            Err(_err) => {
                invalid_count += 1;
                // warn!("tls failed to parse certificate: {:?}", _err);
            }
        }
    }

    if valid_count == 0 && invalid_count > 0 {
        return Err(Error::builder("invalid certificate"));
    }

    Ok(der_roots)
}

#[cfg(any(
    feature = "boring-tls",
    feature = "openssl-tls",
    feature = "rustls-tls"
))]
pub(super) fn filter_map_certs<I>(certs: I) -> impl Iterator<Item = Certificate>
where
    I: IntoIterator<Item = Result<Certificate>>,
{
    certs.into_iter().filter_map(|res| res.ok())
}
