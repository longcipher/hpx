#![allow(unused)]
#[cfg(feature = "boring")]
use boring::{
    pkcs12::Pkcs12,
    pkey::{PKey, Private},
    x509::X509,
};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use const_oid::db::{
    rfc5911::{ID_DATA, ID_ENCRYPTED_DATA},
    rfc5912::{ID_SHA_1, ID_SHA_224, ID_SHA_256, ID_SHA_384, ID_SHA_512},
};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use der::{
    Decode, Encode,
    asn1::{ContextSpecific, OctetString},
};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use hmac::{Hmac, KeyInit, Mac};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use pkcs8::{EncryptedPrivateKeyInfoRef, pkcs5::EncryptionScheme};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use pkcs12::{
    AuthenticatedSafe, CertBag, PKCS_12_CERT_BAG_OID, PKCS_12_KEY_BAG_OID,
    PKCS_12_PKCS8_KEY_BAG_OID, PKCS_12_X509_CERT_OID, Pfx,
    safe_bag::{PrivateKeyInfo, SafeBag, SafeContents},
};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use sha1::Sha1;
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
use sha2::{Sha224, Sha256, Sha384, Sha512};

use crate::Error;

/// Represents a private key and X509 cert as a client certificate.
#[derive(Debug, Clone)]
pub struct Identity {
    #[cfg(feature = "boring")]
    pkey: PKey<Private>,
    #[cfg(feature = "boring")]
    cert: X509,
    #[cfg(feature = "boring")]
    chain: Vec<X509>,

    #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
    pub(crate) cert: Vec<CertificateDer<'static>>,
    #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
    pub(crate) key: std::sync::Arc<PrivateKeyDer<'static>>,
    #[cfg(not(any(feature = "boring", feature = "rustls-tls")))]
    _marker: std::marker::PhantomData<()>,
}

impl Identity {
    /// Parses a PEM bundle containing a leaf certificate, optional certificate chain, and the
    /// corresponding private key.
    ///
    /// This is useful for client certificate bundles where the certificate chain and private key
    /// are delivered in a single PEM file.
    pub fn from_pem(buf: &[u8]) -> crate::Result<Identity> {
        #[cfg(feature = "boring")]
        {
            let key = extract_private_key_pem(buf)
                .ok_or_else(|| Error::builder("no private key found"))?;
            let pkey = PKey::private_key_from_pem(key).map_err(Error::tls)?;
            let mut cert_chain = extract_certificate_pems(buf)
                .into_iter()
                .map(|pem| X509::from_pem(pem).map_err(Error::tls))
                .collect::<crate::Result<Vec<_>>>()?
                .into_iter();
            let cert = cert_chain.next().ok_or_else(|| {
                Error::builder("at least one certificate must be provided to create an identity")
            })?;
            let chain = cert_chain.collect();
            Ok(Identity { pkey, cert, chain })
        }
        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        {
            use std::io::Cursor;

            let mut cert_reader = Cursor::new(buf);
            let certs = rustls_pemfile::certs(&mut cert_reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::tls(Box::new(e)))?;

            if certs.is_empty() {
                return Err(Error::builder(
                    "at least one certificate must be provided to create an identity",
                ));
            }

            let mut key_reader = Cursor::new(buf);
            let key = rustls_pemfile::private_key(&mut key_reader)
                .map_err(|e| Error::tls(Box::new(e)))?
                .ok_or_else(|| Error::builder("no private key found"))?;

            Ok(Identity {
                cert: certs,
                key: std::sync::Arc::new(key),
            })
        }
        #[cfg(not(any(feature = "boring", feature = "rustls-tls")))]
        {
            let _ = buf;
            Err(Error::tls("TLS not supported"))
        }
    }

    /// Parses a DER-formatted PKCS #12 archive, using the specified password to decrypt the key.
    ///
    /// The archive should contain a leaf certificate and its private key, as well any intermediate
    /// certificates that allow clients to build a chain to a trusted root.
    /// The chain certificates should be in order from the leaf certificate towards the root.
    ///
    /// PKCS #12 archives typically have the file extension `.p12` or `.pfx`, and can be created
    /// with the OpenSSL `pkcs12` tool:
    ///
    /// ```bash
    /// openssl pkcs12 -export -out identity.pfx -inkey key.pem -in cert.pem -certfile chain_certs.pem
    /// ```
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::fs::File;
    /// # use std::io::Read;
    /// # fn pkcs12() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut buf = Vec::new();
    /// File::open("my-ident.pfx")?.read_to_end(&mut buf)?;
    /// let pkcs12 = hpx::tls::Identity::from_pkcs12_der(&buf, "my-privkey-password")?;
    /// # drop(pkcs12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_pkcs12_der(buf: &[u8], pass: &str) -> crate::Result<Identity> {
        #[cfg(feature = "boring")]
        {
            let pkcs12 = Pkcs12::from_der(buf).map_err(Error::tls)?;
            let parsed = pkcs12.parse(pass).map_err(Error::tls)?;
            Ok(Identity {
                pkey: parsed.pkey,
                cert: parsed.cert,
                // > The stack is the reverse of what you might expect due to the way
                // > PKCS12_parse is implemented, so we need to load it backwards.
                // > https://github.com/sfackler/rust-native-tls/commit/05fb5e583be589ab63d9f83d986d095639f8ec44
                chain: parsed.chain.into_iter().flatten().rev().collect(),
            })
        }
        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        {
            identity_from_pkcs12_archive(buf, pass)
        }
        #[cfg(not(any(feature = "boring", feature = "rustls-tls")))]
        {
            Err(Error::tls("TLS not supported"))
        }
    }

    /// Parses a chain of PEM encoded X509 certificates, with the leaf certificate first.
    /// `key` is a PEM encoded PKCS #8 formatted private key for the leaf certificate.
    ///
    /// The certificate chain should contain any intermediate certificates that should be sent to
    /// clients to allow them to build a chain to a trusted root.
    ///
    /// A certificate chain here means a series of PEM encoded certificates concatenated together.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::fs;
    /// # fn pkcs8() -> Result<(), Box<dyn std::error::Error>> {
    /// let cert = fs::read("client.pem")?;
    /// let key = fs::read("key.pem")?;
    /// let pkcs8 = hpx::tls::Identity::from_pkcs8_pem(&cert, &key)?;
    /// # drop(pkcs8);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_pkcs8_pem(buf: &[u8], key: &[u8]) -> crate::Result<Identity> {
        #[cfg(feature = "boring")]
        {
            if !key.starts_with(b"-----BEGIN PRIVATE KEY-----") {
                return Err(Error::builder("expected PKCS#8 PEM"));
            }

            let pkey = PKey::private_key_from_pem(key).map_err(Error::tls)?;
            let mut cert_chain = X509::stack_from_pem(buf).map_err(Error::tls)?.into_iter();
            let cert = cert_chain.next().ok_or_else(|| {
                Error::builder("at least one certificate must be provided to create an identity")
            })?;
            let chain = cert_chain.collect();
            Ok(Identity { pkey, cert, chain })
        }
        #[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
        {
            use std::io::Cursor;
            let mut cert_reader = Cursor::new(buf);
            let certs = rustls_pemfile::certs(&mut cert_reader)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::tls(Box::new(e)))?;

            let mut key_reader = Cursor::new(key);
            let key = rustls_pemfile::private_key(&mut key_reader)
                .map_err(|e| Error::tls(Box::new(e)))?
                .ok_or_else(|| Error::builder("no private key found"))?;

            Ok(Identity {
                cert: certs,
                key: std::sync::Arc::new(key),
            })
        }
        #[cfg(not(any(feature = "boring", feature = "rustls-tls")))]
        {
            Err(Error::tls("TLS not supported"))
        }
    }

    #[cfg(feature = "boring")]
    pub(crate) fn add_to_tls(
        &self,
        connector: &mut boring::ssl::SslConnectorBuilder,
    ) -> crate::Result<()> {
        connector.set_certificate(&self.cert).map_err(Error::tls)?;
        connector.set_private_key(&self.pkey).map_err(Error::tls)?;
        for cert in self.chain.iter() {
            // https://www.openssl.org/docs/manmaster/man3/SSL_CTX_add_extra_chain_cert.html
            // specifies that "When sending a certificate chain, extra chain certificates are
            // sent in order following the end entity certificate."
            connector
                .add_extra_chain_cert(cert.clone())
                .map_err(Error::tls)?;
        }
        Ok(())
    }
}

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
fn identity_from_pkcs12_archive(buf: &[u8], pass: &str) -> crate::Result<Identity> {
    let archive = Pfx::from_der(buf)
        .map_err(|error| pkcs12_tls_error("failed to parse PKCS12 archive", error))?;
    verify_pkcs12_mac(&archive, pass)?;

    let mut cert = Vec::new();
    let mut key = None;

    for safe_bag in pkcs12_safe_contents(&archive, pass)? {
        if let Some(certificate) = pkcs12_certificate(&safe_bag)? {
            cert.push(certificate);
            continue;
        }

        if key.is_none() {
            key = pkcs12_private_key(&safe_bag, pass)?;
        }
    }

    if cert.is_empty() {
        return Err(Error::builder(
            "at least one certificate must be provided to create an identity",
        ));
    }

    let key = key.ok_or_else(|| Error::builder("no private key found"))?;

    Ok(Identity {
        cert,
        key: std::sync::Arc::new(key),
    })
}

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
fn verify_pkcs12_mac(archive: &Pfx, pass: &str) -> crate::Result<()> {
    let Some(mac_data) = archive.mac_data.as_ref() else {
        return Ok(());
    };

    let auth_safe = pkcs12_data_content(&archive.auth_safe, "PKCS12 authenticated safe")?;
    let expected = mac_data.mac.digest.as_bytes();
    let actual = pkcs12_mac(
        &mac_data.mac.algorithm.oid,
        pass,
        mac_data.mac_salt.as_bytes(),
        mac_data.iterations,
        &auth_safe,
        expected.len(),
    )?;

    if actual.as_slice() != expected {
        return Err(Error::tls(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "PKCS12 archive MAC verification failed",
        )));
    }

    Ok(())
}

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
fn pkcs12_mac(
    algorithm: &const_oid::ObjectIdentifier,
    pass: &str,
    salt: &[u8],
    iterations: i32,
    data: &[u8],
    output_len: usize,
) -> crate::Result<Vec<u8>> {
    macro_rules! compute_mac {
        ($digest:ty) => {{
            let key = pkcs12::kdf::derive_key_utf8::<$digest>(
                pass,
                salt,
                pkcs12::kdf::Pkcs12KeyType::Mac,
                iterations,
                output_len,
            )
            .map_err(|error| pkcs12_tls_error("failed to derive PKCS12 MAC key", error))?;
            let mut mac = Hmac::<$digest>::new_from_slice(&key)
                .map_err(|error| pkcs12_tls_error("failed to initialize PKCS12 MAC", error))?;
            mac.update(data);
            Ok::<Vec<u8>, crate::Error>(mac.finalize().into_bytes().to_vec())
        }};
    }

    if *algorithm == ID_SHA_1 {
        compute_mac!(Sha1)
    } else if *algorithm == ID_SHA_224 {
        compute_mac!(Sha224)
    } else if *algorithm == ID_SHA_256 {
        compute_mac!(Sha256)
    } else if *algorithm == ID_SHA_384 {
        compute_mac!(Sha384)
    } else if *algorithm == ID_SHA_512 {
        compute_mac!(Sha512)
    } else {
        Err(Error::tls(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unsupported PKCS12 MAC algorithm: {algorithm}"),
        )))
    }
}

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
fn pkcs12_safe_contents(archive: &Pfx, pass: &str) -> crate::Result<SafeContents> {
    if archive.auth_safe.content_type != ID_DATA {
        return Err(Error::tls(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unsupported PKCS12 authenticated safe content type: {}",
                archive.auth_safe.content_type
            ),
        )));
    }

    let auth_safe = pkcs12_data_content(&archive.auth_safe, "PKCS12 authenticated safe")?;
    let content_infos = AuthenticatedSafe::from_der(&auth_safe)
        .map_err(|error| pkcs12_tls_error("failed to decode PKCS12 authenticated safe", error))?;

    let mut safe_bags = Vec::new();

    for content_info in content_infos {
        if content_info.content_type == ID_DATA {
            let plaintext = pkcs12_data_content(&content_info, "PKCS12 safe contents")?;
            safe_bags.extend(SafeContents::from_der(&plaintext).map_err(|error| {
                pkcs12_tls_error("failed to decode PKCS12 safe contents", error)
            })?);
            continue;
        }

        if content_info.content_type == ID_ENCRYPTED_DATA {
            let encrypted_data = pkcs12::cms::encrypted_data::EncryptedData::from_der(
                &content_info.content.to_der().map_err(|error| {
                    pkcs12_tls_error("failed to encode PKCS12 encrypted content", error)
                })?,
            )
            .map_err(|error| {
                pkcs12_tls_error("failed to decode PKCS12 encrypted content", error)
            })?;

            let parameters = encrypted_data
                .enc_content_info
                .content_enc_alg
                .parameters
                .as_ref()
                .ok_or_else(|| {
                    Error::tls(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "PKCS12 encrypted content is missing algorithm parameters",
                    ))
                })?
                .to_der()
                .map_err(|error| {
                    pkcs12_tls_error(
                        "failed to encode PKCS12 encrypted content parameters",
                        error,
                    )
                })?;
            let params =
                pkcs8::pkcs5::pbes2::Parameters::from_der(&parameters).map_err(|error| {
                    pkcs12_tls_error(
                        "failed to decode PKCS12 encrypted content parameters",
                        error,
                    )
                })?;
            let encrypted_content = encrypted_data
                .enc_content_info
                .encrypted_content
                .clone()
                .ok_or_else(|| {
                    Error::tls(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "PKCS12 encrypted content is missing ciphertext",
                    ))
                })?;
            let mut ciphertext = encrypted_content.as_bytes().to_vec();
            let plaintext = EncryptionScheme::from(params)
                .decrypt_in_place(pass, &mut ciphertext)
                .map_err(|error| {
                    pkcs12_tls_error("failed to decrypt PKCS12 safe contents", error)
                })?;

            safe_bags.extend(SafeContents::from_der(plaintext).map_err(|error| {
                pkcs12_tls_error("failed to decode decrypted PKCS12 safe contents", error)
            })?);
            continue;
        }

        return Err(Error::tls(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unsupported PKCS12 content type: {}",
                content_info.content_type
            ),
        )));
    }

    Ok(safe_bags)
}

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
fn pkcs12_data_content(
    content_info: &pkcs12::cms::content_info::ContentInfo,
    context: &'static str,
) -> crate::Result<Vec<u8>> {
    let der = content_info
        .content
        .to_der()
        .map_err(|error| pkcs12_tls_error("failed to encode PKCS12 content", error))?;
    let octets = OctetString::from_der(&der).map_err(|error| pkcs12_tls_error(context, error))?;
    Ok(octets.as_bytes().to_vec())
}

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
fn pkcs12_certificate(safe_bag: &SafeBag) -> crate::Result<Option<CertificateDer<'static>>> {
    if safe_bag.bag_id != PKCS_12_CERT_BAG_OID {
        return Ok(None);
    }

    let cert_bag = ContextSpecific::<CertBag>::from_der(&safe_bag.bag_value)
        .map_err(|error| pkcs12_tls_error("failed to decode PKCS12 certificate bag", error))?;

    if cert_bag.value.cert_id != PKCS_12_X509_CERT_OID {
        return Ok(None);
    }

    Ok(Some(CertificateDer::from(
        cert_bag.value.cert_value.as_bytes().to_vec(),
    )))
}

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
fn pkcs12_private_key(
    safe_bag: &SafeBag,
    pass: &str,
) -> crate::Result<Option<PrivateKeyDer<'static>>> {
    if safe_bag.bag_id == PKCS_12_PKCS8_KEY_BAG_OID {
        let encrypted_key =
            ContextSpecific::<EncryptedPrivateKeyInfoRef<'_>>::from_der(&safe_bag.bag_value)
                .map_err(|error| {
                    pkcs12_tls_error("failed to decode PKCS12 encrypted key bag", error)
                })?;
        let mut ciphertext = encrypted_key.value.encrypted_data.as_bytes().to_vec();
        let plaintext = encrypted_key
            .value
            .encryption_algorithm
            .decrypt_in_place(pass, &mut ciphertext)
            .map_err(|error| pkcs12_tls_error("failed to decrypt PKCS12 private key", error))?;

        return Ok(Some(PrivateKeyDer::try_from(plaintext.to_vec()).map_err(
            |error| Error::builder(format!("invalid PKCS12 private key: {error}")),
        )?));
    }

    if safe_bag.bag_id == PKCS_12_KEY_BAG_OID {
        let key_bag = ContextSpecific::<PrivateKeyInfo>::from_der(&safe_bag.bag_value)
            .map_err(|error| pkcs12_tls_error("failed to decode PKCS12 key bag", error))?;

        return Ok(Some(
            PrivateKeyDer::try_from(
                key_bag
                    .value
                    .to_der()
                    .map_err(|error| pkcs12_tls_error("failed to encode PKCS12 key bag", error))?,
            )
            .map_err(|error| Error::builder(format!("invalid PKCS12 private key: {error}")))?,
        ));
    }

    Ok(None)
}

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
fn pkcs12_tls_error(context: &'static str, error: impl std::fmt::Display) -> Error {
    Error::tls(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{context}: {error}"),
    ))
}

#[cfg(feature = "boring")]
fn extract_private_key_pem(buf: &[u8]) -> Option<&[u8]> {
    const KEY_MARKERS: [(&[u8], &[u8]); 3] = [
        (b"-----BEGIN PRIVATE KEY-----", b"-----END PRIVATE KEY-----"),
        (
            b"-----BEGIN RSA PRIVATE KEY-----",
            b"-----END RSA PRIVATE KEY-----",
        ),
        (
            b"-----BEGIN EC PRIVATE KEY-----",
            b"-----END EC PRIVATE KEY-----",
        ),
    ];

    KEY_MARKERS
        .iter()
        .find_map(|(begin, end)| extract_pem_block(buf, begin, end))
}

#[cfg(feature = "boring")]
fn extract_certificate_pems(buf: &[u8]) -> Vec<&[u8]> {
    extract_all_pem_blocks(
        buf,
        b"-----BEGIN CERTIFICATE-----",
        b"-----END CERTIFICATE-----",
    )
}

#[cfg(feature = "boring")]
fn extract_pem_block<'a>(buf: &'a [u8], begin: &[u8], end: &[u8]) -> Option<&'a [u8]> {
    let start = find_subsequence(buf, begin)?;
    let end_start = start + find_subsequence(&buf[start..], end)?;
    let mut end_idx = end_start + end.len();

    if buf.get(end_idx) == Some(&b'\r') {
        end_idx += 1;
    }
    if buf.get(end_idx) == Some(&b'\n') {
        end_idx += 1;
    }

    Some(&buf[start..end_idx])
}

#[cfg(feature = "boring")]
fn extract_all_pem_blocks<'a>(mut buf: &'a [u8], begin: &[u8], end: &[u8]) -> Vec<&'a [u8]> {
    let mut blocks = Vec::new();

    while let Some(start) = find_subsequence(buf, begin) {
        let remainder = &buf[start..];
        let Some(block) = extract_pem_block(remainder, begin, end) else {
            break;
        };
        blocks.push(block);
        buf = &remainder[block.len()..];
    }

    blocks
}

#[cfg(feature = "boring")]
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod test {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::Identity;

    const CLIENT_CERT_PEM: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/mtls/client.crt"
    ));
    const CLIENT_KEY_PEM: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/mtls/client.key"
    ));
    const CLIENT_PKCS12_DER_B64: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/mtls/client.p12.b64"
    ));

    #[test]
    fn identity_from_pkcs12_der_invalid() {
        Identity::from_pkcs12_der(b"not der", "nope").unwrap_err();
    }

    #[test]
    fn identity_from_pkcs8_pem_invalid() {
        Identity::from_pkcs8_pem(b"not pem", b"not key").unwrap_err();
    }

    #[test]
    fn identity_from_pem_invalid() {
        Identity::from_pem(b"not pem").unwrap_err();
    }

    #[test]
    fn identity_from_pem_combined_bundle() {
        let mut pem = CLIENT_CERT_PEM.to_vec();
        pem.extend_from_slice(CLIENT_KEY_PEM);

        Identity::from_pem(&pem).unwrap();
    }

    #[test]
    fn identity_from_pkcs12_der_bundle() {
        let pkcs12 = STANDARD.decode(CLIENT_PKCS12_DER_B64.trim()).unwrap();

        Identity::from_pkcs12_der(&pkcs12, "changeit").unwrap();
    }
}
