//! Multi-dimensional quota pools with server-authoritative calibration.
//!
//! Exchanges rarely expose a single rate limit. Binance charges a per-minute
//! **request weight** per IP, an independent **order count** per account, and
//! a separate **WebSocket frame** budget. A single token bucket cannot model
//! that, and a purely local token bucket drifts away from the server's view —
//! which is exactly how a client earns a `429` or `418`.
//!
//! [`QuotaPool`] fixes both problems: each dimension has its own capacity and
//! window, and [`QuotaPool::calibrate`] overwrites local usage with the value
//! reported in response headers such as `X-MBX-USED-WEIGHT-1M` or
//! `X-MBX-ORDER-COUNT-1M`. `Retry-After` is honoured as a hard block.

use std::{
    fmt,
    sync::{
        Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Injectable millisecond clock.
///
/// Production uses [`SystemClock`]; tests use a manually advanced clock so no
/// test ever sleeps.
pub trait Clock: Send + Sync {
    /// Milliseconds since an arbitrary but monotonic epoch.
    fn now_ms(&self) -> u64;
}

/// Monotonic process-local [`Clock`].
///
/// The epoch is the first call in the process, so the value is *not* a Unix
/// timestamp. It is deliberately backed by [`Instant`] rather than the wall
/// clock: an NTP step must not move a quota window forward (silently granting
/// budget) or backward (blocking a window that has already elapsed).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        let epoch = EPOCH.get_or_init(Instant::now);
        u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// Deterministic clock for tests and simulations.
#[derive(Debug)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    /// Create a clock starting at `start_ms`.
    #[inline]
    #[must_use]
    pub const fn new(start_ms: u64) -> Self {
        Self(AtomicU64::new(start_ms))
    }

    /// Set the current time.
    #[inline]
    pub fn set(&self, now_ms: u64) {
        self.0.store(now_ms, Ordering::Release);
    }

    /// Advance the current time.
    #[inline]
    pub fn advance(&self, delta_ms: u64) {
        self.0.fetch_add(delta_ms, Ordering::AcqRel);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}

/// The independent quota dimensions used by crypto exchange APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuotaKind {
    /// Per-IP request weight (Binance `X-MBX-USED-WEIGHT-1M`).
    IpWeight,
    /// Per-account order rate (`X-MBX-ORDER-COUNT-1M`).
    Order,
    /// WebSocket subscription/frame rate.
    WebSocketFrame,
}

/// Capacity and window for one quota dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuotaConfig {
    /// Maximum cost that may be spent inside one window.
    pub capacity: u64,
    /// Window length.
    pub window: Duration,
}

impl QuotaConfig {
    /// Create a quota configuration.
    #[inline]
    #[must_use]
    pub const fn new(capacity: u64, window: Duration) -> Self {
        Self { capacity, window }
    }
}

/// Why a quota acquisition was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitError {
    /// The requested cost would exceed the capacity for this window.
    Exhausted {
        /// Milliseconds until the current window rolls over.
        retry_after_ms: u64,
    },
    /// The server asked us to back off via `Retry-After`.
    ServerBackoff {
        /// Milliseconds remaining in the backoff.
        retry_after_ms: u64,
    },
    /// The requested cost can never fit in this pool's capacity.
    CostTooLarge,
    /// A response header held a value outside the representable range.
    InvalidCalibration,
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted { retry_after_ms } => {
                write!(f, "quota exhausted, retry in {retry_after_ms} ms")
            }
            Self::ServerBackoff { retry_after_ms } => {
                write!(f, "server backoff, retry in {retry_after_ms} ms")
            }
            Self::CostTooLarge => f.write_str("request cost exceeds pool capacity"),
            Self::InvalidCalibration => f.write_str("invalid rate-limit response header"),
        }
    }
}

impl std::error::Error for LimitError {}

/// One quota dimension, shared safely between tasks.
///
/// The implementation uses a fixed-window counter. The counter and the window
/// start advance together under one short-lived mutex, because a reset and a
/// concurrent acquisition must not race: publishing the new window before
/// resetting the counter (or the reverse) can lose an acquisition and leave
/// the client under-counting, which is exactly how a local limiter earns a
/// server ban. A window is reset lazily on the first access after expiry, so
/// no background timer is needed.
#[derive(Debug)]
pub struct QuotaPool<C> {
    kind: QuotaKind,
    capacity: u64,
    window_ms: u64,
    state: Mutex<WindowState>,
    blocked_until_ms: AtomicU64,
    clock: C,
}

#[derive(Debug)]
struct WindowState {
    start_ms: u64,
    used: u64,
}

impl<C: Clock> QuotaPool<C> {
    /// Create a pool for `kind` with `config`, using `clock`.
    #[must_use]
    pub fn new(kind: QuotaKind, config: QuotaConfig, clock: C) -> Self {
        let now = clock.now_ms();
        Self {
            kind,
            capacity: config.capacity,
            // A zero-length window would reset on every access and silently
            // disable the limit; one millisecond keeps the pool conservative.
            window_ms: u64::try_from(config.window.as_millis())
                .unwrap_or(u64::MAX)
                .max(1),
            state: Mutex::new(WindowState {
                start_ms: now,
                used: 0,
            }),
            blocked_until_ms: AtomicU64::new(0),
            clock,
        }
    }

    /// The dimension this pool governs.
    #[inline]
    #[must_use]
    pub const fn kind(&self) -> QuotaKind {
        self.kind
    }

    /// Configured capacity.
    #[inline]
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Currently recorded usage in the active window.
    #[inline]
    pub fn used(&self) -> u64 {
        let now = self.clock.now_ms();
        let mut state = self.lock_state();
        self.roll(&mut state, now);
        state.used
    }

    /// Remaining budget in the active window.
    #[inline]
    pub fn remaining(&self) -> u64 {
        self.capacity.saturating_sub(self.used())
    }

    /// Lock the window state.
    ///
    /// A panic while the lock is held cannot leave the counter inconsistent,
    /// so a poisoned lock is still safe to use.
    fn lock_state(&self) -> MutexGuard<'_, WindowState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reset the window counter if the window has elapsed.
    fn roll(&self, state: &mut WindowState, now: u64) {
        if now.saturating_sub(state.start_ms) >= self.window_ms {
            state.start_ms = now;
            state.used = 0;
        }
    }

    /// Try to spend `cost` units.
    ///
    /// # Errors
    ///
    /// Returns [`LimitError`] when a server backoff is active, the cost can
    /// never fit the pool, or the window budget is exhausted.
    #[inline]
    pub fn try_acquire(&self, cost: u64) -> Result<(), LimitError> {
        let now = self.clock.now_ms();
        let blocked_until = self.blocked_until_ms.load(Ordering::Acquire);
        if now < blocked_until {
            return Err(LimitError::ServerBackoff {
                retry_after_ms: blocked_until - now,
            });
        }
        if cost > self.capacity {
            return Err(LimitError::CostTooLarge);
        }

        let mut state = self.lock_state();
        self.roll(&mut state, now);
        match state.used.checked_add(cost) {
            Some(next) if next <= self.capacity => {
                state.used = next;
                Ok(())
            }
            _ => Err(LimitError::Exhausted {
                retry_after_ms: self
                    .window_ms
                    .saturating_sub(now.saturating_sub(state.start_ms)),
            }),
        }
    }

    /// Reconcile local usage with a value reported by the server.
    ///
    /// The server's view is authoritative: calibration may raise **or lower**
    /// the local counter. This is what prevents clock drift and proxy-side
    /// weight aggregation from causing `429`/`418` bans.
    ///
    /// # Errors
    ///
    /// Returns [`LimitError::InvalidCalibration`] when `observed_used` exceeds
    /// the pool capacity. Local usage is clamped to the capacity in that case,
    /// so ignoring the error still leaves the pool exhausted instead of
    /// under-counting.
    pub fn calibrate(&self, observed_used: u64) -> Result<(), LimitError> {
        let now = self.clock.now_ms();
        let mut state = self.lock_state();
        self.roll(&mut state, now);
        if observed_used > self.capacity {
            state.used = self.capacity;
            return Err(LimitError::InvalidCalibration);
        }
        // Calibration happens at a response boundary, i.e. a synchronization
        // point: adopting the server's snapshot (up or down) is intentional.
        state.used = observed_used;
        Ok(())
    }

    /// Honour a `Retry-After` duration from an error response.
    pub fn backoff(&self, duration: Duration) {
        let delta = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        let until = self.clock.now_ms().saturating_add(delta);
        self.blocked_until_ms.fetch_max(until, Ordering::AcqRel);
    }

    /// Honour a `Retry-After` value expressed in seconds.
    pub fn backoff_seconds(&self, seconds: u64) {
        self.backoff(Duration::from_secs(seconds));
    }
}

/// Named response headers that carry server-side usage.
pub mod headers {
    /// Binance per-IP request weight for the current one-minute window.
    pub const USED_WEIGHT_1M: &str = "x-mbx-used-weight-1m";
    /// Binance per-account order count for the current window.
    pub const ORDER_COUNT_1M: &str = "x-mbx-order-count-1m";
    /// Standard backoff header.
    pub const RETRY_AFTER: &str = "retry-after";
}

/// Parse a decimal [`headers::USED_WEIGHT_1M`] value.
///
/// Returns `None` for empty, non-numeric or overflowing values so a malformed
/// header can never poison a quota pool.
#[must_use]
pub fn parse_used_weight(value: &str) -> Option<u64> {
    value.trim().parse().ok()
}

/// Parse `Retry-After` in either delta-seconds or integer-seconds form.
#[must_use]
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn pool(capacity: u64, clock: ManualClock) -> QuotaPool<ManualClock> {
        QuotaPool::new(
            QuotaKind::IpWeight,
            QuotaConfig::new(capacity, MINUTE),
            clock,
        )
    }

    #[test]
    fn acquisition_consumes_the_budget() {
        let clock = ManualClock::new(0);
        let pool = pool(10, clock);
        assert_eq!(pool.remaining(), 10);
        assert_eq!(pool.try_acquire(6), Ok(()));
        assert_eq!(pool.used(), 6);
        assert_eq!(pool.try_acquire(4), Ok(()));
        assert!(matches!(
            pool.try_acquire(1),
            Err(LimitError::Exhausted { .. })
        ));
    }

    #[test]
    fn window_rolls_over_without_a_background_task() {
        let clock = ManualClock::new(0);
        let pool = pool(10, clock);
        assert_eq!(pool.try_acquire(10), Ok(()));
        assert!(pool.try_acquire(1).is_err());
        pool.clock.advance(60_000);
        assert_eq!(pool.try_acquire(10), Ok(()));
        assert_eq!(pool.used(), 10);
    }

    #[test]
    fn cost_larger_than_capacity_is_rejected_before_rollover() {
        let pool = pool(3, ManualClock::new(0));
        assert_eq!(pool.try_acquire(4), Err(LimitError::CostTooLarge));
        assert_eq!(pool.used(), 0);
    }

    #[test]
    fn server_calibration_can_raise_local_usage() {
        let pool = pool(6000, ManualClock::new(0));
        assert_eq!(pool.try_acquire(100), Ok(()));
        assert_eq!(pool.calibrate(5800), Ok(()));
        assert_eq!(pool.used(), 5800);
        assert_eq!(pool.remaining(), 200);
        assert!(pool.try_acquire(201).is_err());
    }

    #[test]
    fn server_calibration_can_lower_stale_local_usage() {
        let pool = pool(6000, ManualClock::new(0));
        assert_eq!(pool.try_acquire(4000), Ok(()));
        assert_eq!(pool.calibrate(1200), Ok(()));
        assert_eq!(pool.used(), 1200);
        assert_eq!(pool.remaining(), 4800);
    }

    #[test]
    fn calibration_rejects_impossible_values_and_leaves_the_pool_exhausted() {
        let pool = pool(100, ManualClock::new(0));
        assert_eq!(pool.calibrate(101), Err(LimitError::InvalidCalibration));
        assert_eq!(pool.used(), 100);
        assert!(matches!(
            pool.try_acquire(1),
            Err(LimitError::Exhausted { .. })
        ));
    }

    #[test]
    fn zero_length_window_still_limits() {
        let pool = QuotaPool::new(
            QuotaKind::WebSocketFrame,
            QuotaConfig::new(1, Duration::ZERO),
            ManualClock::new(0),
        );
        assert_eq!(pool.try_acquire(1), Ok(()));
        assert!(matches!(
            pool.try_acquire(1),
            Err(LimitError::Exhausted { .. })
        ));
    }

    #[test]
    fn counter_overflow_is_exhaustion_not_a_wrap() {
        let pool = pool(u64::MAX, ManualClock::new(0));
        assert_eq!(pool.try_acquire(u64::MAX), Ok(()));
        assert!(matches!(
            pool.try_acquire(u64::MAX),
            Err(LimitError::Exhausted { .. })
        ));
        assert_eq!(pool.used(), u64::MAX);
    }

    /// A window rollover must never lose a concurrently granted acquisition.
    #[test]
    fn concurrent_acquisitions_never_exceed_capacity() {
        let pool = Arc::new(QuotaPool::new(
            QuotaKind::Order,
            QuotaConfig::new(1_000, MINUTE),
            ManualClock::new(0),
        ));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let pool = Arc::clone(&pool);
            handles.push(thread::spawn(move || {
                let mut granted = 0u64;
                for _ in 0..500 {
                    if pool.try_acquire(1).is_ok() {
                        granted += 1;
                    }
                }
                granted
            }));
        }
        let granted: u64 = handles
            .into_iter()
            .map(|handle| handle.join().expect("worker"))
            .sum();
        assert_eq!(granted, 1_000);
        assert_eq!(pool.used(), 1_000);
    }

    #[test]
    fn retry_after_blocks_all_acquisitions_until_it_expires() {
        let clock = ManualClock::new(1_000);
        let pool = pool(10, clock);
        pool.backoff_seconds(5);
        assert!(matches!(
            pool.try_acquire(1),
            Err(LimitError::ServerBackoff {
                retry_after_ms: 5000
            })
        ));
        pool.clock.advance(4_999);
        assert!(pool.try_acquire(1).is_err());
        pool.clock.advance(1);
        assert_eq!(pool.try_acquire(1), Ok(()));
    }

    #[test]
    fn header_parsers_reject_malformed_values() {
        assert_eq!(parse_used_weight(" 1200 "), Some(1200));
        assert_eq!(parse_used_weight(""), None);
        assert_eq!(parse_used_weight("1.5"), None);
        assert_eq!(parse_used_weight("-1"), None);
        assert_eq!(parse_retry_after("30"), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after("nope"), None);
        assert_eq!(headers::USED_WEIGHT_1M, "x-mbx-used-weight-1m");
    }

    #[test]
    fn system_clock_never_moves_backwards() {
        let clock = SystemClock;
        let first = clock.now_ms();
        let second = clock.now_ms();
        assert!(second >= first);
    }

    #[test]
    fn manual_clock_is_monotonic_under_test_control() {
        let clock = ManualClock::new(7);
        assert_eq!(clock.now_ms(), 7);
        clock.advance(3);
        assert_eq!(clock.now_ms(), 10);
        clock.set(42);
        assert_eq!(clock.now_ms(), 42);
    }

    #[test]
    fn kinds_are_distinct() {
        assert_ne!(QuotaKind::IpWeight, QuotaKind::Order);
        assert_ne!(QuotaKind::Order, QuotaKind::WebSocketFrame);
        let quota = QuotaConfig::new(5, MINUTE);
        assert_eq!(quota.capacity, 5);
    }
}
