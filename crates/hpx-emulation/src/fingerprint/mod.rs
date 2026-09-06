//! Structured browser fingerprint types.
//!
//! This module provides structured, comparable, and serializable representations
//! of browser fingerprints. Each fingerprint captures the TLS ClientHello parameters,
//! HTTP/2 SETTINGS, and HTTP headers that a specific browser version would send.
//!
//! # Design Goals
//!
//! - **Type safety**: Use enums instead of strings for cipher suites, curves, etc.
//! - **Comparability**: Enable `Eq`/`Hash` for fingerprint identity checks
//! - **Testability**: Allow unit tests to verify specific fingerprint parameters
//! - **Extensibility**: Support custom fingerprints alongside predefined ones
//!
//! # Example
//!
//! ```rust
//! use hpx_emulation::{Emulation, fingerprint::BrowserFingerprint};
//!
//! let fp = BrowserFingerprint::from_emulation(Emulation::Chrome133);
//! assert_eq!(fp.name, "chrome");
//! assert_eq!(fp.version, "133");
//! assert!(fp.tls.curves.contains(&Curve::X25519MLKEM768));
//! ```

mod cache;
mod composer;
mod diff;

pub use cache::{clear_tls_cache, get_or_build_tls, tls_cache_len};
pub use composer::HeaderComposer;
pub use diff::{FingerprintDiff, diff_fingerprints};

/// Builds a structured `TlsFingerprint` from a named `TlsPreset`.
///
/// This is the bridge between the opaque `tls_options!(N)` macro system
/// and the new structured fingerprint types. Each preset maps to a
/// specific combination of TLS features (curves, ECH, permutation, etc.).
#[cfg(feature = "emulation")]
pub fn tls_fingerprint_from_preset(preset: TlsPreset) -> TlsFingerprint {
    // Delegate to the emulation layer's implementation
    crate::emulation::device::tls_fingerprint_from_preset(preset)
}

/// Named TLS configuration presets.
///
/// Each preset represents a specific TLS fingerprint configuration commonly
/// used by browser families. These replace the opaque `tls_options!(N)` macro
/// numbers with self-documenting names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TlsPreset {
    /// Base Chrome TLS config: standard curves, no ECH, no permute.
    ChromeBase,
    /// Chrome with ECH GREASE enabled.
    ChromeEchGrease,
    /// Chrome with extension permutation.
    ChromePermute,
    /// Chrome with both ECH GREASE and extension permutation.
    ChromePermuteEch,
    /// Chrome with ECH GREASE, permutation, and PSK.
    ChromePermuteEchPsk,
    /// Chrome with X25519Kyber768Draft00 post-quantum curves.
    ChromeKyber,
    /// Chrome with X25519MLKEM768 post-quantum curves and new ALPS codepoint.
    ChromeMlkem768,
    /// Firefox base TLS config.
    FirefoxBase,
    /// Firefox with ECH GREASE.
    FirefoxEchGrease,
    /// Safari base TLS config.
    SafariBase,
    /// OkHttp base TLS config.
    OkHttpBase,
}

/// Elliptic curves supported for TLS key exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Curve {
    /// X25519 (Curve25519) key exchange.
    X25519,
    /// Hybrid X25519 + Kyber768 draft-00 post-quantum key exchange.
    X25519Kyber768Draft00,
    /// Hybrid X25519 + ML-KEM-768 post-quantum key exchange.
    X25519MLKEM768,
    /// NIST P-256 (secp256r1) curve.
    Secp256r1,
    /// NIST P-384 (secp384r1) curve.
    Secp384r1,
    /// NIST P-521 (secp521r1) curve.
    Secp521r1,
}

impl Curve {
    /// Returns the OpenSSL/BoringSSL name for this curve.
    pub const fn openssl_name(&self) -> &'static str {
        match self {
            Self::X25519 => "X25519",
            Self::X25519Kyber768Draft00 => "X25519Kyber768Draft00",
            Self::X25519MLKEM768 => "X25519MLKEM768",
            Self::Secp256r1 => "P-256",
            Self::Secp384r1 => "P-384",
            Self::Secp521r1 => "P-521",
        }
    }
}

impl std::fmt::Display for Curve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.openssl_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_openssl_name_matches_all_variants() {
        assert_eq!(Curve::X25519.openssl_name(), "X25519");
        assert_eq!(
            Curve::X25519Kyber768Draft00.openssl_name(),
            "X25519Kyber768Draft00"
        );
        assert_eq!(Curve::X25519MLKEM768.openssl_name(), "X25519MLKEM768");
        assert_eq!(Curve::Secp256r1.openssl_name(), "P-256");
        assert_eq!(Curve::Secp384r1.openssl_name(), "P-384");
        assert_eq!(Curve::Secp521r1.openssl_name(), "P-521");
    }

    #[test]
    fn curve_display_roundtrips_openssl_name() {
        for curve in [
            Curve::X25519,
            Curve::X25519Kyber768Draft00,
            Curve::X25519MLKEM768,
            Curve::Secp256r1,
            Curve::Secp384r1,
            Curve::Secp521r1,
        ] {
            assert_eq!(curve.to_string(), curve.openssl_name());
        }
    }

    #[cfg(feature = "emulation")]
    #[test]
    fn tls_presets_map_to_distinct_fingerprints() {
        let base = tls_fingerprint_from_preset(TlsPreset::ChromeBase);
        assert!(!base.permute_extensions, "ChromeBase must not permute");
        assert_eq!(base.ech_mode, EchMode::Disabled);

        let permute = tls_fingerprint_from_preset(TlsPreset::ChromePermute);
        assert!(
            permute.permute_extensions,
            "ChromePermute must permute extensions"
        );

        let ech = tls_fingerprint_from_preset(TlsPreset::ChromeEchGrease);
        assert_eq!(
            ech.ech_mode,
            EchMode::Grease,
            "ChromeEchGrease must use ECH grease"
        );

        let permute_ech = tls_fingerprint_from_preset(TlsPreset::ChromePermuteEch);
        assert!(permute_ech.permute_extensions);
        assert_eq!(permute_ech.ech_mode, EchMode::Grease);

        let kyber = tls_fingerprint_from_preset(TlsPreset::ChromeKyber);
        assert!(
            kyber.permute_extensions,
            "ChromeKyber must permute extensions"
        );
        assert_eq!(
            kyber.ech_mode,
            EchMode::Grease,
            "ChromeKyber must use ECH grease"
        );
        assert!(kyber.pre_shared_key, "ChromeKyber must set PSK");

        let mlkem = tls_fingerprint_from_preset(TlsPreset::ChromeMlkem768);
        assert!(
            mlkem.permute_extensions,
            "ChromeMlkem768 must permute extensions"
        );
        assert_eq!(
            mlkem.ech_mode,
            EchMode::Grease,
            "ChromeMlkem768 must use ECH grease"
        );
        assert!(
            mlkem.alps_use_new_codepoint,
            "ChromeMlkem768 must use new ALPS codepoint"
        );

        let ff_base = tls_fingerprint_from_preset(TlsPreset::FirefoxBase);
        assert_eq!(
            ff_base.ech_mode,
            EchMode::Disabled,
            "FirefoxBase must not use ECH"
        );

        let ff_ech = tls_fingerprint_from_preset(TlsPreset::FirefoxEchGrease);
        assert_eq!(
            ff_ech.ech_mode,
            EchMode::Grease,
            "FirefoxEchGrease must use ECH grease"
        );

        let safari_base = tls_fingerprint_from_preset(TlsPreset::SafariBase);
        assert_eq!(
            safari_base.ech_mode,
            EchMode::Disabled,
            "SafariBase must not use ECH"
        );

        let okhttp_base = tls_fingerprint_from_preset(TlsPreset::OkHttpBase);
        assert_eq!(
            okhttp_base.ech_mode,
            EchMode::Disabled,
            "OkHttpBase must not use ECH"
        );
    }

    #[cfg(feature = "emulation-serde")]
    #[test]
    fn browser_fingerprint_serializes_to_object() {
        let fp = BrowserFingerprint::new(
            "chrome",
            "133",
            TlsFingerprint::default(),
            Http2Fingerprint::default(),
            vec![("user-agent", "test")],
        );
        let value = serde_json::to_value(&fp).expect("BrowserFingerprint must serialize");
        assert!(
            value.is_object(),
            "expected a serialized object, got: {value}"
        );
        assert_eq!(value.get("name").and_then(|v| v.as_str()), Some("chrome"));
        assert_eq!(value.get("version").and_then(|v| v.as_str()), Some("133"));
        assert!(
            value.get("tls_curves").and_then(|v| v.as_str()).is_some(),
            "tls_curves field missing"
        );
    }
}

/// TLS cipher suites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CipherSuite {
    /// TLS 1.3 `TLS_AES_128_GCM_SHA256`.
    Tls13Aes128GcmSha256,
    /// TLS 1.3 `TLS_AES_256_GCM_SHA384`.
    Tls13Aes256GcmSha384,
    /// TLS 1.3 `TLS_CHACHA20_POLY1305_SHA256`.
    Tls13ChaCha20Poly1305Sha256,
    /// TLS 1.2 `ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`.
    EcdheEcdsaWithAes128GcmSha256,
    /// TLS 1.2 `ECDHE_ECDSA_WITH_AES_256_GCM_SHA384`.
    EcdheEcdsaWithAes256GcmSha384,
    /// TLS 1.2 `ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256`.
    EcdheEcdsaWithChaCha20Poly1305Sha256,
    /// TLS 1.2 `ECDHE_ECDSA_WITH_AES_128_CBC_SHA`.
    EcdheEcdsaWithAes128CbcSha,
    /// TLS 1.2 `ECDHE_ECDSA_WITH_AES_256_CBC_SHA`.
    EcdheEcdsaWithAes256CbcSha,
    /// TLS 1.2 `ECDHE_RSA_WITH_AES_128_GCM_SHA256`.
    EcdheRsaWithAes128GcmSha256,
    /// TLS 1.2 `ECDHE_RSA_WITH_AES_256_GCM_SHA384`.
    EcdheRsaWithAes256GcmSha384,
    /// TLS 1.2 `ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256`.
    EcdheRsaWithChaCha20Poly1305Sha256,
    /// TLS 1.2 `ECDHE_RSA_WITH_AES_128_CBC_SHA`.
    EcdheRsaWithAes128CbcSha,
    /// TLS 1.2 `ECDHE_RSA_WITH_AES_256_CBC_SHA`.
    EcdheRsaWithAes256CbcSha,
    /// TLS 1.2 `RSA_WITH_AES_128_GCM_SHA256`.
    RsaWithAes128GcmSha256,
    /// TLS 1.2 `RSA_WITH_AES_256_GCM_SHA384`.
    RsaWithAes256GcmSha384,
    /// TLS 1.2 `RSA_WITH_AES_128_CBC_SHA`.
    RsaWithAes128CbcSha,
    /// TLS 1.2 `RSA_WITH_AES_256_CBC_SHA`.
    RsaWithAes256CbcSha,
}

impl CipherSuite {
    /// Returns the OpenSSL/BoringSSL name for this cipher suite.
    pub const fn openssl_name(&self) -> &'static str {
        match self {
            Self::Tls13Aes128GcmSha256 => "TLS_AES_128_GCM_SHA256",
            Self::Tls13Aes256GcmSha384 => "TLS_AES_256_GCM_SHA384",
            Self::Tls13ChaCha20Poly1305Sha256 => "TLS_CHACHA20_POLY1305_SHA256",
            Self::EcdheEcdsaWithAes128GcmSha256 => "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
            Self::EcdheEcdsaWithAes256GcmSha384 => "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
            Self::EcdheEcdsaWithChaCha20Poly1305Sha256 => {
                "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256"
            }
            Self::EcdheEcdsaWithAes128CbcSha => "TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA",
            Self::EcdheEcdsaWithAes256CbcSha => "TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA",
            Self::EcdheRsaWithAes128GcmSha256 => "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
            Self::EcdheRsaWithAes256GcmSha384 => "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
            Self::EcdheRsaWithChaCha20Poly1305Sha256 => {
                "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256"
            }
            Self::EcdheRsaWithAes128CbcSha => "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA",
            Self::EcdheRsaWithAes256CbcSha => "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA",
            Self::RsaWithAes128GcmSha256 => "TLS_RSA_WITH_AES_128_GCM_SHA256",
            Self::RsaWithAes256GcmSha384 => "TLS_RSA_WITH_AES_256_GCM_SHA384",
            Self::RsaWithAes128CbcSha => "TLS_RSA_WITH_AES_128_CBC_SHA",
            Self::RsaWithAes256CbcSha => "TLS_RSA_WITH_AES_256_CBC_SHA",
        }
    }
}

/// Signature algorithms for TLS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignatureAlgorithm {
    /// `ecdsa_secp256r1_sha256`.
    EcdsaSecp256r1Sha256,
    /// `rsa_pss_rsae_sha256`.
    RsaPssRsaeSha256,
    /// `rsa_pkcs1_sha256`.
    RsaPkcs1Sha256,
    /// `ecdsa_secp384r1_sha384`.
    EcdsaSecp384r1Sha384,
    /// `rsa_pss_rsae_sha384`.
    RsaPssRsaeSha384,
    /// `rsa_pkcs1_sha384`.
    RsaPkcs1Sha384,
    /// `rsa_pss_rsae_sha512`.
    RsaPssRsaeSha512,
    /// `rsa_pkcs1_sha512`.
    RsaPkcs1Sha512,
}

impl SignatureAlgorithm {
    /// Returns the OpenSSL/BoringSSL name for this algorithm.
    pub const fn openssl_name(&self) -> &'static str {
        match self {
            Self::EcdsaSecp256r1Sha256 => "ecdsa_secp256r1_sha256",
            Self::RsaPssRsaeSha256 => "rsa_pss_rsae_sha256",
            Self::RsaPkcs1Sha256 => "rsa_pkcs1_sha256",
            Self::EcdsaSecp384r1Sha384 => "ecdsa_secp384r1_sha384",
            Self::RsaPssRsaeSha384 => "rsa_pss_rsae_sha384",
            Self::RsaPkcs1Sha384 => "rsa_pkcs1_sha384",
            Self::RsaPssRsaeSha512 => "rsa_pss_rsae_sha512",
            Self::RsaPkcs1Sha512 => "rsa_pkcs1_sha512",
        }
    }
}

/// Certificate compression algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CertCompression {
    /// Brotli certificate compression.
    Brotli,
    /// Zlib certificate compression.
    Zlib,
    /// Zstandard certificate compression.
    Zstd,
}

/// ECH (Encrypted Client Hello) mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EchMode {
    /// ECH is not offered.
    Disabled,
    /// ECH is offered with a GREASE placeholder only.
    Grease,
}

/// TLS fingerprint specification.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TlsFingerprint {
    /// Ordered list of elliptic curves.
    pub curves: Vec<Curve>,
    /// Ordered list of cipher suites.
    pub cipher_suites: Vec<CipherSuite>,
    /// Ordered list of signature algorithms.
    pub signature_algorithms: Vec<SignatureAlgorithm>,
    /// Whether to permute ClientHello extensions.
    pub permute_extensions: bool,
    /// ECH mode.
    pub ech_mode: EchMode,
    /// Whether to enable PSK (pre-shared key).
    pub pre_shared_key: bool,
    /// Certificate compression algorithms.
    pub cert_compression: Vec<CertCompression>,
    /// Whether to use the new ALPS codepoint (Chrome 132+).
    pub alps_use_new_codepoint: bool,
}

impl Default for TlsFingerprint {
    fn default() -> Self {
        Self {
            curves: vec![Curve::X25519, Curve::Secp256r1, Curve::Secp384r1],
            cipher_suites: vec![
                CipherSuite::Tls13Aes128GcmSha256,
                CipherSuite::Tls13Aes256GcmSha384,
                CipherSuite::Tls13ChaCha20Poly1305Sha256,
                CipherSuite::EcdheEcdsaWithAes128GcmSha256,
                CipherSuite::EcdheRsaWithAes128GcmSha256,
                CipherSuite::EcdheEcdsaWithAes256GcmSha384,
                CipherSuite::EcdheRsaWithAes256GcmSha384,
                CipherSuite::EcdheEcdsaWithChaCha20Poly1305Sha256,
                CipherSuite::EcdheRsaWithChaCha20Poly1305Sha256,
                CipherSuite::EcdheRsaWithAes128CbcSha,
                CipherSuite::EcdheRsaWithAes256CbcSha,
                CipherSuite::RsaWithAes128GcmSha256,
                CipherSuite::RsaWithAes256GcmSha384,
                CipherSuite::RsaWithAes128CbcSha,
                CipherSuite::RsaWithAes256CbcSha,
            ],
            signature_algorithms: vec![
                SignatureAlgorithm::EcdsaSecp256r1Sha256,
                SignatureAlgorithm::RsaPssRsaeSha256,
                SignatureAlgorithm::RsaPkcs1Sha256,
                SignatureAlgorithm::EcdsaSecp384r1Sha384,
                SignatureAlgorithm::RsaPssRsaeSha384,
                SignatureAlgorithm::RsaPkcs1Sha384,
                SignatureAlgorithm::RsaPssRsaeSha512,
                SignatureAlgorithm::RsaPkcs1Sha512,
            ],
            permute_extensions: false,
            ech_mode: EchMode::Disabled,
            pre_shared_key: false,
            cert_compression: vec![CertCompression::Brotli],
            alps_use_new_codepoint: false,
        }
    }
}

impl TlsFingerprint {
    /// Converts the curves list to a colon-separated string for BoringSSL/OpenSSL.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn curves_string(&self) -> String {
        self.curves
            .iter()
            .map(|c| c.openssl_name())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Converts the cipher suites to a colon-separated string.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn cipher_suites_string(&self) -> String {
        self.cipher_suites
            .iter()
            .map(|c| c.openssl_name())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Converts the signature algorithms to a colon-separated string.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn signature_algorithms_string(&self) -> String {
        self.signature_algorithms
            .iter()
            .map(|a| a.openssl_name())
            .collect::<Vec<_>>()
            .join(":")
    }
}

/// HTTP/2 fingerprint specification.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Http2Fingerprint {
    /// SETTINGS frame: initial stream window size.
    pub initial_window_size: u32,
    /// SETTINGS frame: initial connection window size.
    pub initial_connection_window_size: u32,
    /// SETTINGS frame: max concurrent streams.
    pub max_concurrent_streams: Option<u32>,
    /// SETTINGS frame: max header list size.
    pub max_header_list_size: u32,
    /// SETTINGS frame: header table size.
    pub header_table_size: u32,
    /// SETTINGS frame: enable push.
    pub enable_push: Option<bool>,
    /// Pseudo-header order in HEADERS frame.
    pub pseudo_header_order: PseudoHeaderOrder,
}

/// HTTP/2 pseudo-header ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PseudoHeaderOrder {
    /// :method, :authority, :scheme, :path (Chrome default)
    MethodAuthoritySchemePath,
}

impl Default for Http2Fingerprint {
    fn default() -> Self {
        Self {
            initial_window_size: 6_291_456,
            initial_connection_window_size: 15_728_640,
            max_concurrent_streams: Some(1000),
            max_header_list_size: 262_144,
            header_table_size: 65536,
            enable_push: None,
            pseudo_header_order: PseudoHeaderOrder::MethodAuthoritySchemePath,
        }
    }
}

/// A complete browser fingerprint.
///
/// Contains all the information needed to replicate a specific browser's
/// TLS ClientHello, HTTP/2 SETTINGS, and HTTP request headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserFingerprint {
    /// Browser name (e.g., "chrome", "firefox", "safari").
    pub name: &'static str,
    /// Browser version string (e.g., "133", "146").
    pub version: &'static str,
    /// TLS ClientHello fingerprint.
    pub tls: TlsFingerprint,
    /// HTTP/2 SETTINGS fingerprint.
    pub http2: Http2Fingerprint,
    /// Default HTTP headers by OS.
    pub headers: Vec<(&'static str, &'static str)>,
}

#[cfg(feature = "emulation-serde")]
impl serde::Serialize for BrowserFingerprint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("BrowserFingerprint", 5)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("version", &self.version)?;
        state.serialize_field("tls_curves", &self.tls.curves_string())?;
        state.serialize_field("tls_cipher_suites", &self.tls.cipher_suites_string())?;
        state.serialize_field(
            "tls_signature_algorithms",
            &self.tls.signature_algorithms_string(),
        )?;
        state.serialize_field("tls_permute_extensions", &self.tls.permute_extensions)?;
        state.serialize_field("tls_ech_mode", &format!("{:?}", self.tls.ech_mode))?;
        state.serialize_field("tls_pre_shared_key", &self.tls.pre_shared_key)?;
        state.serialize_field(
            "tls_alps_use_new_codepoint",
            &self.tls.alps_use_new_codepoint,
        )?;
        state.serialize_field("h2_initial_window_size", &self.http2.initial_window_size)?;
        state.serialize_field(
            "h2_max_concurrent_streams",
            &self.http2.max_concurrent_streams,
        )?;
        state.serialize_field("h2_max_header_list_size", &self.http2.max_header_list_size)?;
        state.serialize_field("h2_header_table_size", &self.http2.header_table_size)?;
        state.end()
    }
}

impl BrowserFingerprint {
    /// Creates a new `BrowserFingerprint`.
    pub const fn new(
        name: &'static str,
        version: &'static str,
        tls: TlsFingerprint,
        http2: Http2Fingerprint,
        headers: Vec<(&'static str, &'static str)>,
    ) -> Self {
        Self {
            name,
            version,
            tls,
            http2,
            headers,
        }
    }
}
