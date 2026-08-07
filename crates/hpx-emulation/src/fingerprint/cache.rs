//! TLS provider caching.
//!
//! Caches constructed TLS configurations to avoid redundant work when
//! the same fingerprint is used repeatedly.

use std::sync::{Arc, LazyLock};

use ahash::AHashMap;
use arc_swap::ArcSwap;
use hpx::tls::TlsOptions;

use super::TlsFingerprint;

/// Caches constructed TLS configurations keyed by fingerprint.
///
/// Uses `ArcSwap` with copy-on-write semantics so the read path (cache hit)
/// is fully lock-free: `load()` returns a shared snapshot without touching any
/// mutex. Inserts on a cache miss clone the map and publish it through `rcu`,
/// which is O(n) but only runs on the rare slow path (new fingerprint). The
/// set of distinct TLS fingerprints is small (a handful of browser presets),
/// so the clone cost is negligible.
static TLS_CACHE: LazyLock<ArcSwap<AHashMap<TlsFingerprint, TlsOptions>>> =
    LazyLock::new(|| ArcSwap::from_pointee(AHashMap::new()));

/// Gets a cached `TlsOptions` for the given fingerprint, or constructs and caches it.
///
/// The `build` closure is called only on cache miss.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_or_build_tls(
    fingerprint: &TlsFingerprint,
    build: impl FnOnce(&TlsFingerprint) -> TlsOptions,
) -> TlsOptions {
    // Fast path: lock-free read of the shared snapshot.
    let guard = TLS_CACHE.load();
    if let Some(cached) = guard.get(fingerprint) {
        return cached.clone();
    }

    // Slow path: build outside any lock, then publish via copy-on-write rcu.
    // Multiple threads may race here and build independently; the rcu loop
    // retries until it installs its snapshot (last writer wins). The slow path
    // is rare (new fingerprint), so the O(n) map clone is acceptable.
    let options = build(fingerprint);
    TLS_CACHE.rcu(|current| {
        let mut new_map = (**current).clone();
        new_map.insert(fingerprint.clone(), options.clone());
        Arc::new(new_map)
    });
    options
}

/// Clears the TLS configuration cache.
///
/// Useful for testing or when fingerprint data changes at runtime.
pub fn clear_tls_cache() {
    TLS_CACHE.store(Arc::new(AHashMap::new()));
}

/// Returns the number of cached TLS configurations.
pub fn tls_cache_len() -> usize {
    TLS_CACHE.load().len()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn get_or_build_tls_caches_builds_and_clears() {
        clear_tls_cache();
        assert_eq!(tls_cache_len(), 0, "cache must start empty");

        let fp1 = TlsFingerprint::default();
        let mut fp2 = fp1.clone();
        fp2.pre_shared_key = true; // distinct fingerprint key

        let calls = AtomicUsize::new(0);
        let build = |_: &TlsFingerprint| {
            calls.fetch_add(1, Ordering::SeqCst);
            TlsOptions::default()
        };

        // First build populates the cache.
        let a = get_or_build_tls(&fp1, build);
        // Cache hit: the build closure must not run again.
        let b = get_or_build_tls(&fp1, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            TlsOptions::default()
        });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a cached fingerprint must not rebuild"
        );

        // A distinct fingerprint must build again.
        get_or_build_tls(&fp2, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            TlsOptions::default()
        });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a distinct fingerprint must trigger a new build"
        );
        assert_eq!(tls_cache_len(), 2, "two distinct fingerprints cached");

        // Clearing empties the cache.
        clear_tls_cache();
        assert_eq!(tls_cache_len(), 0, "clear_tls_cache must empty the cache");

        drop((a, b));
    }
}
