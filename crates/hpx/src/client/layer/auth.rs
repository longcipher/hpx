//! Authentication middleware for HTTP requests.
//!
//! This module provides built-in authentication support with automatic token
//! refresh for Bearer tokens, API keys, and OAuth2 flows.

use std::{
    fmt,
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
        self.expires_at.is_some_and(|exp| Instant::now() >= exp)
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
        #[expect(clippy::type_complexity)]
        applier: Arc<dyn Fn(&mut HeaderMap) -> Result<(), Error> + Send + Sync>,
    },
}

impl fmt::Debug for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer { .. } => f.debug_struct("Bearer").finish_non_exhaustive(),
            Self::BearerProvider { .. } => f.debug_struct("BearerProvider").finish_non_exhaustive(),
            Self::ApiKey { header_name, .. } => f
                .debug_struct("ApiKey")
                .field("header_name", header_name)
                .finish_non_exhaustive(),
            Self::Custom { .. } => f.debug_struct("Custom").finish_non_exhaustive(),
        }
    }
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
    /// The provider is awaited before every request, so the token can be
    /// refreshed asynchronously. Implement [`CachedTokenProvider`] to avoid
    /// fetching a token for every request.
    ///
    /// # Example
    /// ```
    /// # extern crate hpx;
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::sync::Arc;
    ///
    /// use hpx::auth::{AuthMethod, BearerTokenProvider, Token};
    ///
    /// struct MyProvider;
    ///
    /// #[async_trait::async_trait]
    /// impl BearerTokenProvider for MyProvider {
    ///     async fn get_token(&self) -> Result<Token, hpx::Error> {
    ///         Ok(Token::permanent("secret".to_owned()))
    ///     }
    /// }
    ///
    /// let auth = AuthMethod::bearer_provider(Arc::new(MyProvider));
    /// # Ok(())
    /// # }
    /// # fn main() {}
    /// ```
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
    #[expect(
        clippy::expect_used,
        reason = "header name validity is caller invariant"
    )]
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
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
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
        let auth = self.auth.clone();

        // A token provider is asynchronous, so the token is fetched before
        // the request is handed to the inner service.
        if let AuthMethod::BearerProvider { provider } = auth.as_ref() {
            let provider = provider.clone();
            let mut inner = self.inner.clone();
            return Box::pin(async move {
                let token = provider.get_token().await.map_err(|err| {
                    trace!("Auth failed: {}", err);
                    crate::error::BoxError::from(err)
                })?;

                apply_bearer(&token.value, req.headers_mut()).map_err(|err| {
                    trace!("Auth failed: {}", err);
                    crate::error::BoxError::from(err)
                })?;

                trace!("Auth applied to {} {}", req.method(), req.uri());
                inner.call(req).await.map_err(Into::into)
            });
        }

        // Apply authentication headers
        if let Err(e) = apply_auth(&auth, &mut req) {
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
            apply_bearer(token, req.headers_mut())?;
        }
        AuthMethod::BearerProvider { .. } => {
            // The provider is awaited by the service, which is the only place
            // that can suspend.
            return Err(Error::builder(
                Box::<dyn std::error::Error + Send + Sync>::from(
                    "BearerProvider auth must be applied by the service, not synchronously.",
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

/// Writes the `Authorization: Bearer` header.
fn apply_bearer(token: &str, headers: &mut HeaderMap) -> Result<(), Error> {
    headers.insert(
        http::header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| Error::builder(Box::<dyn std::error::Error + Send + Sync>::from(e)))?,
    );
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
    token: Arc<parking_lot::RwLock<Option<Token>>>,
}

impl<F, Fut> fmt::Debug for CachedTokenProvider<F, Fut>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<Token, Error>> + Send,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedTokenProvider")
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
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
            token: Arc::new(parking_lot::RwLock::new(None)),
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
            let cached = self.token.read();
            if let Some(ref token) = *cached
                && !token.is_expired()
            {
                return Ok(token.clone());
            }
        }

        // Refresh
        let new_token = (self.refresh_fn)().await?;
        {
            let mut write = self.token.write();
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
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    /// A provider that yields a token and counts how often it was called.
    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl BearerTokenProvider for CountingProvider {
        async fn get_token(&self) -> Result<Token, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Token::permanent("refreshed-token"))
        }
    }

    /// A service that records the `Authorization` header it received.
    #[derive(Clone)]
    struct RecordingService {
        seen: Arc<Mutex<Vec<HeaderValue>>>,
    }

    impl Service<Request<Body>> for RecordingService {
        type Response = Response<Body>;
        type Error = crate::error::BoxError;
        type Future = BoxFut<Result<Response<Body>, crate::error::BoxError>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: Request<Body>) -> Self::Future {
            if let Some(value) = req.headers().get(http::header::AUTHORIZATION) {
                self.seen.lock().unwrap().push(value.clone());
            }
            Box::pin(async move { Ok(Response::new(Body::empty())) })
        }
    }

    /// The token provider is awaited before the request is sent.
    #[tokio::test]
    async fn bearer_provider_is_awaited() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(CountingProvider {
            calls: calls.clone(),
        });

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut service = AuthLayer::new(AuthMethod::bearer_provider(provider))
            .layer(RecordingService { seen: seen.clone() });

        let req = Request::new(Body::empty());
        service.call(req).await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], "Bearer refreshed-token");
    }

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
