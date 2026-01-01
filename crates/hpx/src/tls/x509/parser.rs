#![allow(unused)]
#[cfg(feature = "boring")]
use boring::x509::store::{X509Store, X509StoreBuilder};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use rustls::RootCertStore;

use super::{Certificate, CertificateInput};
use crate::{Error, Result};

#[cfg(feature = "boring")]
type StoreBuilder = X509StoreBuilder;
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
type StoreBuilder = RootCertStore;

#[cfg(feature = "boring")]
type StoreType = X509Store;
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
type StoreType = RootCertStore;

pub fn parse_certs<'c, I>(
    certs: I,
    parser: fn(&'c [u8]) -> crate::Result<Certificate>,
) -> Result<StoreType>
where
    I: IntoIterator,
    I::Item: Into<CertificateInput<'c>>,
{
    #[cfg(feature = "boring")]
    let mut store = X509StoreBuilder::new().map_err(Error::tls)?;
    #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
    let mut store = RootCertStore::empty();

    let certs = filter_map_certs(certs.into_iter().map(|c| c.into().with_parser(parser)));
    process_certs(certs, &mut store)?;

    #[cfg(feature = "boring")]
    return Ok(store.build());
    #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
    return Ok(store);
}

pub fn parse_certs_with_stack<C, F>(certs: C, parse: F) -> Result<StoreType>
where
    C: AsRef<[u8]>,
    F: Fn(C) -> Result<Vec<Certificate>>,
{
    #[cfg(feature = "boring")]
    let mut store = X509StoreBuilder::new().map_err(Error::tls)?;
    #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
    let mut store = RootCertStore::empty();

    let certs = parse(certs)?;
    process_certs(certs.into_iter(), &mut store)?;

    #[cfg(feature = "boring")]
    return Ok(store.build());
    #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
    return Ok(store);
}

pub fn process_certs<I>(iter: I, store: &mut StoreBuilder) -> Result<()>
where
    I: Iterator<Item = Certificate>,
{
    let mut valid_count = 0;
    let mut invalid_count = 0;
    for cert in iter {
        #[cfg(feature = "boring")]
        let res = store.add_cert(cert.0).map_err(Error::tls);
        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        let res = store
            .add(cert.0.clone())
            .map_err(|e| Error::tls(Box::new(e)));

        if let Err(_err) = res {
            invalid_count += 1;
            // warn!("tls failed to parse certificate: {:?}", _err);
        } else {
            valid_count += 1;
        }
    }

    if valid_count == 0 && invalid_count > 0 {
        return Err(Error::builder("invalid certificate"));
    }

    Ok(())
}

pub fn filter_map_certs<I>(certs: I) -> impl Iterator<Item = Certificate>
where
    I: IntoIterator<Item = Result<Certificate>>,
{
    certs.into_iter().filter_map(|res| res.ok())
}
