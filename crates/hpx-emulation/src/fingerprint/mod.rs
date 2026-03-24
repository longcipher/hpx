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
    X25519,
    X25519Kyber768Draft00,
    X25519MLKEM768,
    Secp256r1,
    Secp384r1,
    Secp521r1,
}

impl Curve {
    /// Returns the OpenSSL/BoringSSL name for this curve.
    pub const fn openssl_name(&self) -> &'static str {
        match self {
            Curve::X25519 => "X25519",
            Curve::X25519Kyber768Draft00 => "X25519Kyber768Draft00",
            Curve::X25519MLKEM768 => "X25519MLKEM768",
            Curve::Secp256r1 => "P-256",
            Curve::Secp384r1 => "P-384",
            Curve::Secp521r1 => "P-521",
        }
    }
}

impl std::fmt::Display for Curve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.openssl_name())
    }
}

/// TLS cipher suites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CipherSuite {
    // TLS 1.3
    Tls13Aes128GcmSha256,
    Tls13Aes256GcmSha384,
    Tls13ChaCha20Poly1305Sha256,
    // TLS 1.2 ECDHE + ECDSA
    EcdheEcdsaWithAes128GcmSha256,
    EcdheEcdsaWithAes256GcmSha384,
    EcdheEcdsaWithChaCha20Poly1305Sha256,
    EcdheEcdsaWithAes128CbcSha,
    EcdheEcdsaWithAes256CbcSha,
    // TLS 1.2 ECDHE + RSA
    EcdheRsaWithAes128GcmSha256,
    EcdheRsaWithAes256GcmSha384,
    EcdheRsaWithChaCha20Poly1305Sha256,
    EcdheRsaWithAes128CbcSha,
    EcdheRsaWithAes256CbcSha,
    // TLS 1.2 RSA
    RsaWithAes128GcmSha256,
    RsaWithAes256GcmSha384,
    RsaWithAes128CbcSha,
    RsaWithAes256CbcSha,
}

impl CipherSuite {
    /// Returns the OpenSSL/BoringSSL name for this cipher suite.
    pub const fn openssl_name(&self) -> &'static str {
        match self {
            CipherSuite::Tls13Aes128GcmSha256 => "TLS_AES_128_GCM_SHA256",
            CipherSuite::Tls13Aes256GcmSha384 => "TLS_AES_256_GCM_SHA384",
            CipherSuite::Tls13ChaCha20Poly1305Sha256 => "TLS_CHACHA20_POLY1305_SHA256",
            CipherSuite::EcdheEcdsaWithAes128GcmSha256 => "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
            CipherSuite::EcdheEcdsaWithAes256GcmSha384 => "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
            CipherSuite::EcdheEcdsaWithChaCha20Poly1305Sha256 => {
                "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256"
            }
            CipherSuite::EcdheEcdsaWithAes128CbcSha => "TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA",
            CipherSuite::EcdheEcdsaWithAes256CbcSha => "TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA",
            CipherSuite::EcdheRsaWithAes128GcmSha256 => "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
            CipherSuite::EcdheRsaWithAes256GcmSha384 => "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
            CipherSuite::EcdheRsaWithChaCha20Poly1305Sha256 => {
                "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256"
            }
            CipherSuite::EcdheRsaWithAes128CbcSha => "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA",
            CipherSuite::EcdheRsaWithAes256CbcSha => "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA",
            CipherSuite::RsaWithAes128GcmSha256 => "TLS_RSA_WITH_AES_128_GCM_SHA256",
            CipherSuite::RsaWithAes256GcmSha384 => "TLS_RSA_WITH_AES_256_GCM_SHA384",
            CipherSuite::RsaWithAes128CbcSha => "TLS_RSA_WITH_AES_128_CBC_SHA",
            CipherSuite::RsaWithAes256CbcSha => "TLS_RSA_WITH_AES_256_CBC_SHA",
        }
    }
}

/// Signature algorithms for TLS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignatureAlgorithm {
    EcdsaSecp256r1Sha256,
    RsaPssRsaeSha256,
    RsaPkcs1Sha256,
    EcdsaSecp384r1Sha384,
    RsaPssRsaeSha384,
    RsaPkcs1Sha384,
    RsaPssRsaeSha512,
    RsaPkcs1Sha512,
}

impl SignatureAlgorithm {
    /// Returns the OpenSSL/BoringSSL name for this algorithm.
    pub const fn openssl_name(&self) -> &'static str {
        match self {
            SignatureAlgorithm::EcdsaSecp256r1Sha256 => "ecdsa_secp256r1_sha256",
            SignatureAlgorithm::RsaPssRsaeSha256 => "rsa_pss_rsae_sha256",
            SignatureAlgorithm::RsaPkcs1Sha256 => "rsa_pkcs1_sha256",
            SignatureAlgorithm::EcdsaSecp384r1Sha384 => "ecdsa_secp384r1_sha384",
            SignatureAlgorithm::RsaPssRsaeSha384 => "rsa_pss_rsae_sha384",
            SignatureAlgorithm::RsaPkcs1Sha384 => "rsa_pkcs1_sha384",
            SignatureAlgorithm::RsaPssRsaeSha512 => "rsa_pss_rsae_sha512",
            SignatureAlgorithm::RsaPkcs1Sha512 => "rsa_pkcs1_sha512",
        }
    }
}

/// Certificate compression algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CertCompression {
    Brotli,
    Zlib,
    Zstd,
}

/// ECH (Encrypted Client Hello) mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EchMode {
    Disabled,
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
    pub fn curves_string(&self) -> String {
        self.curves
            .iter()
            .map(|c| c.openssl_name())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Converts the cipher suites to a colon-separated string.
    pub fn cipher_suites_string(&self) -> String {
        self.cipher_suites
            .iter()
            .map(|c| c.openssl_name())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Converts the signature algorithms to a colon-separated string.
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
            initial_window_size: 6291456,
            initial_connection_window_size: 15728640,
            max_concurrent_streams: Some(1000),
            max_header_list_size: 262144,
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
    pub fn new(
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
