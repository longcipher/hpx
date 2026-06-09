//! Token-bucket rate limiter for byte-level speed control.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use crate::error::DownloadError;

/// Token-bucket rate limiter for byte-level speed control.
///
/// Refills tokens at a fixed rate (bytes per second).
/// Before reading each chunk, the caller waits until enough tokens are available.
///
/// Uses a lock-free atomic CAS loop for token management, avoiding async mutex
/// contention under high concurrency.
#[derive(Debug)]
pub struct SpeedLimiter {
    /// Maximum tokens (burst capacity).
    capacity: u64,
    /// Current available tokens.
    tokens: Arc<AtomicU64>,
    /// Refill rate in bytes per second.
    bytes_per_second: u64,
    /// Last refill timestamp as nanoseconds since a monotonic reference point.
    /// Uses `AtomicU64` with CAS for lock-free updates.
    last_refill_nanos: Arc<AtomicU64>,
    /// Reference instant for monotonic time calculations.
    origin: Instant,
}

impl SpeedLimiter {
    /// Create a new speed limiter with the given bytes-per-second rate.
    ///
    /// A rate of `0` is treated as unlimited.
    #[must_use]
    pub fn new(bytes_per_second: u64) -> Self {
        Self {
            capacity: bytes_per_second,
            tokens: Arc::new(AtomicU64::new(bytes_per_second)),
            bytes_per_second,
            last_refill_nanos: Arc::new(AtomicU64::new(0)),
            origin: Instant::now(),
        }
    }

    /// Create a limiter with an empty bucket so the first chunk is throttled.
    #[must_use]
    pub(crate) fn depleted(bytes_per_second: u64) -> Self {
        Self {
            capacity: bytes_per_second,
            tokens: Arc::new(AtomicU64::new(0)),
            bytes_per_second,
            last_refill_nanos: Arc::new(AtomicU64::new(0)),
            origin: Instant::now(),
        }
    }

    /// Create an unlimited limiter (no-op).
    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            capacity: 0,
            tokens: Arc::new(AtomicU64::new(0)),
            bytes_per_second: 0,
            last_refill_nanos: Arc::new(AtomicU64::new(0)),
            origin: Instant::now(),
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

    /// Convert an `Instant` to nanos relative to `self.origin`.
    fn instant_to_nanos(&self, instant: Instant) -> u64 {
        instant.duration_since(self.origin).as_nanos() as u64
    }

    /// Refill tokens based on elapsed time using a CAS loop.
    fn refill_tokens(&self, now_nanos: u64) {
        loop {
            let last = self.last_refill_nanos.load(Ordering::Acquire);
            if now_nanos <= last {
                return;
            }
            let elapsed_nanos = now_nanos - last;
            let new_tokens = elapsed_nanos
                .saturating_mul(self.bytes_per_second)
                .div_ceil(1_000_000_000);
            if new_tokens == 0 {
                return;
            }
            let current = self.tokens.load(Ordering::Acquire);
            let updated = current.saturating_add(new_tokens).min(self.capacity);
            if updated == current {
                // No change needed, just update the timestamp.
                let _ = self.last_refill_nanos.compare_exchange_weak(
                    last,
                    now_nanos,
                    Ordering::Release,
                    Ordering::Relaxed,
                );
                return;
            }
            // Try to atomically update tokens and timestamp.
            // We update tokens first, then timestamp. The timestamp CAS
            // may fail if another thread updated it, which is fine —
            // the next refill will pick up from the new timestamp.
            if self
                .tokens
                .compare_exchange_weak(current, updated, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                let _ = self.last_refill_nanos.compare_exchange_weak(
                    last,
                    now_nanos,
                    Ordering::Release,
                    Ordering::Relaxed,
                );
                return;
            }
            // CAS failed, retry.
        }
    }

    /// Try to consume `requested` tokens. Returns the number actually consumed.
    fn try_consume(&self, requested: u64) -> u64 {
        loop {
            let available = self.tokens.load(Ordering::Acquire);
            if available == 0 {
                return 0;
            }
            let granted = available.min(requested);
            if self
                .tokens
                .compare_exchange_weak(
                    available,
                    available - granted,
                    Ordering::Release,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return granted;
            }
        }
    }

    /// Wait until `bytes` tokens are available, then consume them.
    ///
    /// For unlimited limiters, returns immediately.
    pub async fn wait_for(&self, bytes: u64) -> Result<(), DownloadError> {
        if self.is_unlimited() || bytes == 0 {
            return Ok(());
        }

        let mut remaining = bytes;
        loop {
            let now_nanos = self.instant_to_nanos(Instant::now());
            self.refill_tokens(now_nanos);

            let granted = self.try_consume(remaining);
            remaining -= granted;
            if remaining == 0 {
                return Ok(());
            }

            // Calculate sleep duration based on deficit.
            let sleep_ms = remaining
                .saturating_mul(1000)
                .div_ceil(self.bytes_per_second);
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        }
    }
}

impl Clone for SpeedLimiter {
    fn clone(&self) -> Self {
        Self {
            capacity: self.capacity,
            tokens: Arc::clone(&self.tokens),
            bytes_per_second: self.bytes_per_second,
            last_refill_nanos: Arc::clone(&self.last_refill_nanos),
            origin: self.origin,
        }
    }
}

/// Combines global and per-download speed limits.
///
/// Both limiters are enforced independently; the caller waits on whichever
/// is more restrictive for each chunk.
#[derive(Debug, Clone)]
pub struct CompositeLimiter {
    global: Option<Arc<SpeedLimiter>>,
    per_download: Option<Arc<SpeedLimiter>>,
}

impl CompositeLimiter {
    /// Create a new composite limiter from optional global and per-download limiters.
    #[must_use]
    pub const fn new(
        global: Option<Arc<SpeedLimiter>>,
        per_download: Option<Arc<SpeedLimiter>>,
    ) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── SpeedLimiter tests ──────────────────────────────────────────────

    #[test]
    fn test_limiter_new_has_capacity_tokens() {
        let lim = SpeedLimiter::new(1024);
        assert_eq!(lim.bytes_per_second(), 1024);
        assert!(!lim.is_unlimited());
        assert_eq!(lim.tokens.load(Ordering::Relaxed), 1024);
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
    async fn test_tokens_consumed_within_capacity() {
        let lim = SpeedLimiter::new(1000);
        // Should consume 200 tokens immediately (have 1000).
        lim.wait_for(200).await.unwrap();
        let remaining = lim.tokens.load(Ordering::Relaxed);
        assert_eq!(remaining, 800);
    }

    #[tokio::test]
    async fn test_speed_limit_approximately_respected() {
        let bps: u64 = 5000; // 5 KB/s
        let bytes: u64 = 10_000; // 10 KB
        let lim = SpeedLimiter::new(bps);
        // Consume initial capacity to start fresh.
        lim.tokens.store(0, Ordering::Relaxed);

        let start = Instant::now();
        lim.wait_for(bytes).await.unwrap();
        let elapsed = start.elapsed();

        // Expected: ~2 seconds. Allow 1.5–3.0 s tolerance.
        assert!(
            elapsed >= Duration::from_millis(1500),
            "Too fast: {elapsed:?}"
        );
        assert!(elapsed <= Duration::from_secs(5), "Too slow: {elapsed:?}");
    }

    #[tokio::test]
    async fn test_refill_respects_capacity() {
        let lim = SpeedLimiter::new(500);
        // Drain all tokens.
        lim.tokens.store(0, Ordering::Relaxed);

        // Wait a short time and try to consume more than capacity.
        tokio::time::sleep(Duration::from_millis(50)).await;
        lim.wait_for(100).await.unwrap();

        // After consuming, tokens should not exceed capacity.
        let tokens = lim.tokens.load(Ordering::Relaxed);
        assert!(tokens <= 500);
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let lim = SpeedLimiter::new(1000);
        let lim2 = lim.clone();

        lim.wait_for(500).await.unwrap();
        let remaining = lim2.tokens.load(Ordering::Relaxed);
        assert_eq!(remaining, 500);
    }

    #[tokio::test]
    async fn test_large_chunk_triggers_sleep() {
        // 100 B/s, request 200 bytes from an empty bucket with 100-byte burst
        // capacity takes about 3s total: 2s to fill the burst, then 1s more for
        // the remaining 100 bytes.
        let lim = SpeedLimiter::new(100);
        lim.tokens.store(0, Ordering::Relaxed);

        let start = Instant::now();
        lim.wait_for(200).await.unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(800),
            "Too fast: {elapsed:?}"
        );
        assert!(
            elapsed <= Duration::from_millis(4000),
            "Too slow: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_concurrent_waiters_share_the_same_bucket() {
        let lim = SpeedLimiter::new(100);
        lim.tokens.store(0, Ordering::Relaxed);

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
        assert!(
            elapsed >= Duration::from_millis(1800),
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
        lim.tokens.store(0, Ordering::Relaxed);

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
            elapsed >= Duration::from_millis(1200),
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
        let global = Arc::new(SpeedLimiter::new(1000));
        let comp = CompositeLimiter::new(Some(global), None);

        comp.wait_for(500).await.unwrap();
        assert!(!comp.is_unlimited());
        assert_eq!(comp.effective_bytes_per_second(), Some(1000));
    }

    #[tokio::test]
    async fn test_composite_with_per_download_only() {
        let per_dl = Arc::new(SpeedLimiter::new(2000));
        let comp = CompositeLimiter::new(None, Some(per_dl));

        comp.wait_for(500).await.unwrap();
        assert!(!comp.is_unlimited());
        assert_eq!(comp.effective_bytes_per_second(), Some(2000));
    }

    #[tokio::test]
    async fn test_composite_effective_is_minimum() {
        let global = Arc::new(SpeedLimiter::new(1000));
        let per_dl = Arc::new(SpeedLimiter::new(500));
        let comp = CompositeLimiter::new(Some(global), Some(per_dl));

        assert_eq!(comp.effective_bytes_per_second(), Some(500));
    }

    #[tokio::test]
    async fn test_composite_enforces_both_limiters() {
        // Global: 200 B/s, per-download: 100 B/s
        // Requesting 300 bytes from zero tokens → per-download dominates (~3s).
        let global = Arc::new(SpeedLimiter::new(200));
        global.tokens.store(0, Ordering::Relaxed);
        let per_dl = Arc::new(SpeedLimiter::new(100));
        per_dl.tokens.store(0, Ordering::Relaxed);

        let comp = CompositeLimiter::new(Some(global), Some(per_dl));

        let start = Instant::now();
        comp.wait_for(300).await.unwrap();
        let elapsed = start.elapsed();

        // Per-download (100 B/s) takes ~3s; global (200 B/s) takes ~1.5s.
        // Sequential: ~4.5s total, but per-download runs after global so refills overlap.
        // Allow generous bounds.
        assert!(
            elapsed >= Duration::from_millis(2500),
            "Too fast: {elapsed:?}"
        );
        assert!(elapsed <= Duration::from_secs(8), "Too slow: {elapsed:?}");
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
