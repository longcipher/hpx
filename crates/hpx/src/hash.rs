use std::{
    borrow::Borrow,
    hash::{BuildHasher, Hash, Hasher},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use ahash::RandomState;

/// Pre-seeded [`RandomState`] for consistent internal hashing.
///
/// Uses fixed seeds to ensure deterministic hashing behavior across
/// program runs. Primarily used for connection pools and internal caches.
///
/// **Note**: Not cryptographically secure due to fixed seeds.
pub(crate) const HASHER: RandomState = RandomState::with_seeds(
    0x6b68_d618_a4b5_3c57,
    0xadc8_c4d5_82bb_1313,
    0x2f72_c2c1_9b04_2d4c,
    0x94e5_8d83_a26c_3f28,
);

/// A type alias for a hash set using `ahash` with a pre-seeded `RandomState`.
pub(crate) type HashSet<T> = std::collections::HashSet<T, RandomState>;

/// A type alias for a hash map using `ahash` with a pre-seeded `RandomState`.
pub(crate) type HashMap<K, V> = std::collections::HashMap<K, V, RandomState>;

/// A wrapper that memoizes the hash value of its contained data.
#[derive(Debug)]
pub(crate) struct HashMemo<T, H: BuildHasher = RandomState>
where
    T: Eq + PartialEq + Hash,
{
    value: T,
    hash: AtomicU64,
    computed: AtomicBool,
    hasher: H,
}

impl<T, H> HashMemo<T, H>
where
    T: Eq + Hash,
    H: BuildHasher,
{
    /// Creates a new `HashMemo` with a custom hasher.
    ///
    /// This allows you to specify a custom `BuildHasher` implementation for
    /// controlling how hash values are computed.
    pub(crate) const fn with_hasher(value: T, hasher: H) -> Self {
        Self {
            value,
            hash: AtomicU64::new(0),
            computed: AtomicBool::new(false),
            hasher,
        }
    }
}

impl<T, H> Hash for HashMemo<T, H>
where
    T: Eq + Hash,
    H: BuildHasher,
{
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn hash<H2: Hasher>(&self, state: &mut H2) {
        if self.computed.load(Ordering::Acquire) {
            // The acquire load pairs with the release store below, so the
            // memoized hash value is guaranteed to be visible to this thread.
            state.write_u64(self.hash.load(Ordering::Relaxed));
            return;
        }

        let computed_hash = self.hasher.hash_one(&self.value);

        // Publish the hash value *before* the flag. The release store makes
        // the preceding hash store visible to any thread that observes
        // `computed == true` through its acquire load; without that pairing a
        // reader could see the flag but a stale hash value.
        self.hash.store(computed_hash, Ordering::Relaxed);
        self.computed.store(true, Ordering::Release);
        state.write_u64(computed_hash);
    }
}

impl<T, H> PartialOrd for HashMemo<T, H>
where
    T: Eq + Hash + PartialOrd,
    H: BuildHasher,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<T, H> Ord for HashMemo<T, H>
where
    T: Eq + Hash + Ord,
    H: BuildHasher,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<T, H> PartialEq for HashMemo<T, H>
where
    T: Eq + Hash,
    H: BuildHasher,
{
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T, H> Eq for HashMemo<T, H>
where
    T: Eq + Hash,
    H: BuildHasher,
{
}

impl<T, H> AsRef<T> for HashMemo<T, H>
where
    T: Eq + Hash,
    H: BuildHasher,
{
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T, H> Borrow<T> for HashMemo<T, H>
where
    T: Eq + Hash,
    H: BuildHasher,
{
    fn borrow(&self) -> &T {
        &self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hash a memoized value with the crate's fixed-seed hasher.
    ///
    /// The lint's suggested rewrite (`hasher.hash_one(&memo.value)`) would
    /// hash the wrapped value instead of the memo, which is a different
    /// computation.
    #[expect(
        clippy::manual_hash_one,
        reason = "the memo's hash must be hashed, not the value it wraps"
    )]
    fn hash_of<T, H>(memo: &HashMemo<T, H>) -> u64
    where
        T: Eq + Hash,
        H: BuildHasher,
    {
        let mut hasher = HASHER.build_hasher();
        memo.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn hash_memo_distinguishes_different_values() {
        // A memoized hash that collapsed to a constant would map every key
        // to the same bucket.
        let a = HashMemo::with_hasher("alpha".to_string(), HASHER);
        let b = HashMemo::with_hasher("beta".to_string(), HASHER);

        assert_ne!(hash_of(&a), hash_of(&b));
        assert_ne!(a, b);
    }

    #[test]
    fn hash_memo_is_stable_across_calls() {
        // The memoized value must not change between calls: the flag and the
        // cached hash are published with the right ordering.
        let memo = HashMemo::with_hasher("stable".to_string(), HASHER);

        let first = hash_of(&memo);
        let second = hash_of(&memo);
        assert_eq!(first, second);
    }

    #[test]
    fn hash_memo_of_equal_values_matches() {
        let a = HashMemo::with_hasher("same".to_string(), HASHER);
        let b = HashMemo::with_hasher("same".to_string(), HASHER);

        assert_eq!(a, b);
        assert_eq!(hash_of(&a), hash_of(&b));
    }
}
