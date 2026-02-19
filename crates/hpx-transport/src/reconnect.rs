use std::time::Duration;

use rand::RngExt;

/// Shared reconnect/backoff configuration used by stream transports.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BackoffConfig {
    pub(crate) initial_delay: Duration,
    pub(crate) max_delay: Duration,
    pub(crate) factor: f64,
    pub(crate) jitter: f64,
}

impl BackoffConfig {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.initial_delay.is_zero() {
            return Err("Initial reconnect delay must be > 0".to_string());
        }
        if self.max_delay.is_zero() {
            return Err("Max reconnect delay must be > 0".to_string());
        }
        if self.max_delay < self.initial_delay {
            return Err("Max reconnect delay must be >= initial reconnect delay".to_string());
        }
        if self.factor < 1.0 || !self.factor.is_finite() {
            return Err("Backoff factor must be >= 1.0".to_string());
        }
        if !(0.0..=1.0).contains(&self.jitter) || !self.jitter.is_finite() {
            return Err("Jitter must be between 0.0 and 1.0".to_string());
        }
        Ok(())
    }
}

pub(crate) fn calculate_backoff(config: BackoffConfig, attempt: u32) -> Duration {
    let initial = config.initial_delay.as_secs_f64();
    let max = config.max_delay.as_secs_f64();
    let exponent = config.factor.powf(f64::from(attempt));
    let base = (initial * exponent).min(max);

    if config.jitter == 0.0 {
        return Duration::from_secs_f64(base);
    }

    let mut rng = rand::rng();
    let randomized = rng.random_range(0.0..=base);
    let blended = base * (1.0 - config.jitter) + randomized * config.jitter;
    Duration::from_secs_f64(blended)
}
