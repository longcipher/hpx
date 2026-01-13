//! Authentication and authorization abstractions.

use std::{fmt, time::SystemTime};

use async_trait::async_trait;

use crate::error::{AuthError, TransportResult};

/// Authentication trait for different authentication methods.
#[async_trait]
pub trait Authentication: Send + Sync + std::fmt::Debug {
    /// Authenticate a request by modifying it in place.
    async fn authenticate(&self, request: &mut dyn AuthenticatedRequest) -> TransportResult<()>;
}

/// A request that can be authenticated.
pub trait AuthenticatedRequest: Send {
    /// Add a header to the request.
    #[allow(clippy::result_large_err)]
    fn add_header(&mut self, name: &str, value: &str) -> TransportResult<()>;

    /// Get a header value.
    fn get_header(&self, name: &str) -> Option<&str>;

    /// Add a query parameter to the request.
    #[allow(clippy::result_large_err)]
    fn add_query(&mut self, name: &str, value: &str) -> TransportResult<()>;

    /// Get all query parameters.
    fn get_query_params(&self) -> Vec<(String, String)>;

    /// Get the request body for signing purposes.
    fn body(&self) -> Option<&[u8]>;

    /// Get the request URL for signing purposes.
    fn url(&self) -> &str;

    /// Get the request method.
    fn method(&self) -> &str;
}

/// No authentication
#[derive(Debug, Clone, Default)]
pub struct NoAuth;

#[async_trait]
impl Authentication for NoAuth {
    #[allow(clippy::result_large_err)]
    async fn authenticate(&self, _request: &mut dyn AuthenticatedRequest) -> TransportResult<()> {
        // No authentication needed
        Ok(())
    }
}

/// API Key authentication (via header or query parameter)
#[derive(Debug, Clone)]
pub struct ApiKeyAuth {
    key: String,
    value: String,
    location: ApiKeyLocation,
}

#[derive(Debug, Clone)]
pub enum ApiKeyLocation {
    Header,
    Query,
}

impl ApiKeyAuth {
    /// Create a new API key authentication for headers.
    pub fn header(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            location: ApiKeyLocation::Header,
        }
    }

    /// Create a new API key authentication for query parameters.
    pub fn query(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            location: ApiKeyLocation::Query,
        }
    }

    /// Create a new API key authentication with default header location.
    pub fn new(key: impl Into<String>) -> Self {
        Self::header("Authorization", key)
    }
}

#[async_trait]
impl Authentication for ApiKeyAuth {
    #[allow(clippy::result_large_err)]
    async fn authenticate(&self, request: &mut dyn AuthenticatedRequest) -> TransportResult<()> {
        match self.location {
            ApiKeyLocation::Header => {
                request.add_header(&self.key, &self.value)?;
            }
            ApiKeyLocation::Query => {
                request.add_query(&self.key, &self.value)?;
            }
        }
        Ok(())
    }
}

/// Bearer token authentication
#[derive(Debug, Clone)]
pub struct BearerAuth {
    token: String,
}

impl BearerAuth {
    /// Create a new bearer token authentication.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

#[async_trait]
impl Authentication for BearerAuth {
    #[allow(clippy::result_large_err)]
    async fn authenticate(&self, request: &mut dyn AuthenticatedRequest) -> TransportResult<()> {
        let auth_header = format!("Bearer {}", self.token);
        request.add_header("Authorization", &auth_header)?;
        Ok(())
    }
}

/// HMAC signature-based authentication
#[derive(Debug, Clone)]
pub struct HmacAuth {
    access_key: String,
    secret_key: String,
    algorithm: HmacAlgorithm,
}

#[derive(Debug, Clone)]
pub enum HmacAlgorithm {
    Sha256,
    Sha512,
}

impl HmacAuth {
    /// Create a new HMAC authentication with SHA-256.
    pub fn sha256(access_key: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            algorithm: HmacAlgorithm::Sha256,
        }
    }

    /// Create a new HMAC authentication with SHA-512.
    pub fn sha512(access_key: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            algorithm: HmacAlgorithm::Sha512,
        }
    }

    #[allow(clippy::result_large_err)]
    fn generate_signature(
        &self,
        request: &dyn AuthenticatedRequest,
        timestamp: u64,
    ) -> TransportResult<String> {
        use hmac::{Hmac, Mac};

        // Build signing string
        let method = request.method();
        let url = request.url();
        let body = request.body().unwrap_or(&[]);

        let string_to_sign = format!(
            "{}\n{}\n{}\n{}\n{}",
            method,
            url,
            timestamp,
            self.access_key,
            hex::encode(body)
        );

        match self.algorithm {
            HmacAlgorithm::Sha256 => {
                type HmacSha256 = Hmac<sha2::Sha256>;
                let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
                    .map_err(|_| AuthError::InvalidApiKey)?;
                mac.update(string_to_sign.as_bytes());
                Ok(hex::encode(mac.finalize().into_bytes()))
            }
            HmacAlgorithm::Sha512 => {
                type HmacSha512 = Hmac<sha2::Sha512>;
                let mut mac = HmacSha512::new_from_slice(self.secret_key.as_bytes())
                    .map_err(|_| AuthError::InvalidApiKey)?;
                mac.update(string_to_sign.as_bytes());
                Ok(hex::encode(mac.finalize().into_bytes()))
            }
        }
    }
}

#[async_trait]
impl Authentication for HmacAuth {
    #[allow(clippy::result_large_err)]
    async fn authenticate(&self, request: &mut dyn AuthenticatedRequest) -> TransportResult<()> {
        let timestamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let signature = self.generate_signature(request, timestamp)?;

        request.add_header("X-API-KEY", &self.access_key)?;
        request.add_header("X-SIGNATURE", &signature)?;
        request.add_header("X-TIMESTAMP", &timestamp.to_string())?;

        Ok(())
    }
}

/// JWT token authentication
#[derive(Debug, Clone)]
pub struct JwtAuth {
    token: String,
    #[allow(dead_code)]
    expires_at: Option<SystemTime>,
    #[allow(dead_code)]
    refresh_token: Option<String>,
}

impl JwtAuth {
    /// Create a new JWT authentication.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            expires_at: None,
            refresh_token: None,
        }
    }

    /// Create a new JWT authentication with expiration.
    pub fn with_expiration(
        token: impl Into<String>,
        expires_at: SystemTime,
        refresh_token: Option<String>,
    ) -> Self {
        Self {
            token: token.into(),
            expires_at: Some(expires_at),
            refresh_token,
        }
    }
}

#[async_trait]
impl Authentication for JwtAuth {
    #[allow(clippy::result_large_err)]
    async fn authenticate(&self, request: &mut dyn AuthenticatedRequest) -> TransportResult<()> {
        let auth_header = format!("Bearer {}", self.token);
        request.add_header("Authorization", &auth_header)?;
        Ok(())
    }
}

/// OAuth 2.0 authentication
#[derive(Debug, Clone)]
pub struct OAuth2Auth {
    access_token: String,
    token_type: String,
    #[allow(dead_code)]
    expires_at: Option<SystemTime>,
    #[allow(dead_code)]
    refresh_token: Option<String>,
    #[allow(dead_code)]
    refresh_url: Option<String>,
}

impl OAuth2Auth {
    /// Create a new OAuth 2.0 authentication.
    pub fn new(access_token: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            token_type: "Bearer".to_string(),
            expires_at: None,
            refresh_token: None,
            refresh_url: None,
        }
    }

    /// Create a new OAuth 2.0 authentication with refresh capability.
    pub fn with_refresh(
        access_token: impl Into<String>,
        expires_at: SystemTime,
        refresh_token: impl Into<String>,
        refresh_url: impl Into<String>,
    ) -> Self {
        Self {
            access_token: access_token.into(),
            token_type: "Bearer".to_string(),
            expires_at: Some(expires_at),
            refresh_token: Some(refresh_token.into()),
            refresh_url: Some(refresh_url.into()),
        }
    }
}

#[async_trait]
impl Authentication for OAuth2Auth {
    #[allow(clippy::result_large_err)]
    async fn authenticate(&self, request: &mut dyn AuthenticatedRequest) -> TransportResult<()> {
        let auth_header = format!("{} {}", self.token_type, self.access_token);
        request.add_header("Authorization", &auth_header)?;
        Ok(())
    }
}

/// Composite authentication that applies multiple authenticators
#[derive(Clone)]
pub struct CompositeAuth {
    authenticators: std::sync::Arc<Vec<Box<dyn Authentication>>>,
}

impl CompositeAuth {
    /// Create a new composite authentication.
    pub fn new() -> Self {
        Self {
            authenticators: std::sync::Arc::new(Vec::new()),
        }
    }

    /// Add an authenticator to the composite.
    pub fn with_auth<A: Authentication + 'static>(mut self, auth: A) -> Self {
        if let Some(authenticators) = std::sync::Arc::get_mut(&mut self.authenticators) {
            authenticators.push(Box::new(auth));
        } else {
            panic!("Cannot add authenticator to shared CompositeAuth");
        }
        self
    }
}

impl Default for CompositeAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CompositeAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompositeAuth")
            .field("authenticators", &self.authenticators.len())
            .finish()
    }
}

#[async_trait]
impl Authentication for CompositeAuth {
    #[allow(clippy::result_large_err)]
    async fn authenticate(&self, request: &mut dyn AuthenticatedRequest) -> TransportResult<()> {
        for auth in self.authenticators.iter() {
            auth.authenticate(request).await?;
        }
        Ok(())
    }
}
