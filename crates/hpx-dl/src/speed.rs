//! Rate limiter for byte-level speed control using the `governor` crate.

use std::{num::NonZeroU32, sync::Arc};

use governor::{
    Quota, RateLimiter,
    clock::{Clock, QuantaClock},
    middleware::NoOpMiddleware,
    state::{InMemoryState, NotKeyed},
};

use crate::error::DownloadError;

type GovLimiter = RateLimiter<NotKeyed, InMemoryState, QuantaClock, NoOpMiddleware>;

/// Rate limiter for byte-level speed control.
///
/// Backed by governor's GCRA algorithm. A rate of 0 means unlimited.
/// Clone is cheap (Arc internally).
#[derive(Debug, Clone)]
pub struct SpeedLimiter {
    inner: Option<Arc<GovLimiter>>,
    bytes_per_second: u64,
}

impl SpeedLimiter {
    /// Create a new speed limiter with the given bytes-per-second rate.
    ///
    /// A rate of `0` is treated as unlimited.
    #[must_use]
    pub fn new(bytes_per_second: u64) -> Self {
        Self {
            inner: make_limiter(bytes_per_second),
            bytes_per_second,
        }
    }

    /// Create a limiter with an empty bucket so the first chunk is throttled.
    ///
    /// ponytail: identical to `new` — governor's GCRA starts with zero burst
    /// capacity by default, which matches "depleted" behavior.
    #[must_use]
    pub(crate) fn depleted(bytes_per_second: u64) -> Self {
        Self::new(bytes_per_second)
    }

    /// Create an unlimited limiter (no-op).
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            inner: None,
            bytes_per_second: 0,
        }
    }

    /// Return the configured rate in bytes per second.
    #[must_use]
    pub const fn bytes_per_second(&self) -> u64 {
        self.bytes_per_second
    }

    /// Return `true` if this is an unlimited limiter.
    #[must_use]
    pub const fn is_unlimited(&self) -> bool {
        self.bytes_per_second == 0
    }

    /// Wait until `bytes` tokens are available, then consume them.
    ///
    /// For unlimited limiters, returns immediately.
    /// Large requests exceeding burst capacity are chunked automatically.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub async fn wait_for(&self, bytes: u64) -> Result<(), DownloadError> {
        if self.is_unlimited() || bytes == 0 {
            return Ok(());
        }

        let Some(limiter) = self.inner.as_ref() else {
            return Ok(());
        };

        let mut remaining = bytes;
        while remaining > 0 {
            // Try consuming the full remaining amount.
            let n = match NonZeroU32::new(u32::try_from(remaining).unwrap_or(u32::MAX)) {
                Some(n) => n,
                None => return Ok(()),
            };
            match limiter.check_n(n) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(not_until)) => {
                    let wait = not_until.wait_time_from(limiter.clock().now());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                }
                Err(insufficient) => {
                    // Requested more than burst capacity. Chunk down to burst size.
                    // ponytail: governor's InsufficientCapacity.0 is always > 0
                    let Some(burst) = NonZeroU32::new(insufficient.0) else {
                        unreachable!("burst must be > 0")
                    };
                    match limiter.check_n(burst) {
                        Ok(Ok(())) => {
                            remaining -= u64::from(burst.get());
                        }
                        Ok(Err(not_until)) => {
                            let wait = not_until.wait_time_from(limiter.clock().now());
                            if !wait.is_zero() {
                                tokio::time::sleep(wait).await;
                            }
                        }
                        Err(_) => unreachable!("burst fits within capacity"),
                    }
                }
            }
        }
        Ok(())
    }
}

/// Combines global and per-download speed limits.
///
/// Both limiters are enforced independently; the caller waits on whichever
/// is more restrictive for each chunk.
#[derive(Debug, Clone)]
pub struct CompositeLimiter {
    global: Option<SpeedLimiter>,
    per_download: Option<SpeedLimiter>,
}

impl CompositeLimiter {
    /// Create a new composite limiter from optional global and per-download limiters.
    #[must_use]
    pub const fn new(global: Option<SpeedLimiter>, per_download: Option<SpeedLimiter>) -> Self {
        Self {
            global,
            per_download,
        }
    }

    /// Create a composite with no limits.
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            global: None,
            per_download: None,
        }
    }

    /// Wait on both limiters (sequential — the caller must satisfy both).
    pub async fn wait_for(&self, bytes: u64) -> Result<(), DownloadError> {
        if let Some(ref global) = self.global {
            global.wait_for(bytes).await?;
        }
        if let Some(ref per_dl) = self.per_download {
            per_dl.wait_for(bytes).await?;
        }
        Ok(())
    }

    /// Return the effective rate in bytes per second (minimum of available limiters).
    #[must_use]
    pub fn effective_bytes_per_second(&self) -> Option<u64> {
        let g = self.global.as_ref().map_or(u64::MAX, |l| {
            if l.is_unlimited() {
                u64::MAX
            } else {
                l.bytes_per_second()
            }
        });
        let p = self.per_download.as_ref().map_or(u64::MAX, |l| {
            if l.is_unlimited() {
                u64::MAX
            } else {
                l.bytes_per_second()
            }
        });
        match (g, p) {
            (u64::MAX, u64::MAX) => None,
            (a, b) => Some(a.min(b)),
        }
    }

    /// Return `true` if neither limiter restricts speed.
    #[must_use]
    pub const fn is_unlimited(&self) -> bool {
        self.global.is_none() && self.per_download.is_none()
    }
}

fn make_limiter(bytes_per_second: u64) -> Option<Arc<GovLimiter>> {
    let bps = NonZeroU32::new(u32::try_from(bytes_per_second).unwrap_or(u32::MAX))?;
    // ponytail: burst = rate (1 second of burst), matching the old token bucket.
    let quota = Quota::per_second(bps).allow_burst(bps);
    Some(Arc::new(RateLimiter::direct(quota)))
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use super::*;

    // ── SpeedLimiter tests ──────────────────────────────────────────────

    #[test]
    fn test_limiter_new_has_correct_rate() {
        let lim = SpeedLimiter::new(1024);
        assert_eq!(lim.bytes_per_second(), 1024);
        assert!(!lim.is_unlimited());
    }

    #[test]
    fn test_unlimited_limiter() {
        let lim = SpeedLimiter::unlimited();
        assert!(lim.is_unlimited());
        assert_eq!(lim.bytes_per_second(), 0);
    }

    #[tokio::test]
    async fn test_unlimited_returns_immediately() {
        let lim = SpeedLimiter::unlimited();
        let start = Instant::now();
        lim.wait_for(1_000_000).await.unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn test_zero_bytes_returns_immediately() {
        let lim = SpeedLimiter::new(100);
        let start = Instant::now();
        lim.wait_for(0).await.unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn test_burst_within_capacity() {
        let lim = SpeedLimiter::new(1000);
        // 200 bytes < 1000-byte burst capacity → should complete immediately.
        let start = Instant::now();
        lim.wait_for(200).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn test_speed_limit_approximately_respected() {
        let bps: u64 = 5000; // 5 KB/s
        let bytes: u64 = 10_000; // 10 KB
        let lim = SpeedLimiter::new(bps);

        let start = Instant::now();
        lim.wait_for(bytes).await.unwrap();
        let elapsed = start.elapsed();

        // GCRA burst = bps (5000 bytes consumed instantly), remaining 5000 at 5000 B/s = ~1s.
        assert!(
            elapsed >= Duration::from_millis(800),
            "Too fast: {elapsed:?}"
        );
        assert!(
            elapsed <= Duration::from_millis(2500),
            "Too slow: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let lim = SpeedLimiter::new(1000);
        let lim2 = lim.clone();

        // Both should report the same rate.
        assert_eq!(lim.bytes_per_second(), lim2.bytes_per_second());
        assert_eq!(lim.is_unlimited(), lim2.is_unlimited());
    }

    #[tokio::test]
    async fn test_large_chunk_triggers_sleep() {
        // 100 B/s, request 200 bytes.
        // GCRA burst = 100, first 100 instant, remaining 100 at 100 B/s = ~1s.
        let lim = SpeedLimiter::new(100);

        let start = Instant::now();
        lim.wait_for(200).await.unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(800),
            "Too fast: {elapsed:?}"
        );
        assert!(
            elapsed <= Duration::from_millis(3000),
            "Too slow: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_concurrent_waiters_share_the_same_bucket() {
        let lim = SpeedLimiter::new(100);

        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let lim2 = lim.clone();
        let barrier1 = Arc::clone(&barrier);
        let barrier2 = Arc::clone(&barrier);

        let start = Instant::now();
        let waiter1 = tokio::spawn(async move {
            barrier1.wait().await;
            lim.wait_for(100).await.unwrap();
        });
        let waiter2 = tokio::spawn(async move {
            barrier2.wait().await;
            lim2.wait_for(100).await.unwrap();
        });

        barrier.wait().await;
        waiter1.await.unwrap();
        waiter2.await.unwrap();

        let elapsed = start.elapsed();
        // First waiter consumes burst (100) instantly, second waits ~1s.
        assert!(
            elapsed >= Duration::from_millis(500),
            "Concurrent waiters finished too quickly: {elapsed:?}"
        );
        assert!(
            elapsed <= Duration::from_secs(5),
            "Concurrent waiters took too long: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_concurrent_waiters_do_not_double_credit_sleep_time() {
        let lim = SpeedLimiter::new(100);

        let barrier = Arc::new(tokio::sync::Barrier::new(4));
        let mut handles = Vec::new();
        for _ in 0..3 {
            let waiter = lim.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                waiter.wait_for(50).await.unwrap();
            }));
        }

        let start = Instant::now();
        barrier.wait().await;
        for handle in handles {
            handle.await.unwrap();
        }

        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(500),
            "Three concurrent waiters finished too quickly: {elapsed:?}"
        );
        assert!(
            elapsed <= Duration::from_secs(5),
            "Three concurrent waiters took too long: {elapsed:?}"
        );
    }

    // ── CompositeLimiter tests ──────────────────────────────────────────

    #[test]
    fn test_composite_unlimited() {
        let comp = CompositeLimiter::unlimited();
        assert!(comp.is_unlimited());
        assert_eq!(comp.effective_bytes_per_second(), None);
    }

    #[tokio::test]
    async fn test_composite_with_global_only() {
        let global = SpeedLimiter::new(1000);
        let comp = CompositeLimiter::new(Some(global), None);

        comp.wait_for(500).await.unwrap();
        assert!(!comp.is_unlimited());
        assert_eq!(comp.effective_bytes_per_second(), Some(1000));
    }

    #[tokio::test]
    async fn test_composite_with_per_download_only() {
        let per_dl = SpeedLimiter::new(2000);
        let comp = CompositeLimiter::new(None, Some(per_dl));

        comp.wait_for(500).await.unwrap();
        assert!(!comp.is_unlimited());
        assert_eq!(comp.effective_bytes_per_second(), Some(2000));
    }

    #[tokio::test]
    async fn test_composite_effective_is_minimum() {
        let global = SpeedLimiter::new(1000);
        let per_dl = SpeedLimiter::new(500);
        let comp = CompositeLimiter::new(Some(global), Some(per_dl));

        assert_eq!(comp.effective_bytes_per_second(), Some(500));
    }

    #[tokio::test]
    async fn test_composite_enforces_both_limiters() {
        // Global: 200 B/s, per-download: 100 B/s
        // Requesting 300 bytes → GCRA burst absorbs the first chunk,
        // per-download (100 B/s) takes ~1s for remaining tokens.
        let global = SpeedLimiter::new(200);
        let per_dl = SpeedLimiter::new(100);
        let comp = CompositeLimiter::new(Some(global), Some(per_dl));

        let start = Instant::now();
        comp.wait_for(300).await.unwrap();
        let elapsed = start.elapsed();

        // Global finishes instantly (burst=200 ≥ 300 after chunking).
        // Per-download: burst=100, remaining 200 split into 2×100, one refill wait ~1s.
        assert!(
            elapsed >= Duration::from_millis(500),
            "Too fast: {elapsed:?}"
        );
        assert!(elapsed <= Duration::from_secs(5), "Too slow: {elapsed:?}");
    }

    #[tokio::test]
    async fn test_composite_unlimited_skips_both() {
        let comp = CompositeLimiter::unlimited();
        let start = Instant::now();
        comp.wait_for(1_000_000).await.unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }
}
