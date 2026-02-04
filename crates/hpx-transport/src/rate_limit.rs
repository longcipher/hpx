//! Rate limiting for exchange APIs.
//!
//! This module provides a token bucket rate limiter for managing
//! API request rates to exchanges.

use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// A token bucket rate limiter.
#[derive(Debug)]
pub struct RateLimiter {
    buckets: Mutex<Vec<TokenBucket>>,
}

#[derive(Debug)]
struct TokenBucket {
    name: String,
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
            buckets: Mutex::new(Vec::new()),
        }
    }

    /// Add a rate limit rule.
    ///
    /// # Arguments
    /// * `name` - Name of the rate limit (e.g., "orders", "public")
    /// * `capacity` - Maximum tokens in the bucket
    /// * `refill_rate` - Tokens added per second
    pub fn add_limit(&self, name: impl Into<String>, capacity: u32, refill_rate: f64) {
        let mut buckets = self.buckets.lock();
        buckets.push(TokenBucket {
            name: name.into(),
            capacity: f64::from(capacity),
            tokens: f64::from(capacity),
            refill_rate,
            last_update: Instant::now(),
        });
    }

    /// Try to acquire a token without blocking.
    ///
    /// Returns `true` if a token was acquired, `false` if rate limited.
    pub fn try_acquire(&self, name: &str) -> bool {
        let mut buckets = self.buckets.lock();

        for bucket in buckets.iter_mut() {
            if bucket.name == name {
                // Refill tokens based on elapsed time
                let now = Instant::now();
                let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
                bucket.tokens = (bucket.tokens + elapsed * bucket.refill_rate).min(bucket.capacity);
                bucket.last_update = now;

                // Try to consume a token
                if bucket.tokens >= 1.0 {
                    bucket.tokens -= 1.0;
                    return true;
                }
                return false;
            }
        }

        // No matching rule, allow by default
        true
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
        let mut buckets = self.buckets.lock();

        for bucket in buckets.iter_mut() {
            if bucket.name == name {
                // Refill tokens based on elapsed time
                let now = Instant::now();
                let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
                let current_tokens =
                    (bucket.tokens + elapsed * bucket.refill_rate).min(bucket.capacity);

                if current_tokens >= 1.0 {
                    return Some(Duration::ZERO);
                }

                // Calculate time until we have 1 token
                let needed = 1.0 - current_tokens;
                let wait_secs = needed / bucket.refill_rate;
                return Some(Duration::from_secs_f64(wait_secs));
            }
        }

        None
    }

    /// Get current available tokens for a bucket.
    pub fn available_tokens(&self, name: &str) -> Option<f64> {
        let mut buckets = self.buckets.lock();

        for bucket in buckets.iter_mut() {
            if bucket.name == name {
                let now = Instant::now();
                let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
                return Some((bucket.tokens + elapsed * bucket.refill_rate).min(bucket.capacity));
            }
        }

        None
    }

    /// Reset a bucket to full capacity.
    pub fn reset(&self, name: &str) {
        let mut buckets = self.buckets.lock();

        for bucket in buckets.iter_mut() {
            if bucket.name == name {
                bucket.tokens = bucket.capacity;
                bucket.last_update = Instant::now();
                return;
            }
        }
    }

    /// Remove all rate limit rules.
    pub fn clear(&self) {
        let mut buckets = self.buckets.lock();
        buckets.clear();
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
