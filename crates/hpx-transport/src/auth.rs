//! Per-message dynamic authentication for exchange WebSocket sessions.
//!
//! Some venues do not authenticate a connection once. Coinbase Advanced Trade,
//! for example, requires a fresh ES256 JWT whose `exp` is 120 seconds after
//! issuance, carried on each `subscribe` message. A static `Authorization`
//! header cannot model that, and caching a token for the whole connection
//! silently turns into auth failures after two minutes.
//!
//! [`MessageSigner`] is the hook an adapter implements; [`SignedMessage`]
//! carries the token together with its issuance time so callers can check
//! expiry deterministically against an injected clock.

use std::{
    fmt,
    sync::{Mutex, PoisonError},
};

/// Failures produced by a [`MessageSigner`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// The credential is missing or malformed.
    InvalidCredential,
    /// Signing failed.
    SigningFailed,
    /// The signer refuses to issue a token at this time.
    NotReady,
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredential => f.write_str("invalid credential"),
            Self::SigningFailed => f.write_str("signing failed"),
            Self::NotReady => f.write_str("signer not ready"),
        }
    }
}

impl std::error::Error for AuthError {}

/// A token bound to the message it was issued for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedMessage {
    token: String,
    issued_at_ms: u64,
    ttl_ms: u64,
}

impl SignedMessage {
    /// Create a signed message.
    ///
    /// Pass `ttl_ms = 0` for a token that does not expire.
    #[inline]
    #[must_use]
    pub const fn new(token: String, issued_at_ms: u64, ttl_ms: u64) -> Self {
        Self {
            token,
            issued_at_ms,
            ttl_ms,
        }
    }

    /// The signed token.
    #[inline]
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Issuance timestamp in milliseconds.
    #[inline]
    #[must_use]
    pub const fn issued_at_ms(&self) -> u64 {
        self.issued_at_ms
    }

    /// Token lifetime in milliseconds.
    ///
    /// Zero means the token does not expire, which is the common case for a
    /// static bearer token.
    #[inline]
    #[must_use]
    pub const fn ttl_ms(&self) -> u64 {
        self.ttl_ms
    }

    /// Whether the token has expired at `now_ms`.
    ///
    /// A zero [`SignedMessage::ttl_ms`] means "never expires" and always
    /// returns `false`.
    #[inline]
    #[must_use]
    pub const fn is_expired_at(&self, now_ms: u64) -> bool {
        self.ttl_ms != 0 && now_ms >= self.issued_at_ms.saturating_add(self.ttl_ms)
    }

    /// Milliseconds until expiry.
    ///
    /// Returns [`u64::MAX`] for a token that never expires, and zero once it
    /// has expired.
    #[inline]
    #[must_use]
    pub const fn remaining_ms(&self, now_ms: u64) -> u64 {
        if self.ttl_ms == 0 {
            return u64::MAX;
        }
        self.issued_at_ms
            .saturating_add(self.ttl_ms)
            .saturating_sub(now_ms)
    }
}

/// Coinbase Advanced Trade WebSocket JWT lifetime, in milliseconds.
pub const COINBASE_WS_JWT_TTL_MS: u64 = 120_000;

/// Hook for venues that sign each outbound subscription independently.
pub trait MessageSigner: Send + Sync {
    /// Sign `payload` at `now_ms`.
    ///
    /// Implementations must issue a **fresh** token (with a fresh nonce where
    /// required) on every call; the transport layer never caches the result.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError`] when the credential is unusable or signing fails.
    fn sign(&self, payload: &[u8], now_ms: u64) -> Result<SignedMessage, AuthError>;

    /// Token lifetime in milliseconds.
    fn ttl_ms(&self) -> u64;
}

/// A simple static bearer token, useful for venues that authenticate once.
#[derive(Clone, Debug)]
pub struct StaticToken {
    token: String,
    ttl_ms: u64,
}

impl StaticToken {
    /// Create a static token.
    ///
    /// `ttl_ms = 0` means the token never expires.
    #[inline]
    #[must_use]
    pub const fn new(token: String, ttl_ms: u64) -> Self {
        Self { token, ttl_ms }
    }
}

impl MessageSigner for StaticToken {
    fn sign(&self, _payload: &[u8], now_ms: u64) -> Result<SignedMessage, AuthError> {
        Ok(SignedMessage::new(self.token.clone(), now_ms, self.ttl_ms))
    }

    fn ttl_ms(&self) -> u64 {
        self.ttl_ms
    }
}

/// A rate-aware signer that refuses to reuse a token beyond a safety margin.
///
/// This is a guard, not a cache: if the caller asks for a new signature before
/// the previous one has exceeded `min_interval_ms`, it returns
/// [`AuthError::NotReady`] rather than silently reusing a token.
///
/// The check and the signing call are serialized, so the interval holds even
/// when several tasks sign concurrently. A failed signing call does not update
/// the interval.
#[derive(Debug)]
pub struct NonReusableSigner<F> {
    sign: F,
    ttl_ms: u64,
    min_interval_ms: u64,
    last_issued_ms: Mutex<Option<u64>>,
}

impl<F> NonReusableSigner<F>
where
    F: Fn(&[u8], u64) -> Result<String, AuthError> + Send + Sync,
{
    /// Wrap a signing closure with `ttl_ms` and a minimum reissue interval.
    #[must_use]
    pub const fn new(sign: F, ttl_ms: u64, min_interval_ms: u64) -> Self {
        Self {
            sign,
            ttl_ms,
            min_interval_ms,
            last_issued_ms: Mutex::new(None),
        }
    }
}

impl<F> MessageSigner for NonReusableSigner<F>
where
    F: Fn(&[u8], u64) -> Result<String, AuthError> + Send + Sync,
{
    fn sign(&self, payload: &[u8], now_ms: u64) -> Result<SignedMessage, AuthError> {
        // Hold the lock across the signing call: the interval check and the
        // timestamp update must be one atomic decision, and a signer is not a
        // hot path. A poisoned lock cannot leave `Option<u64>` inconsistent.
        let mut last_issued = self
            .last_issued_ms
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = *last_issued
            && now_ms.saturating_sub(previous) < self.min_interval_ms
        {
            return Err(AuthError::NotReady);
        }
        let token = (self.sign)(payload, now_ms)?;
        *last_issued = Some(now_ms);
        Ok(SignedMessage::new(token, now_ms, self.ttl_ms))
    }

    fn ttl_ms(&self) -> u64 {
        self.ttl_ms
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    use super::*;

    #[test]
    fn signed_message_expiry_is_deterministic() {
        let msg = SignedMessage::new("jwt".into(), 1_000, COINBASE_WS_JWT_TTL_MS);
        assert_eq!(msg.token(), "jwt");
        assert_eq!(msg.ttl_ms(), 120_000);
        assert_eq!(msg.remaining_ms(1_000), 120_000);
        assert!(!msg.is_expired_at(120_999));
        assert!(msg.is_expired_at(121_000));
        assert_eq!(msg.remaining_ms(200_000), 0);
    }

    #[test]
    fn static_token_reports_its_lifetime() {
        let signer = StaticToken::new("secret".into(), 0);
        assert_eq!(signer.ttl_ms(), 0);
        let signed = signer.sign(b"subscribe", 42).expect("sign");
        assert_eq!(signed.token(), "secret");
        assert_eq!(signed.issued_at_ms(), 42);
        assert!(!signed.is_expired_at(u64::MAX), "ttl 0 never expires");
        assert_eq!(signed.remaining_ms(u64::MAX), u64::MAX);
    }

    #[test]
    fn signer_issues_a_fresh_token_per_subscription() {
        let signer = NonReusableSigner::new(
            |payload, now| Ok(format!("{}.{}", String::from_utf8_lossy(payload), now)),
            COINBASE_WS_JWT_TTL_MS,
            1,
        );
        let a = signer.sign(b"level2.BTC-USD", 1_000).expect("first");
        let b = signer.sign(b"level2.ETH-USD", 1_001).expect("second");
        assert_ne!(a.token(), b.token());
        assert_eq!(b.issued_at_ms(), 1_001);
    }

    #[test]
    fn non_reusable_signer_rejects_immediate_reuse() {
        let signer = NonReusableSigner::new(
            |_, now| Ok(format!("t{now}")),
            COINBASE_WS_JWT_TTL_MS,
            1_000,
        );
        assert!(signer.sign(b"a", 10_000).is_ok());
        assert_eq!(signer.sign(b"a", 10_500), Err(AuthError::NotReady));
        assert!(signer.sign(b"a", 11_000).is_ok());
    }

    #[test]
    fn non_reusable_signer_enforces_the_interval_from_time_zero() {
        let signer = NonReusableSigner::new(|_, now| Ok(format!("t{now}")), 60_000, 1_000);
        assert!(signer.sign(b"a", 0).is_ok());
        assert_eq!(signer.sign(b"a", 0), Err(AuthError::NotReady));
        assert!(signer.sign(b"a", 1_000).is_ok());
    }

    #[test]
    fn non_reusable_signer_is_atomic_across_threads() {
        let signer = Arc::new(NonReusableSigner::new(
            |_, now| Ok(format!("t{now}")),
            60_000,
            1_000,
        ));
        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let signer = Arc::clone(&signer);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                signer.sign(b"a", 10_000).is_ok()
            }));
        }
        let successes = handles
            .into_iter()
            .map(|handle| handle.join().expect("worker"))
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 1, "only one token may be issued per interval");
    }

    #[test]
    fn signer_errors_propagate() {
        let signer = NonReusableSigner::new(|_, _| Err(AuthError::SigningFailed), 120_000, 0);
        assert_eq!(signer.sign(b"x", 1), Err(AuthError::SigningFailed));
        assert_eq!(signer.ttl_ms(), 120_000);
    }

    #[test]
    fn auth_errors_have_messages() {
        assert_eq!(
            format!("{}", AuthError::InvalidCredential),
            "invalid credential"
        );
        assert_eq!(format!("{}", AuthError::NotReady), "signer not ready");
    }
}
