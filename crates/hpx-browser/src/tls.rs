//! TLS ClientHello fingerprint configurations.
//!
//! Ported from browser_oxide's `net/tls.rs`. Provides per-browser TLS
//! fingerprint presets (Chrome 147, Safari iOS 18, Firefox 135) that
//! produce JA3/JA4-identical ClientHellos when used with hpx's TLS layer.
//!
//! Each preset defines cipher suites, signature algorithms, curves, and
//! extension ordering as string/integer constants. The [`DeviceFingerprint`]
//! struct converts these into hpx's [`TlsOptions`] for actual connection use.

use hpx::tls::TlsOptions;
use rand::prelude::SliceRandom;

use crate::stealth::DeviceClass;

// ---------------------------------------------------------------------------
// Chrome 147 cipher suites (15 ciphers, order is JA3-critical)
// ---------------------------------------------------------------------------

const CHROME_CIPHER_LIST: &str = "TLS_AES_128_GCM_SHA256:\
TLS_AES_256_GCM_SHA384:\
TLS_CHACHA20_POLY1305_SHA256:\
TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256:\
TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256:\
TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384:\
TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384:\
TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256:\
TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256:\
TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA:\
TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA:\
TLS_RSA_WITH_AES_128_GCM_SHA256:\
TLS_RSA_WITH_AES_256_GCM_SHA384:\
TLS_RSA_WITH_AES_128_CBC_SHA:\
TLS_RSA_WITH_AES_256_CBC_SHA";

// ---------------------------------------------------------------------------
// Chrome 147 signature algorithms (8 entries)
// ---------------------------------------------------------------------------

const CHROME_SIGALGS_LIST: &str = "ecdsa_secp256r1_sha256:\
rsa_pss_rsae_sha256:\
rsa_pkcs1_sha256:\
ecdsa_secp384r1_sha384:\
rsa_pss_rsae_sha384:\
rsa_pkcs1_sha384:\
rsa_pss_rsae_sha512:\
rsa_pkcs1_sha512";

// ---------------------------------------------------------------------------
// Chrome 147 curves — MLKEM768 post-quantum (Chrome 131+)
// ---------------------------------------------------------------------------

const CHROME_CURVES_LIST: &str = "X25519MLKEM768:X25519:P-256:P-384";

// ---------------------------------------------------------------------------
// Safari iOS 18 cipher suites (20 ciphers, Apple's order)
// ---------------------------------------------------------------------------

const SAFARI_IOS_CIPHER_LIST: &str = "TLS_AES_128_GCM_SHA256:\
TLS_AES_256_GCM_SHA384:\
TLS_CHACHA20_POLY1305_SHA256:\
TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384:\
TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256:\
TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256:\
TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384:\
TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256:\
TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256:\
TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA:\
TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA:\
TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA:\
TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA:\
TLS_RSA_WITH_AES_256_GCM_SHA384:\
TLS_RSA_WITH_AES_128_GCM_SHA256:\
TLS_RSA_WITH_AES_256_CBC_SHA:\
TLS_RSA_WITH_AES_128_CBC_SHA:\
TLS_ECDHE_ECDSA_WITH_3DES_EDE_CBC_SHA:\
TLS_ECDHE_RSA_WITH_3DES_EDE_CBC_SHA:\
TLS_RSA_WITH_3DES_EDE_CBC_SHA";

// ---------------------------------------------------------------------------
// Safari iOS 18 signature algorithms (10 entries, includes duplicated
// rsa_pss_rsae_sha384 Apple quirk)
// ---------------------------------------------------------------------------

const SAFARI_IOS_SIGALGS_LIST: &str = "ecdsa_secp256r1_sha256:\
rsa_pss_rsae_sha256:\
rsa_pkcs1_sha256:\
ecdsa_secp384r1_sha384:\
rsa_pss_rsae_sha384:\
rsa_pss_rsae_sha384:\
rsa_pkcs1_sha384:\
rsa_pss_rsae_sha512:\
rsa_pkcs1_sha512:\
rsa_pkcs1_sha1";

// ---------------------------------------------------------------------------
// Safari iOS 18 curves — no PQ, includes P-521
// ---------------------------------------------------------------------------

const SAFARI_IOS_CURVES_LIST: &str = "X25519:P-256:P-384:P-521";

// ---------------------------------------------------------------------------
// Firefox 135 cipher suites (17 ciphers, NSS order)
// ---------------------------------------------------------------------------

const FIREFOX_CIPHER_LIST: &str = "TLS_AES_128_GCM_SHA256:\
TLS_CHACHA20_POLY1305_SHA256:\
TLS_AES_256_GCM_SHA384:\
TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256:\
TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256:\
TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256:\
TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256:\
TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384:\
TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384:\
TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA:\
TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA:\
TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA:\
TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA:\
TLS_RSA_WITH_AES_128_GCM_SHA256:\
TLS_RSA_WITH_AES_256_GCM_SHA384:\
TLS_RSA_WITH_AES_128_CBC_SHA:\
TLS_RSA_WITH_AES_256_CBC_SHA";

// ---------------------------------------------------------------------------
// Firefox 135 signature algorithms (11 entries, includes ecdsa_sha1 and
// rsa_pkcs1_sha1 tail)
// ---------------------------------------------------------------------------

const FIREFOX_SIGALGS_LIST: &str = "ecdsa_secp256r1_sha256:\
ecdsa_secp384r1_sha384:\
ecdsa_secp521r1_sha512:\
rsa_pss_rsae_sha256:\
rsa_pss_rsae_sha384:\
rsa_pss_rsae_sha512:\
rsa_pkcs1_sha256:\
rsa_pkcs1_sha384:\
rsa_pkcs1_sha512:\
ecdsa_sha1:\
rsa_pkcs1_sha1";

// ---------------------------------------------------------------------------
// Firefox 135 curves — MLKEM768 PQ + FFDHE groups (Firefox-only)
// ---------------------------------------------------------------------------

const FIREFOX_CURVES_LIST: &str = "X25519MLKEM768:X25519:P-256:P-384:P-521:ffdhe2048:ffdhe3072";

// ---------------------------------------------------------------------------
// Firefox 135 delegated_credentials (ext 0x22) sigalg list
// ---------------------------------------------------------------------------

const FIREFOX_DELEGATED_CREDENTIALS: &str = "ecdsa_secp256r1_sha256:\
ecdsa_secp384r1_sha384:\
ecdsa_secp521r1_sha512:\
ecdsa_sha1";

// ---------------------------------------------------------------------------
// Firefox 135 record_size_limit (ext 0x1c) value: 0x4001 (16385)
// ---------------------------------------------------------------------------

const FIREFOX_RECORD_SIZE_LIMIT: u16 = 0x4001;

// ---------------------------------------------------------------------------
// Extension permutation indices
//
// Indices into BoringSSL's internal `BORING_SSLEXTENSION_PERMUTATION` table.
// 0=server_name, 1=encrypted_client_hello, 2=extended_master_secret,
// 3=renegotiate, 4=supported_groups, 5=ec_point_formats, 6=session_ticket,
// 7=ALPN, 8=status_request, 9=signature_algorithms, 11=certificate_timestamp,
// 14=key_share, 15=psk_key_exchange_modes, 17=supported_versions,
// 21=cert_compression, 22=delegated_credentials, 24=application_settings_new,
// 25=record_size_limit
// ---------------------------------------------------------------------------

/// Chrome 147 extensions (16 total). Fisher-Yates shuffled per handshake.
const CHROME_EXTENSION_PERMUTATION: [u16; 16] = [
    51,    // key_share
    65037, // encrypted_client_hello
    10,    // supported_groups
    18,    // certificate_timestamp
    45,    // psk_key_exchange_modes
    23,    // extended_master_secret
    17613, // application_settings_new
    27,    // cert_compression
    43,    // supported_versions
    0,     // server_name
    65281, // renegotiate
    11,    // ec_point_formats
    5,     // status_request
    16,    // ALPN
    35,    // session_ticket
    13,    // signature_algorithms
];

/// Safari iOS 18 extension order — FIXED (no shuffle). 13 extensions.
const SAFARI_IOS_EXTENSION_PERMUTATION: [u16; 13] = [
    0,     // server_name
    23,    // extended_master_secret
    65281, // renegotiate
    10,    // supported_groups
    11,    // ec_point_formats
    16,    // ALPN
    5,     // status_request
    13,    // signature_algorithms
    18,    // certificate_timestamp
    51,    // key_share
    45,    // psk_key_exchange_modes
    43,    // supported_versions
    27,    // cert_compression
];

/// Firefox 135 extension order — FIXED (no shuffle). 15 extensions.
const FIREFOX_EXTENSION_PERMUTATION: [u16; 15] = [
    0,     // server_name
    23,    // extended_master_secret
    65281, // renegotiation_info
    10,    // supported_groups
    11,    // ec_point_formats
    35,    // session_ticket
    16,    // ALPN
    5,     // status_request
    34,    // delegated_credentials (0x22)
    51,    // key_share
    43,    // supported_versions
    13,    // signature_algorithms
    45,    // psk_key_exchange_modes
    28,    // record_size_limit (0x1c)
    65037, // encrypted_client_hello (ECH grease)
];

// ---------------------------------------------------------------------------
// Certificate compression
// ---------------------------------------------------------------------------

/// Certificate compression algorithm identifiers (RFC 8879).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertCompression {
    Brotli,
    Zlib,
}

// ---------------------------------------------------------------------------
// DeviceFingerprint — per-browser TLS config
// ---------------------------------------------------------------------------

/// Per-browser TLS ClientHello fingerprint configuration.
///
/// Holds the raw TLS parameters needed to produce a JA3/JA4-identical
/// ClientHello. Call [`to_tls_options`](Self::to_tls_options) to convert
/// into hpx's [`TlsOptions`].
#[derive(Debug, Clone)]
pub struct DeviceFingerprint {
    /// Colon-separated cipher suite string for BoringSSL.
    pub cipher_list: &'static str,
    /// Colon-separated signature algorithms string.
    pub sigalgs_list: &'static str,
    /// Colon-separated curves list.
    pub curves_list: &'static str,
    /// Extension type IDs in emission order.
    pub extension_permutation: Vec<u16>,
    /// Certificate compression algorithms.
    pub cert_compression: Vec<CertCompression>,
    /// Minimum TLS version (`"1.0"` or `"1.2"`).
    pub min_tls_version: &'static str,
    /// Enable ECH GREASE.
    pub ech_grease: bool,
    /// Enable extension permutation (Fisher-Yates shuffle).
    pub permute_extensions: bool,
    /// Disable session tickets (Safari iOS quirk).
    pub no_session_ticket: bool,
    /// GREASE enabled (disabled for Firefox).
    pub grease_enabled: bool,
    /// Enable OCSP stapling.
    pub ocsp_stapling: bool,
    /// Enable Signed Certificate Timestamps.
    pub signed_cert_timestamps: bool,
    /// Maximum key shares.
    pub key_shares_limit: u8,
    /// Delegated credentials sigalg list (Firefox-only).
    pub delegated_credentials: Option<&'static str>,
    /// Record size limit (Firefox-only).
    pub record_size_limit: Option<u16>,
    /// Use new ALPS codepoint.
    pub alps_use_new_codepoint: bool,
}

impl DeviceFingerprint {
    /// Convert to hpx's [`TlsOptions`].
    ///
    /// Note: `extension_permutation` is not set here because hpx's
    /// `TlsOptionsBuilder` uses boring's `ExtensionType` which requires
    /// platform-specific conversion. Use [`extension_permutation`](Self::extension_permutation)
    /// to get the raw extension IDs.
    pub fn to_tls_options(&self) -> TlsOptions {
        let min_ver = match self.min_tls_version {
            "1.0" => hpx::tls::TlsVersion::TLS_1_0,
            _ => hpx::tls::TlsVersion::TLS_1_2,
        };

        let mut builder = TlsOptions::builder()
            .cipher_list(self.cipher_list)
            .sigalgs_list(self.sigalgs_list)
            .curves_list(self.curves_list)
            .session_ticket(!self.no_session_ticket)
            .enable_ech_grease(self.ech_grease)
            .permute_extensions(Some(false))
            .grease_enabled(Some(self.grease_enabled))
            .enable_ocsp_stapling(self.ocsp_stapling)
            .enable_signed_cert_timestamps(self.signed_cert_timestamps)
            .key_shares_limit(Some(self.key_shares_limit))
            .alps_use_new_codepoint(self.alps_use_new_codepoint)
            .min_tls_version(min_ver)
            .max_tls_version(hpx::tls::TlsVersion::TLS_1_3);

        if let Some(dc) = self.delegated_credentials {
            builder = builder.delegated_credentials(dc);
        }
        if let Some(limit) = self.record_size_limit {
            builder = builder.record_size_limit(limit);
        }

        builder.build()
    }

    /// Return the extension permutation, shuffled if this is a Chrome profile.
    pub fn get_extension_permutation(&self) -> Vec<u16> {
        if self.permute_extensions {
            let mut rng = rand::rng();
            let mut perm = self.extension_permutation.clone();
            perm.shuffle(&mut rng);
            perm
        } else {
            self.extension_permutation.clone()
        }
    }

    /// Fisher-Yates shuffle of the Chrome extension permutation (for testing).
    pub fn shuffled_chrome_permutation() -> Vec<u16> {
        let mut rng = rand::rng();
        let mut perm = CHROME_EXTENSION_PERMUTATION.to_vec();
        perm.shuffle(&mut rng);
        perm
    }

    /// Return the correct fingerprint for a device class and browser name.
    pub fn for_device(device_class: DeviceClass, browser_name: &str) -> Self {
        if browser_name.eq_ignore_ascii_case("firefox") {
            return Self::firefox_135();
        }
        match device_class {
            DeviceClass::Desktop | DeviceClass::MobileAndroid => Self::chrome_147(),
            DeviceClass::MobileIOS => Self::safari_ios_18(),
        }
    }

    /// Chrome 147 desktop/Android fingerprint.
    pub fn chrome_147() -> Self {
        Self {
            cipher_list: CHROME_CIPHER_LIST,
            sigalgs_list: CHROME_SIGALGS_LIST,
            curves_list: CHROME_CURVES_LIST,
            extension_permutation: CHROME_EXTENSION_PERMUTATION.to_vec(),
            cert_compression: vec![CertCompression::Brotli],
            min_tls_version: "1.2",
            ech_grease: true,
            permute_extensions: true,
            no_session_ticket: false,
            grease_enabled: true,
            ocsp_stapling: true,
            signed_cert_timestamps: true,
            key_shares_limit: 2,
            delegated_credentials: None,
            record_size_limit: None,
            alps_use_new_codepoint: true,
        }
    }

    /// Safari iOS 18 fingerprint.
    pub fn safari_ios_18() -> Self {
        Self {
            cipher_list: SAFARI_IOS_CIPHER_LIST,
            sigalgs_list: SAFARI_IOS_SIGALGS_LIST,
            curves_list: SAFARI_IOS_CURVES_LIST,
            extension_permutation: SAFARI_IOS_EXTENSION_PERMUTATION.to_vec(),
            cert_compression: vec![CertCompression::Zlib],
            min_tls_version: "1.0",
            ech_grease: false,
            permute_extensions: false,
            no_session_ticket: true,
            grease_enabled: true,
            ocsp_stapling: true,
            signed_cert_timestamps: true,
            key_shares_limit: 2,
            delegated_credentials: None,
            record_size_limit: None,
            alps_use_new_codepoint: false,
        }
    }

    /// Firefox 135 (NSS) fingerprint.
    pub fn firefox_135() -> Self {
        Self {
            cipher_list: FIREFOX_CIPHER_LIST,
            sigalgs_list: FIREFOX_SIGALGS_LIST,
            curves_list: FIREFOX_CURVES_LIST,
            extension_permutation: FIREFOX_EXTENSION_PERMUTATION.to_vec(),
            cert_compression: vec![CertCompression::Zlib, CertCompression::Brotli],
            min_tls_version: "1.2",
            ech_grease: true,
            permute_extensions: false,
            no_session_ticket: false,
            grease_enabled: false,
            ocsp_stapling: true,
            signed_cert_timestamps: true,
            key_shares_limit: 2,
            delegated_credentials: Some(FIREFOX_DELEGATED_CREDENTIALS),
            record_size_limit: Some(FIREFOX_RECORD_SIZE_LIMIT),
            alps_use_new_codepoint: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_cipher_list_matches_reference() {
        let expected = "TLS_AES_128_GCM_SHA256:\
            TLS_AES_256_GCM_SHA384:\
            TLS_CHACHA20_POLY1305_SHA256:\
            TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256:\
            TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256:\
            TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384:\
            TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384:\
            TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256:\
            TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256:\
            TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA:\
            TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA:\
            TLS_RSA_WITH_AES_128_GCM_SHA256:\
            TLS_RSA_WITH_AES_256_GCM_SHA384:\
            TLS_RSA_WITH_AES_128_CBC_SHA:\
            TLS_RSA_WITH_AES_256_CBC_SHA";
        assert_eq!(CHROME_CIPHER_LIST, expected);
    }

    #[test]
    fn chrome_sigalgs_matches_reference() {
        let expected = "ecdsa_secp256r1_sha256:\
            rsa_pss_rsae_sha256:\
            rsa_pkcs1_sha256:\
            ecdsa_secp384r1_sha384:\
            rsa_pss_rsae_sha384:\
            rsa_pkcs1_sha384:\
            rsa_pss_rsae_sha512:\
            rsa_pkcs1_sha512";
        assert_eq!(CHROME_SIGALGS_LIST, expected);
    }

    #[test]
    fn chrome_extension_count() {
        assert_eq!(CHROME_EXTENSION_PERMUTATION.len(), 16);
    }

    #[test]
    fn safari_extension_count() {
        assert_eq!(SAFARI_IOS_EXTENSION_PERMUTATION.len(), 13);
    }

    #[test]
    fn firefox_extension_count() {
        assert_eq!(FIREFOX_EXTENSION_PERMUTATION.len(), 15);
    }

    #[test]
    fn safari_has_20_ciphers() {
        assert_eq!(SAFARI_IOS_CIPHER_LIST.matches(':').count() + 1, 20);
    }

    #[test]
    fn firefox_has_17_ciphers() {
        assert_eq!(FIREFOX_CIPHER_LIST.matches(':').count() + 1, 17);
    }

    #[test]
    fn safari_sigalg_has_duplicate_rsa_pss_rsae_sha384() {
        // Apple quirk: rsa_pss_rsae_sha384 appears twice
        let count = SAFARI_IOS_SIGALGS_LIST
            .split(':')
            .filter(|s| *s == "rsa_pss_rsae_sha384")
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn firefox_curves_have_ffdhe() {
        assert!(FIREFOX_CURVES_LIST.contains("ffdhe2048"));
        assert!(FIREFOX_CURVES_LIST.contains("ffdhe3072"));
    }

    #[test]
    fn chrome_curves_have_mlkem768() {
        assert!(CHROME_CURVES_LIST.starts_with("X25519MLKEM768"));
    }

    #[test]
    fn chrome_shuffle_preserves_set() {
        let fp = DeviceFingerprint::chrome_147();
        let p1 = fp.get_extension_permutation();
        let p2 = fp.get_extension_permutation();

        assert_eq!(p1.len(), 16);
        assert_eq!(p2.len(), 16);

        let mut sorted = p1.clone();
        sorted.sort();
        let mut expected = CHROME_EXTENSION_PERMUTATION.to_vec();
        expected.sort();
        assert_eq!(sorted, expected, "shuffle must preserve the set");
        assert_ne!(p1, p2, "shuffle should be non-deterministic");
    }

    #[test]
    fn chrome_preset_to_tls_options() {
        let fp = DeviceFingerprint::chrome_147();
        let opts = fp.to_tls_options();
        assert!(opts.cipher_list.is_some());
        assert!(opts.sigalgs_list.is_some());
        assert!(opts.curves_list.is_some());
        assert!(opts.session_ticket);
        assert!(opts.enable_ech_grease);
        assert_eq!(opts.grease_enabled, Some(true));
        assert_eq!(opts.key_shares_limit, Some(2));
        assert!(opts.alps_use_new_codepoint);
        assert!(opts.delegated_credentials.is_none());
        assert!(opts.record_size_limit.is_none());
        // extension_permutation is available on the fingerprint directly
        assert_eq!(fp.extension_permutation.len(), 16);
    }

    #[test]
    fn safari_preset_to_tls_options() {
        let fp = DeviceFingerprint::safari_ios_18();
        let opts = fp.to_tls_options();
        assert!(!opts.session_ticket);
        assert!(!opts.enable_ech_grease);
        assert_eq!(opts.min_tls_version, Some(hpx::tls::TlsVersion::TLS_1_0));
        assert!(opts.delegated_credentials.is_none());
    }

    #[test]
    fn firefox_preset_to_tls_options() {
        let fp = DeviceFingerprint::firefox_135();
        let opts = fp.to_tls_options();
        assert_eq!(opts.grease_enabled, Some(false));
        assert!(opts.delegated_credentials.is_some());
        assert_eq!(opts.record_size_limit, Some(FIREFOX_RECORD_SIZE_LIMIT));
    }

    #[test]
    fn for_device_dispatches_correctly() {
        let chrome = DeviceFingerprint::for_device(DeviceClass::Desktop, "Chrome");
        assert_eq!(chrome.cipher_list, CHROME_CIPHER_LIST);

        let safari = DeviceFingerprint::for_device(DeviceClass::MobileIOS, "Safari");
        assert_eq!(safari.cipher_list, SAFARI_IOS_CIPHER_LIST);

        let firefox = DeviceFingerprint::for_device(DeviceClass::Desktop, "Firefox");
        assert_eq!(firefox.cipher_list, FIREFOX_CIPHER_LIST);

        // Firefox overrides device class
        let ff_ios = DeviceFingerprint::for_device(DeviceClass::MobileIOS, "Firefox");
        assert_eq!(ff_ios.cipher_list, FIREFOX_CIPHER_LIST);
    }
}
