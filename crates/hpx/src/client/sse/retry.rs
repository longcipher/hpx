//! Exponential backoff and jitter for SSE reconnections.
//!
//! Vendored from [sse-core](https://github.com/PizzasBear/sse-rs) and adapted for hpx.

use std::time::Duration;

/// Configuration for exponential backoff and jitter during stream reconnections.
#[derive(Debug, Clone, Copy)]
pub struct SseRetryConfig {
    /// The maximum number of consecutive connection attempts before giving up.
    pub max_retries: u32,
    /// The absolute maximum wait time between connection attempts in milliseconds.
    pub max_backoff_ms: u32,
    /// The absolute minimum wait time between connection attempts in milliseconds.
    pub min_sleep_ms: u32,
    /// The multiplier applied to the delay after each failed attempt.
    pub backoff_multiplier: f32,
    /// Whether to apply randomness (jitter) to the reconnect delay.
    pub jitter: bool,
}

impl SseRetryConfig {
    /// Creates a new retry configuration with sensible defaults.
    ///
    /// Defaults: 20 max retries, 60s max backoff, 200ms min sleep, 2x multiplier, jitter on.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_retries: 20,
            max_backoff_ms: 60_000,
            min_sleep_ms: 200,
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }

    /// Creates a retry configuration that disables all automatic retries.
    #[inline]
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            max_retries: 0,
            ..Self::new()
        }
    }

    /// Calculates the delay duration for the next reconnection attempt.
    ///
    /// Returns [`None`] if the `attempt` count exceeds [`Self::max_retries`].
    #[must_use]
    pub fn calculate_backoff(&self, reconnect_time_ms: u32, attempt: u32) -> Option<Duration> {
        self.calculate_backoff_with_factor(reconnect_time_ms, attempt, fastrand::f32())
    }

    /// Calculates the delay with an explicit jitter factor (0.0..=1.0).
    #[must_use]
    pub fn calculate_backoff_with_factor(
        &self,
        reconnect_time_ms: u32,
        attempt: u32,
        jitter_factor: f32,
    ) -> Option<Duration> {
        if self.max_retries <= attempt {
            return None;
        }

        debug_assert!(self.min_sleep_ms <= self.max_backoff_ms);

        let reconnect_time_ms = reconnect_time_ms.max(self.min_sleep_ms) as f32;
        let mut sleep_ms =
            match self.backoff_multiplier.is_finite() && 1.0 <= self.backoff_multiplier {
                true => {
                    reconnect_time_ms
                        * self
                            .backoff_multiplier
                            .powi(attempt.min(i32::MAX as _) as _)
                }
                false => reconnect_time_ms,
            };

        if !sleep_ms.is_finite() || (self.max_backoff_ms as f32) <= sleep_ms {
            sleep_ms = self.max_backoff_ms as _;
        }

        if self.jitter && reconnect_time_ms < sleep_ms {
            let jitter_factor =
                match jitter_factor.is_finite() && (0.0..=1.0).contains(&jitter_factor) {
                    true => jitter_factor,
                    false => 1.0,
                };
            sleep_ms = reconnect_time_ms + jitter_factor * (sleep_ms - reconnect_time_ms);
        }

        Some(Duration::from_millis(sleep_ms as _))
    }
}

impl Default for SseRetryConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_backoff() {
        let config = SseRetryConfig::new();
        let dur = config.calculate_backoff_with_factor(3000, 0, 0.0);
        assert!(dur.is_some());
        // With jitter_factor=0.0, should be exactly the reconnect_time (min_sleep clamped)
        assert_eq!(dur.unwrap(), Duration::from_millis(3000));
    }

    #[test]
    fn disabled_returns_none() {
        let config = SseRetryConfig::disabled();
        assert!(config.calculate_backoff_with_factor(3000, 0, 0.5).is_none());
    }

    #[test]
    fn respects_max_retries() {
        let config = SseRetryConfig {
            max_retries: 3,
            ..SseRetryConfig::new()
        };
        assert!(config.calculate_backoff_with_factor(1000, 3, 0.0).is_none());
        assert!(config.calculate_backoff_with_factor(1000, 2, 0.0).is_some());
    }
}
