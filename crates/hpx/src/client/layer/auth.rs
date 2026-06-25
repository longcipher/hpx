//! Authentication middleware for HTTP requests.
//!
//! This module provides built-in authentication support with automatic token
//! refresh for Bearer tokens, API keys, and OAuth2 flows.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use hpx::auth::{AuthMethod, BearerTokenProvider, Token};
//!
//! # async fn example() -> hpx::Result<()> {
//! // Simple static bearer token
//! let client = hpx::Client::builder()
//!     .auth(AuthMethod::bearer("my-secret-token"))
//!     .build()?;
//!
//! // API key in header
//! let client = hpx::Client::builder()
//!     .auth(AuthMethod::api_key("X-API-KEY", "my-api-key"))
//!     .build()?;
//!
//! # Ok(())
//! # }
//! ```

use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use http::{HeaderMap, HeaderName, HeaderValue, Request, Response};
use tower::{Layer, Service};

use crate::{Body, Error};

/// A token with an optional expiration time.
#[derive(Clone, Debug)]
pub struct Token {
    /// The token value.
    pub value: String,
    /// When the token expires. `None` means it never expires.
    pub expires_at: Option<Instant>,
}

impl Token {
    /// Create a new token that never expires.
    pub fn permanent(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            expires_at: None,
        }
    }

    /// Create a new token with a TTL (time-to-live) from now.
    pub fn with_ttl(value: impl Into<String>, ttl: Duration) -> Self {
        Self {
            value: value.into(),
            expires_at: Some(Instant::now() + ttl),
        }
    }

    /// Create a new token with an absolute expiration timestamp (Unix seconds).
    pub fn with_expires_at_secs(value: impl Into<String>, expires_at_secs: u64) -> Self {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let remaining = expires_at_secs.saturating_sub(now_secs);
        Self {
            value: value.into(),
            expires_at: Some(Instant::now() + Duration::from_secs(remaining)),
        }
    }

    /// Returns `true` if the token has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| Instant::now() >= exp)
            .unwrap_or(false)
    }
}

/// A provider that supplies authentication tokens with optional auto-refresh.
///
/// Implement this trait to integrate with OAuth2, JWT, or any custom token
/// refresh mechanism.
#[async_trait::async_trait]
pub trait BearerTokenProvider: Send + Sync {
    /// Get a valid token. Called once per request (or when the cached token expires).
    async fn get_token(&self) -> Result<Token, Error>;
}

/// Configuration for bearer token authentication.
pub enum AuthMethod {
    /// Static bearer token.
    Bearer {
        /// The token value.
        token: String,
    },
    /// Dynamic bearer token provider with auto-refresh.
    BearerProvider {
        /// Token provider.
        provider: Arc<dyn BearerTokenProvider>,
    },
    /// API key in a custom header.
    ApiKey {
        /// Header name (e.g., `X-API-KEY`).
        header_name: HeaderName,
        /// Header value.
        header_value: String,
    },
    /// Custom authentication via a closure.
    Custom {
        /// Function to apply auth to request headers.
        #[allow(clippy::type_complexity)]
        applier: Arc<dyn Fn(&mut HeaderMap) -> Result<(), Error> + Send + Sync>,
    },
}

impl AuthMethod {
    /// Create a static bearer token authentication method.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self::Bearer {
            token: token.into(),
        }
    }

    /// Create a dynamic bearer token provider authentication method.
    ///
    /// # Warning
    ///
    /// This variant requires an async token refresh mechanism. The auth layer
    /// operates synchronously, so this method returns an error at request time.
    /// Use [`CachedTokenProvider`] or hooks-based auth instead.
    #[deprecated(
        since = "2.5.0",
        note = "BearerProvider cannot be used synchronously. Use `cached_token_provider()` or hooks instead."
    )]
    pub fn bearer_provider(provider: Arc<dyn BearerTokenProvider>) -> Self {
        Self::BearerProvider { provider }
    }

    /// Create an API key authentication method.
    ///
    /// Returns `None` if the header name is invalid.
    pub fn try_api_key(name: impl TryInto<HeaderName>, value: impl Into<String>) -> Option<Self> {
        Some(Self::ApiKey {
            header_name: name.try_into().ok()?,
            header_value: value.into(),
        })
    }

    /// Create an API key authentication method with a pre-validated header name.
    pub fn api_key_with_name(name: HeaderName, value: impl Into<String>) -> Self {
        Self::ApiKey {
            header_name: name,
            header_value: value.into(),
        }
    }

    /// Create an API key authentication method.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not a valid HTTP header name.
    pub fn api_key(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self::ApiKey {
            header_name: HeaderName::try_from(name.into())
                .expect("invalid header name for API key"),
            header_value: value.into(),
        }
    }

    /// Create a custom authentication method using a closure.
    pub fn custom<F>(f: F) -> Self
    where
        F: Fn(&mut HeaderMap) -> Result<(), Error> + Send + Sync + 'static,
    {
        Self::Custom {
            applier: Arc::new(f),
        }
    }
}

/// Layer that adds authentication to requests.
#[derive(Clone)]
pub struct AuthLayer {
    auth: Arc<AuthMethod>,
}

impl AuthLayer {
    /// Create a new auth layer with the given authentication method.
    pub fn new(auth: AuthMethod) -> Self {
        Self {
            auth: Arc::new(auth),
        }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            auth: self.auth.clone(),
        }
    }
}

/// Tower service that adds authentication headers to requests.
#[derive(Clone)]
pub struct AuthService<S> {
    inner: S,
    auth: Arc<AuthMethod>,
}

type BoxFut<T> = std::pin::Pin<Box<dyn Future<Output = T> + Send>>;

impl<S, ResBody> Service<Request<Body>> for AuthService<S>
where
    S: Service<Request<Body>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Error: Into<crate::error::BoxError> + Send,
    S::Future: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = crate::error::BoxError;
    type Future = BoxFut<Result<Response<ResBody>, crate::error::BoxError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        // Apply authentication headers
        if let Err(e) = apply_auth(&self.auth, &mut req) {
            trace!("Auth failed: {}", e);
            return Box::pin(async move { Err(e.into()) });
        }

        trace!("Auth applied to {} {}", req.method(), req.uri());

        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await.map_err(Into::into) })
    }
}

fn apply_auth(auth: &AuthMethod, req: &mut Request<Body>) -> Result<(), Error> {
    match auth {
        AuthMethod::Bearer { token } => {
            req.headers_mut().insert(
                http::header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| {
                    Error::builder(Box::<dyn std::error::Error + Send + Sync>::from(e))
                })?,
            );
        }
        AuthMethod::BearerProvider { .. } => {
            // Provider-based auth requires async, handled via blocking_get_token
            // in a spawn_blocking context. For simplicity, we use the blocking approach here.
            // In production, consider using a cached token with ArcSwap.
            // The provider call happens synchronously via a cached token pattern.
            // Users should implement their own caching layer or use `ArcSwap`.
            //
            // For now, we'll panic in the provider case since it requires async.
            // The recommended approach is to use hooks for provider-based auth.
            return Err(Error::builder(
                Box::<dyn std::error::Error + Send + Sync>::from(
                    "BearerProvider auth requires async token refresh. \
                     Use hooks or implement a cached token provider with ArcSwap.",
                ),
            ));
        }
        AuthMethod::ApiKey {
            header_name,
            header_value,
        } => {
            req.headers_mut().insert(
                header_name.clone(),
                HeaderValue::from_str(header_value).map_err(|e| {
                    Error::builder(Box::<dyn std::error::Error + Send + Sync>::from(e))
                })?,
            );
        }
        AuthMethod::Custom { applier } => {
            applier(req.headers_mut())?;
        }
    }
    Ok(())
}

// Built-in: Cached bearer token provider with auto-refresh
/// A thread-safe cached token provider that automatically refreshes expired tokens.
pub struct CachedTokenProvider<F, Fut>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<Token, Error>> + Send,
{
    refresh_fn: F,
    token: Arc<tokio::sync::RwLock<Option<Token>>>,
}

impl<F, Fut> CachedTokenProvider<F, Fut>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Token, Error>> + Send + 'static,
{
    /// Create a new cached token provider.
    pub fn new(refresh_fn: F) -> Self {
        Self {
            refresh_fn,
            token: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl<F, Fut> BearerTokenProvider for CachedTokenProvider<F, Fut>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<Token, Error>> + Send,
{
    async fn get_token(&self) -> Result<Token, Error> {
        // Check cached token
        {
            let cached = self.token.read().await;
            if let Some(ref token) = *cached
                && !token.is_expired()
            {
                return Ok(token.clone());
            }
        }

        // Refresh
        let new_token = (self.refresh_fn)().await?;
        {
            let mut write = self.token.write().await;
            *write = Some(new_token.clone());
        }
        Ok(new_token)
    }
}

/// Create a cached token provider.
pub fn cached_token_provider<F, Fut>(refresh_fn: F) -> CachedTokenProvider<F, Fut>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Token, Error>> + Send + 'static,
{
    CachedTokenProvider::new(refresh_fn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_permanent() {
        let token = Token::permanent("test");
        assert_eq!(token.value, "test");
        assert!(!token.is_expired());
        assert!(token.expires_at.is_none());
    }

    #[test]
    fn test_token_with_ttl() {
        let token = Token::with_ttl("test", Duration::from_secs(3600));
        assert_eq!(token.value, "test");
        assert!(!token.is_expired());
        assert!(token.expires_at.is_some());
    }

    #[test]
    fn test_token_expired() {
        let token = Token {
            value: "test".to_string(),
            expires_at: Some(Instant::now() - Duration::from_secs(1)),
        };
        assert!(token.is_expired());
    }

    #[test]
    fn test_auth_method_bearer() {
        let auth = AuthMethod::bearer("my-token");
        assert!(matches!(auth, AuthMethod::Bearer { .. }));
    }

    #[test]
    fn test_auth_method_api_key() {
        let auth = AuthMethod::api_key("X-API-KEY", "key123");
        assert!(matches!(auth, AuthMethod::ApiKey { .. }));
    }
}
