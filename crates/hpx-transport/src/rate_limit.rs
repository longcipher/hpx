//! Rate limiting for exchange APIs.
//!
//! This module provides a token bucket rate limiter for managing
//! API request rates to exchanges. Uses lock-free `scc::HashMap` for
//! high-performance concurrent access.

use std::time::{Duration, Instant};

use scc::HashMap as SccHashMap;

/// A token bucket rate limiter backed by lock-free `scc::HashMap`.
#[derive(Debug)]
pub struct RateLimiter {
    buckets: SccHashMap<String, TokenBucket>,
}

#[derive(Debug)]
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_rate: f64, // tokens per second
    last_update: Instant,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    /// Create a new rate limiter.
    pub fn new() -> Self {
        Self {
            buckets: SccHashMap::new(),
        }
    }

    /// Add a rate limit rule.
    ///
    /// # Arguments
    /// * `name` - Name of the rate limit (e.g., "orders", "public")
    /// * `capacity` - Maximum tokens in the bucket
    /// * `refill_rate` - Tokens added per second
    pub fn add_limit(&self, name: impl Into<String>, capacity: u32, refill_rate: f64) {
        let name = name.into();
        let bucket = TokenBucket {
            capacity: f64::from(capacity),
            tokens: f64::from(capacity),
            refill_rate,
            last_update: Instant::now(),
        };
        // Insert or update the bucket
        let _ = self.buckets.insert_sync(name, bucket);
    }

    /// Try to acquire a token without blocking.
    ///
    /// Returns `true` if a token was acquired, `false` if rate limited.
    pub fn try_acquire(&self, name: &str) -> bool {
        self.buckets
            .update_sync(name, |_, bucket| {
                // Refill tokens based on elapsed time
                let now = Instant::now();
                let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
                bucket.tokens = (bucket.tokens + elapsed * bucket.refill_rate).min(bucket.capacity);
                bucket.last_update = now;

                // Try to consume a token
                if bucket.tokens >= 1.0 {
                    bucket.tokens -= 1.0;
                    true
                } else {
                    false
                }
            })
            // No matching rule, allow by default
            .unwrap_or(true)
    }

    /// Acquire a token, waiting if necessary.
    pub async fn acquire(&self, name: &str) {
        loop {
            if self.try_acquire(name) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Get the time until a token will be available.
    pub fn time_until_available(&self, name: &str) -> Option<Duration> {
        self.buckets.read_sync(name, |_, bucket| {
            let now = Instant::now();
            let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
            let current_tokens =
                (bucket.tokens + elapsed * bucket.refill_rate).min(bucket.capacity);

            if current_tokens >= 1.0 {
                Duration::ZERO
            } else {
                let needed = 1.0 - current_tokens;
                let wait_secs = needed / bucket.refill_rate;
                Duration::from_secs_f64(wait_secs)
            }
        })
    }

    /// Get current available tokens for a bucket.
    pub fn available_tokens(&self, name: &str) -> Option<f64> {
        self.buckets.read_sync(name, |_, bucket| {
            let now = Instant::now();
            let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
            (bucket.tokens + elapsed * bucket.refill_rate).min(bucket.capacity)
        })
    }

    /// Reset a bucket to full capacity.
    pub fn reset(&self, name: &str) {
        self.buckets.update_sync(name, |_, bucket| {
            bucket.tokens = bucket.capacity;
            bucket.last_update = Instant::now();
        });
    }

    /// Remove all rate limit rules.
    pub fn clear(&self) {
        self.buckets.retain_sync(|_, _| false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_basic() {
        let limiter = RateLimiter::new();
        limiter.add_limit("test", 5, 1.0);

        // Should allow 5 requests
        for _ in 0..5 {
            assert!(limiter.try_acquire("test"));
        }

        // 6th should be denied
        assert!(!limiter.try_acquire("test"));
    }

    #[test]
    fn test_unknown_bucket() {
        let limiter = RateLimiter::new();
        // Unknown bucket should allow
        assert!(limiter.try_acquire("unknown"));
    }

    #[test]
    fn test_available_tokens() {
        let limiter = RateLimiter::new();
        limiter.add_limit("test", 10, 1.0);

        assert!(limiter.available_tokens("test").is_some());
        assert!(limiter.available_tokens("unknown").is_none());
    }

    #[test]
    fn test_reset() {
        let limiter = RateLimiter::new();
        limiter.add_limit("test", 5, 1.0);

        // Consume all tokens
        for _ in 0..5 {
            limiter.try_acquire("test");
        }
        assert!(!limiter.try_acquire("test"));

        // Reset
        limiter.reset("test");
        assert!(limiter.try_acquire("test"));
    }
}
