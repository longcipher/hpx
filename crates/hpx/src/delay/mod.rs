//! Request delay middleware.
//!
//! Adds configurable delays before HTTP requests — useful for rate limiting,
//! testing under slow network conditions, or just being polite to APIs.
//!
//! # Quick Start
//!
//! Fixed 1-second delay:
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use hpx::{Client, delay::DelayLayer};
//!
//! let client = Client::builder()
//!     .layer(DelayLayer::new(Duration::from_secs(1)))
//!     .build()?;
//! # Ok::<(), hpx::Error>(())
//! ```
//!
//! Random jitter (0.8s ~ 1.2s):
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use hpx::{Client, delay::JitterDelayLayer};
//!
//! let client = Client::builder()
//!     .layer(JitterDelayLayer::new(Duration::from_secs(1), 0.2))
//!     .build()?;
//! # Ok::<(), hpx::Error>(())
//! ```
//!
//! # Conditional Delays
//!
//! Use `.when()` to apply delays only to matching requests:
//!
//! ```ignore
//! // Only delay POST requests
//! DelayLayer::new(Duration::from_secs(1))
//!     .when(|req: &http::Request<_>| req.method() == http::Method::POST)
//!
//! // Jitter on specific paths
//! JitterDelayLayer::new(Duration::from_millis(500), 0.3)
//!     .when(|req: &http::Request<_>| req.uri().path().starts_with("/api"))
//! ```
//!
//! # Notes
//!
//! - Delays are async and won't block the runtime
//! - Not a substitute for proper rate limiters — servers can still see timing patterns
//! - Keep delays short in hot paths

// pin_project_lite does not support doc comments on fields,
// so we allow missing_docs for the future module.
#[allow(missing_docs)]
mod future;
mod layer;
mod service;

use std::time::Duration;

pub use self::{
    future::ResponseFuture,
    layer::{DelayLayer, DelayLayerWith, JitterDelayLayer, JitterDelayLayerWith},
    service::{Delay, DelayWith, JitterDelay, JitterDelayWith},
};

/// Compute a randomized duration in `[base * (1 - pct), base * (1 + pct)]`.
fn jittered_duration(base: Duration, pct: f64) -> Duration {
    let jitter = base.mul_f64(pct);
    let low = base.saturating_sub(jitter);
    let high = base.saturating_add(jitter);

    if low >= high {
        return base;
    }

    // XOR-Shift PRNG — same proven approach used in emulation::rand.
    let rand = fast_random();

    // Convert to fraction in [0, 1)
    let frac = (rand as f64) / (u64::MAX as f64);

    let span = (high - low).as_secs_f64();
    low + Duration::from_secs_f64(span * frac)
}

/// Fast thread-local XOR-Shift PRNG seeded from `RandomState`.
fn fast_random() -> u64 {
    use std::{
        cell::Cell,
        collections::hash_map::RandomState,
        hash::{BuildHasher, Hasher},
        num::Wrapping,
    };

    thread_local! {
        static RNG: Cell<Wrapping<u64>> = Cell::new(Wrapping(seed()));
    }

    #[inline]
    fn seed() -> u64 {
        let seed = RandomState::new();
        let mut out = 0;
        let mut cnt = 0;
        while out == 0 {
            cnt += 1;
            let mut hasher = seed.build_hasher();
            hasher.write_usize(cnt);
            out = hasher.finish();
        }
        out
    }

    RNG.with(|rng| {
        let mut n = rng.get();
        debug_assert_ne!(n.0, 0);
        n ^= n >> 12;
        n ^= n << 25;
        n ^= n >> 27;
        rng.set(n);
        n.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    })
}
