//! TLS provider caching.
//!
//! Caches constructed TLS configurations to avoid redundant work when
//! the same fingerprint is used repeatedly.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use hpx::tls::TlsOptions;

use super::TlsFingerprint;

static TLS_CACHE: LazyLock<Mutex<HashMap<TlsFingerprint, TlsOptions>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Gets a cached `TlsOptions` for the given fingerprint, or constructs and caches it.
///
/// The `build` closure is called only on cache miss.
pub fn get_or_build_tls(
    fingerprint: &TlsFingerprint,
    build: impl FnOnce(&TlsFingerprint) -> TlsOptions,
) -> TlsOptions {
    {
        let cache = TLS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = cache.get(fingerprint) {
            return cached.clone();
        }
    }

    let options = build(fingerprint);

    {
        let mut cache = TLS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(fingerprint.clone(), options.clone());
    }

    options
}

/// Clears the TLS configuration cache.
///
/// Useful for testing or when fingerprint data changes at runtime.
pub fn clear_tls_cache() {
    let mut cache = TLS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.clear();
}

/// Returns the number of cached TLS configurations.
pub fn tls_cache_len() -> usize {
    let cache = TLS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::{CertCompression, CipherSuite, Curve, EchMode, SignatureAlgorithm};

    fn make_test_fingerprint() -> TlsFingerprint {
        TlsFingerprint {
            curves: vec![Curve::X25519, Curve::Secp256r1],
            cipher_suites: vec![CipherSuite::Tls13Aes128GcmSha256],
            signature_algorithms: vec![SignatureAlgorithm::EcdsaSecp256r1Sha256],
            permute_extensions: false,
            ech_mode: EchMode::Disabled,
            pre_shared_key: false,
            cert_compression: vec![CertCompression::Brotli],
            alps_use_new_codepoint: false,
        }
    }

    #[test]
    fn test_cache_hit() {
        clear_tls_cache();
        let fp = make_test_fingerprint();

        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count_clone = call_count.clone();

        let _opt1 = get_or_build_tls(&fp, |_fp| {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            TlsOptions::default()
        });

        let count_clone = call_count.clone();
        let _opt2 = get_or_build_tls(&fp, |_fp| {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            TlsOptions::default()
        });

        // Build closure should only be called once (cache hit on second call)
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(tls_cache_len(), 1);
    }

    #[test]
    fn test_clear_cache() {
        let fp = make_test_fingerprint();
        let _opt = get_or_build_tls(&fp, |_| TlsOptions::default());
        assert!(tls_cache_len() > 0);
        clear_tls_cache();
        assert_eq!(tls_cache_len(), 0);
    }
}
