//! QUIC TLS path: build `quinn::ClientConfig` and `quinn::Endpoint` from
//! `TlsOptions` + `Http3Options`.
//!
//! This module is the single entry point for converting hpx's TLS + HTTP/3
//! option structs into the `quinn` primitives required by `QuicConnector`
//! (T1.4). Per [C-01] and [C-19], ALPN is forced to `b"h3"` regardless of
//! `TlsOptions::alpn_protocols`, satisfying RFC 9114's single-ALPN rule for
//! HTTP/3. Per [C-23], the `AlpnProtocol::HTTP3` constant is reused as the
//! single source of truth for the ALPN bytes.
//!
//! # Constraints (RFC 2119)
//!
//! - **[C-01]** ALPN MUST be `b"h3"` (no draft versions like `h3-29`).
//! - **[C-03]** TLS config MUST use `rustls` (no BoringSSL on the QUIC path).
//! - **[C-19]** `enable_0rtt` MUST map to `rustls::ClientConfig::enable_early_data`.
//! - **[C-23]** ALPN constant MUST be `AlpnProtocol::HTTP3` (single source of truth).

use std::{fmt::Write as _, net::SocketAddr, sync::Arc};

use quinn::congestion::{BbrConfig, CubicConfig};
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};

use crate::{
    client::http3::Http3Options,
    error::Error,
    tls::{AlpnProtocol, CertStore, Identity, KeyLog, TlsOptions, TlsVersion},
};

/// Build a `quinn::ClientConfig` from `TlsOptions` + `Http3Options`, using
/// the `webpki-root-certs` CA bundle as the trust anchor.
///
/// ALPN is forced to `[b"h3"]` per [C-01] and [C-23]. `enable_0rtt` is mapped
/// to `rustls::ClientConfig::enable_early_data` per [C-19].
///
/// Use [`build_quinn_client_config_with_root_store`] when a custom root store
/// is required (e.g., for self-signed cert integration tests).
pub fn build_quinn_client_config(
    tls_opts: &TlsOptions,
    h3_opts: &Http3Options,
) -> crate::Result<quinn::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
    build_quinn_client_config_with_root_store(tls_opts, h3_opts, root_store)
}

/// Build a `quinn::ClientConfig` from `TlsOptions` + `Http3Options` + a
/// caller-supplied `RootCertStore`.
///
/// ALPN is forced to `[b"h3"]` per [C-01] and [C-23]. `enable_0rtt` is mapped
/// to `rustls::ClientConfig::enable_early_data` per [C-19].
///
/// The `rustls::ClientConfig` is constructed via
/// `ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))`
/// to avoid the CryptoProvider ambiguity between `aws-lc-rs` (rustls default)
/// and `ring` (hpx explicit feature) that would otherwise cause
/// `ClientConfig::builder()` to panic with "Could not automatically determine
/// the process-level CryptoProvider".
///
/// The `tls_opts` parameter is accepted for future extension (SNI, keylog,
/// min/max version) but not currently used; ALPN is forced to `b"h3"` per
/// [C-01] regardless of `TlsOptions::alpn_protocols`.
pub fn build_quinn_client_config_with_root_store(
    _tls_opts: &TlsOptions,
    h3_opts: &Http3Options,
    root_store: rustls::RootCertStore,
) -> crate::Result<quinn::ClientConfig> {
    // Per [C-01] and [C-23]: ALPN is forced to `b"h3"` regardless of
    // `TlsOptions::alpn_protocols`.
    let alpn_protocols = vec![AlpnProtocol::HTTP3.as_wire_bytes().to_vec()];

    // Per [C-03]: TLS config uses rustls. Use `builder_with_provider` with an
    // explicit `ring` provider to avoid the CryptoProvider ambiguity panic.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls_config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(Error::tls)?
        .with_root_certificates(root_store)
        .with_no_client_auth();
    tls_config.alpn_protocols = alpn_protocols;

    // Per [C-19]: `enable_0rtt` maps to `enable_early_data`.
    if h3_opts.enable_0rtt {
        tls_config.enable_early_data = true;
    }

    let quic_client_config =
        quinn::crypto::rustls::QuicClientConfig::try_from(Arc::new(tls_config))
            .map_err(Error::tls)?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_client_config));
    client_config.transport_config(Arc::new(build_transport_config(h3_opts)?));
    Ok(client_config)
}

/// Build a `quinn::Endpoint` bound to `local_addr`.
///
/// Wraps `quinn::Endpoint::client` so callers (e.g., `QuicConnector`) have a
/// single, error-mapping entry point for constructing client-side QUIC
/// endpoints. The endpoint uses the default `quinn::EndpointConfig`.
pub fn build_quinn_endpoint(local_addr: SocketAddr) -> crate::Result<quinn::Endpoint> {
    quinn::Endpoint::client(local_addr).map_err(Error::tls)
}

/// Build a `quinn::TransportConfig` from `Http3Options`.
///
/// Maps the QUIC transport parameter fields of [`Http3Options`] to the
/// corresponding `quinn::TransportConfig` setters:
///
/// | `Http3Options` field            | `quinn::TransportConfig` setter      |
/// |---------------------------------|--------------------------------------|
/// | `max_idle_timeout`              | `max_idle_timeout(Some(VarInt))` (ms)|
/// | `stream_receive_window`         | `stream_receive_window(u64)`         |
/// | `conn_receive_window`           | `receive_window(u64)`                |
/// | `send_window`                   | `send_window(u64)`                   |
/// | `max_concurrent_bidi_streams`   | `max_concurrent_bidi_streams(u32)`   |
/// | `max_concurrent_uni_streams`    | `max_concurrent_uni_streams(u32)`    |
/// | `initial_max_data`              | `receive_window(u64)` (overrides conn_receive_window) |
/// | `initial_max_stream_data_*`     | `stream_receive_window(u64)` (overrides stream_receive_window) |
/// | `congestion_bbr`                | `congestion_controller_factory(BbrConfig)` vs `CubicConfig`    |
///
/// `active_connection_id_limit` is managed internally by quinn (not
/// configurable on `TransportConfig`). `max_udp_payload_size` is set on
/// [`quinn::EndpointConfig`] via [`build_quinn_endpoint`].
pub(crate) fn build_transport_config(
    h3_opts: &Http3Options,
) -> crate::Result<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();

    // Congestion control: BBR is preferred for lower latency under lossy or
    // congested networks (at the cost of more aggressive throughput). CUBIC is
    // the quinn default and is used when `congestion_bbr` is `false`.
    let congestion_factory: Arc<dyn quinn::congestion::ControllerFactory + Send + Sync> =
        if h3_opts.congestion_bbr {
            Arc::new(BbrConfig::default())
        } else {
            Arc::new(CubicConfig::default())
        };
    transport.congestion_controller_factory(congestion_factory);

    if let Some(idle) = h3_opts.max_idle_timeout {
        // quinn expects milliseconds as a `VarInt`-backed `IdleTimeout`.
        let ms = idle.as_millis();
        let ms = u64::try_from(ms).map_err(Error::tls)?;
        let varint = quinn::VarInt::from_u64(ms).map_err(Error::tls)?;
        transport.max_idle_timeout(Some(varint.into()));
    }

    if let Some(wnd) = h3_opts.stream_receive_window {
        let varint = quinn::VarInt::from_u64(wnd).map_err(Error::tls)?;
        transport.stream_receive_window(varint);
    }
    if let Some(wnd) = h3_opts.conn_receive_window {
        let varint = quinn::VarInt::from_u64(wnd).map_err(Error::tls)?;
        transport.receive_window(varint);
    }
    if let Some(wnd) = h3_opts.send_window {
        transport.send_window(wnd);
    }
    if let Some(n) = h3_opts.max_concurrent_bidi_streams {
        let n = u32::try_from(n).map_err(Error::tls)?;
        transport.max_concurrent_bidi_streams(n.into());
    }
    if let Some(n) = h3_opts.max_concurrent_uni_streams {
        let n = u32::try_from(n).map_err(Error::tls)?;
        transport.max_concurrent_uni_streams(n.into());
    }

    // Fingerprint-level QUIC transport parameters. These override the
    // generic window settings above when set (e.g., by browser emulation).
    // `initial_max_data` maps to `receive_window` (connection-level flow control).
    if let Some(wnd) = h3_opts.initial_max_data {
        let varint = quinn::VarInt::from_u64(wnd).map_err(Error::tls)?;
        transport.receive_window(varint);
    }
    // `initial_max_stream_data_*` all map to `stream_receive_window`
    // (per-stream flow control). Use `initial_max_stream_data_bidi_local`
    // as the canonical source; the other two are typically identical.
    if let Some(wnd) = h3_opts.initial_max_stream_data_bidi_local {
        let varint = quinn::VarInt::from_u64(wnd).map_err(Error::tls)?;
        transport.stream_receive_window(varint);
    }

    Ok(transport)
}

/// TLS material resolved from the client's `TlsConfigOptions` for the QUIC
/// (h3) path.
///
/// `client/http/builder` owns the user-facing `TlsConfigOptions` and lowers
/// it into this backend-neutral snapshot before calling
/// [`default_quic_connector`] (the `TlsConfigOptions` path is not visible to
/// `tls::quic`). Every field applied here mirrors the TCP TLS-stack behavior:
///
/// - `cert_store` provides the trust anchors (custom store or system store).
/// - `identity` is presented as client authentication when set.
/// - `cert_verification == false` installs a verifier that accepts any
///   server certificate chain, matching the TCP path's `NoVerifier`.
/// - `keylog` routes TLS secrets to the keylog file for debugging.
/// - `tls_sni` toggles the SNI extension.
/// - `min_version` / `max_version` must not contradict QUIC's mandatory
///   TLS 1.3 (RFC 9001 §4.10); a `max_version` below TLS 1.3 is rejected.
#[derive(Clone, Debug)]
pub struct QuicTlsSettings {
    /// Trust anchors derived from the client's cert store.
    pub(crate) cert_store: CertStore,
    /// Client authentication identity, if any.
    pub(crate) identity: Option<Identity>,
    /// Whether server certificates are verified at all.
    pub(crate) cert_verification: bool,
    /// Keylog policy for TLS secret logging, if enabled.
    pub(crate) keylog: Option<KeyLog>,
    /// Whether the SNI extension is sent.
    pub(crate) tls_sni: bool,
    /// Lower TLS version bound requested by the client.
    pub(crate) min_version: Option<TlsVersion>,
    /// Upper TLS version bound requested by the client.
    pub(crate) max_version: Option<TlsVersion>,
}

impl QuicTlsSettings {
    /// Creates a new [`QuicTlsSettings`] from the client's TLS configuration.
    ///
    /// Argument order follows the `TlsConfigOptions` field order
    /// (`cert_store`, `identity`, `cert_verification`, `keylog`, `tls_sni`,
    /// `min_version`, `max_version`).
    pub(crate) fn new(
        cert_store: CertStore,
        identity: Option<Identity>,
        cert_verification: bool,
        keylog: Option<KeyLog>,
        tls_sni: bool,
        min_version: Option<TlsVersion>,
        max_version: Option<TlsVersion>,
    ) -> Self {
        Self {
            cert_store,
            identity,
            cert_verification,
            keylog,
            tls_sni,
            min_version,
            max_version,
        }
    }
}

/// Build a production [`QuicConnector`](crate::http3::QuicConnector) from
/// HTTP/3 options, the client's resolved TLS settings (and an optional
/// low-level transport override).
///
/// This is the non-test construction path used by `Client::build`
/// whenever the `http3` feature is enabled, so `http3_only()` and
/// `prefer_http3()` both have a live QUIC connector. The client's TLS trust
/// configuration (custom CAs, disabled verification, client auth, keylog,
/// SNI, version bounds) is honored — see [`QuicTlsSettings`].
pub fn default_quic_connector(
    h3_opts: &Http3Options,
    tls_settings: QuicTlsSettings,
    transport_override: Option<quinn::TransportConfig>,
    #[cfg(all(feature = "socks", feature = "http3"))] socks5: Option<crate::http3::Socks5Proxy>,
) -> crate::Result<crate::http3::QuicConnector> {
    let tls_config = Arc::new(quic_rustls_client_config(&tls_settings, h3_opts)?);

    let transport_config = Arc::new(match transport_override {
        Some(cfg) => cfg,
        None => build_transport_config(h3_opts)?,
    });

    let endpoint = build_quinn_endpoint(std::net::SocketAddr::from(([0, 0, 0, 0], 0)))?;
    let connector =
        crate::http3::QuicConnector::new(endpoint, transport_config, tls_config, h3_opts.clone());

    #[cfg(all(feature = "socks", feature = "http3"))]
    let connector = match socks5 {
        Some(proxy) => connector.with_socks5_proxy(proxy),
        None => connector,
    };

    Ok(connector)
}

/// Build the rustls [`rustls::ClientConfig`] for QUIC from the client's TLS
/// settings.
///
/// Mirrors the TCP rustls connector's semantics (`tls/rustls.rs`): custom or
/// system trust anchors, `dangerous` verifier when verification is disabled,
/// optional client auth, ALPN forced to `h3`, SNI toggle, keylog, and 0-RTT
/// via `enable_early_data`.
///
/// QUIC handshakes are TLS 1.3-only (RFC 9001 §4.10) and rustls's `quic`
/// module only implements that handshake, so the version set is always
/// TLS 1.3. A `max_version` bound below TLS 1.3 makes a successful QUIC
/// handshake impossible and is rejected up front.
fn quic_rustls_client_config(
    settings: &QuicTlsSettings,
    h3_opts: &Http3Options,
) -> crate::Result<rustls::ClientConfig> {
    // RFC 9001 §4.10: QUIC handshakes are TLS 1.3 only, and rustls's `quic`
    // module only implements the 1.3 handshake. Version bounds therefore
    // cannot be applied literally; reject bounds that would make a
    // successful QUIC handshake impossible instead of failing obscurely in
    // the middle of a connection attempt.
    if let Some(max) = settings.max_version
        && max != TlsVersion::TLS_1_3
    {
        return Err(Error::tls(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "QUIC requires TLS 1.3: max_version must be TLS_1_3 (or unset)",
        )));
    }
    if let Some(min) = settings.min_version
        && min != TlsVersion::TLS_1_2
        && min != TlsVersion::TLS_1_3
    {
        return Err(Error::tls(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "QUIC requires TLS 1.3: min_version must be TLS_1_2, TLS_1_3, or unset",
        )));
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(QUIC_PROTOCOL_VERSIONS)
        .map_err(Error::tls)?;

    let builder = if settings.cert_verification {
        builder.with_root_certificates(root_store_from(&settings.cert_store))
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerifier))
    };

    let mut config = match &settings.identity {
        None => builder.with_no_client_auth(),
        Some(identity) => {
            let (chain, key) = identity.client_auth_der()?;
            let certs: Vec<CertificateDer<'static>> =
                chain.into_iter().map(CertificateDer::from).collect();
            let key = PrivateKeyDer::try_from(key).map_err(Error::tls)?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(Error::tls)?
        }
    };

    // Per [C-01] and [C-23]: ALPN is forced to `b"h3"`.
    config.alpn_protocols = vec![AlpnProtocol::HTTP3.as_wire_bytes().to_vec()];
    config.enable_sni = settings.tls_sni;
    // Per [C-19]: `enable_0rtt` maps to `enable_early_data`.
    if h3_opts.enable_0rtt {
        config.enable_early_data = true;
    }
    if let Some(policy) = &settings.keylog {
        config.key_log = Arc::new(QuicKeyLogBridge {
            handle: policy.clone().handle().map_err(Error::tls)?,
        });
    }
    Ok(config)
}

/// TLS 1.3 is mandatory for QUIC (RFC 9001 §4.10).
const QUIC_PROTOCOL_VERSIONS: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS13];

/// Derives the QUIC trust anchors from the client's [`CertStore`].
///
/// The rustls backend already holds a `rustls::RootCertStore`, so it is
/// reused verbatim: the QUIC path then trusts exactly the same anchors as the
/// TCP path (system/webpki defaults plus the user's own roots).
///
/// The BoringSSL/OpenSSL backends keep their anchors in an `X509Store` that
/// cannot be read back without touching the unsound `X509Store::objects()`
/// accessors, so the DER bytes recorded while the store was built are
/// replayed instead. A store without explicit roots falls back to the webpki
/// anchors, matching the historical QUIC path behavior.
fn root_store_from(store: &CertStore) -> rustls::RootCertStore {
    #[cfg(all(feature = "rustls-tls", not(feature = "boring-tls")))]
    {
        return (*store.store).clone();
    }

    #[cfg(not(all(feature = "rustls-tls", not(feature = "boring-tls"))))]
    {
        let mut root_store = rustls::RootCertStore::empty();
        if store.der_roots.is_empty() {
            root_store.add_parsable_certificates(
                webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned(),
            );
        } else {
            root_store.add_parsable_certificates(
                store
                    .der_roots
                    .iter()
                    .map(|der| CertificateDer::from(der.clone())),
            );
        }
        root_store
    }
}

/// Bridges hpx's [`KeyLog`] policy into rustls's key-log trait so that TLS
/// secrets reach the keylog file on the QUIC path too.
#[derive(Debug)]
struct QuicKeyLogBridge {
    handle: crate::tls::keylog::Handle,
}

impl rustls::KeyLog for QuicKeyLogBridge {
    fn log(&self, label: &str, client_random: &[u8], secret: &[u8]) {
        let mut line = String::new();
        let _ = write!(&mut line, "{label} ");
        for byte in client_random {
            let _ = write!(&mut line, "{byte:02x}");
        }
        let _ = write!(&mut line, " ");
        for byte in secret {
            let _ = write!(&mut line, "{byte:02x}");
        }
        self.handle.write(&line);
    }
}

/// A verifier that accepts every server certificate chain, matching the TCP
/// rustls path's `NoVerifier` for `cert_verification == false`.
#[derive(Debug)]
struct SkipServerVerifier;

impl ServerCertVerifier for SkipServerVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "keylog")]
    use rustls::KeyLog as _;

    use super::*;

    const CA_CERT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/ca.crt");

    fn empty_store() -> CertStore {
        CertStore::builder().build().expect("empty cert store")
    }

    fn settings(cert_store: CertStore) -> QuicTlsSettings {
        QuicTlsSettings::new(cert_store, None, true, None, true, None, None)
    }

    #[test]
    fn quic_root_store_honors_client_cert_store() {
        // Regression: the QUIC path used to ignore the client's cert store
        // and always trust the bundled webpki anchors, so a user-provided CA
        // could never be used for HTTP/3.
        let pem = std::fs::read(CA_CERT_PATH).expect("read ca cert");
        let store = CertStore::builder()
            .add_pem_cert(&pem)
            .build()
            .expect("build store");
        let root_store = root_store_from(&store);
        assert_eq!(
            root_store.roots.len(),
            1,
            "the client's custom CA must be the only trust anchor"
        );
    }

    #[cfg(not(all(feature = "rustls-tls", not(feature = "boring-tls"))))]
    #[test]
    fn quic_root_store_falls_back_to_webpki_anchors() {
        // On the BoringSSL/OpenSSL backends the default (system) store carries
        // no explicit roots, so the QUIC path keeps using the bundled anchors.
        let root_store = root_store_from(&empty_store());
        assert!(
            !root_store.roots.is_empty(),
            "webpki anchors must back the default store"
        );
    }

    #[test]
    fn quic_config_forces_h3_alpn() {
        let config = quic_rustls_client_config(&settings(empty_store()), &Http3Options::default())
            .expect("client config");
        assert_eq!(config.alpn_protocols, vec![b"h3".to_vec()]);
    }

    #[test]
    #[cfg(feature = "keylog")]
    fn quic_keylog_bridge_writes_the_nss_key_log_format() {
        // The bridge must forward rustls' secrets in the NSS key log format
        // (`<label> <client_random> <secret>`), otherwise the key log file is
        // useless for debugging.
        let dir = std::env::temp_dir().join(format!("hpx-quic-keylog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("keylog.txt");
        let keylog = crate::tls::KeyLog::from_file(&path);
        let bridge = QuicKeyLogBridge {
            handle: keylog.handle().expect("keylog handle"),
        };

        bridge.log("CLIENT_RANDOM", &[0x01, 0xab], &[0xcd, 0xef]);

        // The key log handle writes through a background thread, so the line
        // may not be on disk yet.
        let mut contents = String::new();
        for _ in 0..100 {
            contents = std::fs::read_to_string(&path).unwrap_or_default();
            if contents.contains("CLIENT_RANDOM 01ab cdef") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            contents.contains("CLIENT_RANDOM 01ab cdef"),
            "unexpected keylog contents: {contents}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skip_verifier_accepts_any_certificate() {
        // The verifier is only installed when the user explicitly disabled
        // certificate verification, so it must accept every chain and
        // advertise the signature schemes rustls needs.
        let verifier = SkipServerVerifier;
        let cert = CertificateDer::from(vec![0x30, 0x00]);
        let name = ServerName::try_from("example.com").expect("server name");

        let decision = verifier.verify_server_cert(&cert, &[], &name, &[], UnixTime::now());
        assert!(
            decision.is_ok(),
            "the skip verifier must accept any certificate: {decision:?}"
        );
        assert!(
            !verifier.supported_verify_schemes().is_empty(),
            "the verifier must advertise signature schemes"
        );
    }

    #[test]
    fn quic_config_rejects_tls12_max_version() {
        // RFC 9001 §4.10 mandates TLS 1.3 for QUIC; a TLS 1.2 upper bound
        // would make every handshake fail, so it is rejected up front.
        let settings = QuicTlsSettings::new(
            empty_store(),
            None,
            true,
            None,
            true,
            None,
            Some(TlsVersion::TLS_1_2),
        );
        let err = quic_rustls_client_config(&settings, &Http3Options::default())
            .expect_err("TLS 1.2 upper bound must be rejected");
        assert!(
            err.to_string().contains("TLS 1.3"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn quic_config_rejects_unsupported_min_version() {
        let settings = QuicTlsSettings::new(
            empty_store(),
            None,
            true,
            None,
            true,
            Some(TlsVersion::TLS_1_1),
            None,
        );
        let err = quic_rustls_client_config(&settings, &Http3Options::default())
            .expect_err("unsupported lower bound must be rejected");
        assert!(
            err.to_string().contains("TLS 1.3"),
            "unexpected error: {err}"
        );
    }
}
