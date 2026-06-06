//! TLS provider caching.
//!
//! Caches constructed TLS configurations to avoid redundant work when
//! the same fingerprint is used repeatedly.

use std::{collections::HashMap, sync::LazyLock};

use hpx::tls::TlsOptions;
use parking_lot::Mutex;

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
        let cache = TLS_CACHE.lock();
        if let Some(cached) = cache.get(fingerprint) {
            return cached.clone();
        }
    }

    let options = build(fingerprint);

    {
        let mut cache = TLS_CACHE.lock();
        cache.insert(fingerprint.clone(), options.clone());
    }

    options
}

/// Clears the TLS configuration cache.
///
/// Useful for testing or when fingerprint data changes at runtime.
pub fn clear_tls_cache() {
    let mut cache = TLS_CACHE.lock();
    cache.clear();
}

/// Returns the number of cached TLS configurations.
pub fn tls_cache_len() -> usize {
    let cache = TLS_CACHE.lock();
    cache.len()
}
