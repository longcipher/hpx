//! Fingerprint-level unit tests.
//!
//! These tests verify specific fingerprint parameters rather than just
//! JA4/Akamai hashes, providing better regression detection.

use hpx_emulation::fingerprint::{
    BrowserFingerprint, CipherSuite, Curve, EchMode, Http2Fingerprint, PseudoHeaderOrder,
    SignatureAlgorithm, TlsPreset, diff_fingerprints, tls_fingerprint_from_preset,
};

/// Helper to build a `BrowserFingerprint` from a `TlsPreset`.
fn fp_from_preset(
    name: &'static str,
    version: &'static str,
    preset: TlsPreset,
) -> BrowserFingerprint {
    let tls = tls_fingerprint_from_preset(preset);
    BrowserFingerprint::new(name, version, tls, Http2Fingerprint::default(), vec![])
}

#[test]
fn test_chrome_base_has_standard_curves() {
    let fp = fp_from_preset("chrome", "100", TlsPreset::ChromeBase);
    assert_eq!(fp.tls.curves.len(), 3);
    assert_eq!(fp.tls.curves[0], Curve::X25519);
    assert_eq!(fp.tls.curves[1], Curve::Secp256r1);
    assert_eq!(fp.tls.curves[2], Curve::Secp384r1);
}

#[test]
fn test_chrome_base_no_ech() {
    let fp = fp_from_preset("chrome", "100", TlsPreset::ChromeBase);
    assert_eq!(fp.tls.ech_mode, EchMode::Disabled);
    assert!(!fp.tls.permute_extensions);
    assert!(!fp.tls.pre_shared_key);
}

#[test]
fn test_chrome_ech_grease_has_ech() {
    let fp = fp_from_preset("chrome", "105", TlsPreset::ChromeEchGrease);
    assert_eq!(fp.tls.ech_mode, EchMode::Grease);
    assert!(!fp.tls.permute_extensions);
}

#[test]
fn test_chrome_permute_has_permutation() {
    let fp = fp_from_preset("chrome", "106", TlsPreset::ChromePermute);
    assert!(fp.tls.permute_extensions);
    assert_eq!(fp.tls.ech_mode, EchMode::Disabled);
}

#[test]
fn test_chrome_permute_ech_has_both() {
    let fp = fp_from_preset("chrome", "116", TlsPreset::ChromePermuteEch);
    assert!(fp.tls.permute_extensions);
    assert_eq!(fp.tls.ech_mode, EchMode::Grease);
}

#[test]
fn test_chrome_permute_ech_psk_has_all() {
    let fp = fp_from_preset("chrome", "117", TlsPreset::ChromePermuteEchPsk);
    assert!(fp.tls.permute_extensions);
    assert_eq!(fp.tls.ech_mode, EchMode::Grease);
    assert!(fp.tls.pre_shared_key);
}

#[test]
fn test_chrome_kyber_has_post_quantum_curves() {
    let fp = fp_from_preset("chrome", "124", TlsPreset::ChromeKyber);
    assert_eq!(fp.tls.curves[0], Curve::X25519Kyber768Draft00);
    assert_eq!(fp.tls.curves.len(), 4);
}

#[test]
fn test_chrome_mlkem768_has_new_codepoint() {
    let fp = fp_from_preset("chrome", "131", TlsPreset::ChromeMlkem768);
    assert_eq!(fp.tls.curves[0], Curve::X25519MLKEM768);
    assert!(fp.tls.alps_use_new_codepoint);
    assert!(fp.tls.pre_shared_key);
}

#[test]
fn test_chrome_has_correct_cipher_count() {
    let fp = fp_from_preset("chrome", "133", TlsPreset::ChromeBase);
    // Chrome always has 15 cipher suites
    assert_eq!(fp.tls.cipher_suites.len(), 15);
    assert!(
        fp.tls
            .cipher_suites
            .contains(&CipherSuite::Tls13Aes128GcmSha256)
    );
    assert!(
        fp.tls
            .cipher_suites
            .contains(&CipherSuite::Tls13Aes256GcmSha384)
    );
    assert!(
        fp.tls
            .cipher_suites
            .contains(&CipherSuite::Tls13ChaCha20Poly1305Sha256)
    );
}

#[test]
fn test_chrome_has_correct_signature_algorithms() {
    let fp = fp_from_preset("chrome", "133", TlsPreset::ChromeBase);
    assert_eq!(fp.tls.signature_algorithms.len(), 8);
    assert_eq!(
        fp.tls.signature_algorithms[0],
        SignatureAlgorithm::EcdsaSecp256r1Sha256
    );
}

#[test]
fn test_default_http2_settings() {
    let h2 = Http2Fingerprint::default();
    assert_eq!(h2.initial_window_size, 6291456);
    assert_eq!(h2.initial_connection_window_size, 15728640);
    assert_eq!(h2.max_concurrent_streams, Some(1000));
    assert_eq!(h2.max_header_list_size, 262144);
    assert_eq!(h2.header_table_size, 65536);
    assert_eq!(
        h2.pseudo_header_order,
        PseudoHeaderOrder::MethodAuthoritySchemePath
    );
}

#[test]
fn test_fingerprint_equality() {
    let a = fp_from_preset("chrome", "133", TlsPreset::ChromeMlkem768);
    let b = fp_from_preset("chrome", "133", TlsPreset::ChromeMlkem768);
    assert_eq!(a, b);
}

#[test]
fn test_fingerprint_inequality_different_version() {
    let a = fp_from_preset("chrome", "132", TlsPreset::ChromeMlkem768);
    let b = fp_from_preset("chrome", "133", TlsPreset::ChromeMlkem768);
    assert_ne!(a, b);
}

#[test]
fn test_fingerprint_inequality_different_preset() {
    let a = fp_from_preset("chrome", "133", TlsPreset::ChromeBase);
    let b = fp_from_preset("chrome", "133", TlsPreset::ChromeMlkem768);
    assert_ne!(a, b);
}

#[test]
fn test_curves_string() {
    let fp = fp_from_preset("chrome", "124", TlsPreset::ChromeKyber);
    let s = fp.tls.curves_string();
    assert_eq!(s, "X25519Kyber768Draft00:X25519:P-256:P-384");
}

#[test]
fn test_cipher_suites_string() {
    let fp = fp_from_preset("chrome", "100", TlsPreset::ChromeBase);
    let s = fp.tls.cipher_suites_string();
    assert!(s.starts_with("TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384"));
    assert!(s.contains("TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA"));
}

#[test]
fn test_signature_algorithms_string() {
    let fp = fp_from_preset("chrome", "100", TlsPreset::ChromeBase);
    let s = fp.tls.signature_algorithms_string();
    assert_eq!(
        s,
        "ecdsa_secp256r1_sha256:rsa_pss_rsae_sha256:rsa_pkcs1_sha256:ecdsa_secp384r1_sha384:rsa_pss_rsae_sha384:rsa_pkcs1_sha384:rsa_pss_rsae_sha512:rsa_pkcs1_sha512"
    );
}

// === Diff tests ===

#[test]
fn test_diff_identical_fingerprints() {
    let a = fp_from_preset("chrome", "133", TlsPreset::ChromeMlkem768);
    let b = fp_from_preset("chrome", "133", TlsPreset::ChromeMlkem768);
    assert!(diff_fingerprints(&a, &b).is_empty());
}

#[test]
fn test_diff_version_change() {
    let a = fp_from_preset("chrome", "132", TlsPreset::ChromeMlkem768);
    let b = fp_from_preset("chrome", "133", TlsPreset::ChromeMlkem768);
    let diffs = diff_fingerprints(&a, &b);
    assert_eq!(diffs.len(), 1);
}

#[test]
fn test_diff_preset_change() {
    let a = fp_from_preset("chrome", "133", TlsPreset::ChromeBase);
    let b = fp_from_preset("chrome", "133", TlsPreset::ChromeMlkem768);
    let diffs = diff_fingerprints(&a, &b);
    // Should detect curves, ech_mode, pre_shared_key, alps_use_new_codepoint changes
    assert!(
        diffs.len() >= 3,
        "Expected at least 3 diffs, got {}",
        diffs.len()
    );
}

#[test]
fn test_diff_curves_change() {
    let a = fp_from_preset("chrome", "133", TlsPreset::ChromeBase);
    let b = fp_from_preset("chrome", "133", TlsPreset::ChromeKyber);
    let diffs = diff_fingerprints(&a, &b);
    assert!(diffs.iter().any(|d| matches!(
        d,
        hpx_emulation::fingerprint::FingerprintDiff::CurvesChanged { .. }
    )));
}
