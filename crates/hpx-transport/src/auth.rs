//! Authentication strategies for exchange APIs.
//!
//! This module provides various authentication methods commonly used by
//! cryptocurrency exchanges.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use http::header::HeaderMap;

use crate::error::{TransportError, TransportResult};

/// Authentication trait for signing requests.
#[async_trait]
pub trait Authentication: Send + Sync {
    /// Sign a request by modifying headers and optionally returning query parameters.
    ///
    /// # Arguments
    /// * `method` - HTTP method
    /// * `path` - Request path (without base URL)
    /// * `headers` - Mutable headers to modify
    /// * `body` - Optional request body bytes
    ///
    /// # Returns
    /// Optional query string to append to the URL
    async fn sign(
        &self,
        method: &http::Method,
        path: &str,
        headers: &mut HeaderMap,
        body: Option<&[u8]>,
    ) -> TransportResult<Option<String>>;

    /// Generate WebSocket authentication message (if needed).
    fn ws_auth_message(&self) -> Option<String> {
        None
    }
}

/// No authentication.
#[derive(Debug, Clone, Default)]
pub struct NoAuth;

#[async_trait]
impl Authentication for NoAuth {
    async fn sign(
        &self,
        _method: &http::Method,
        _path: &str,
        _headers: &mut HeaderMap,
        _body: Option<&[u8]>,
    ) -> TransportResult<Option<String>> {
        Ok(None)
    }
}

/// API Key authentication (via header or query parameter).
#[derive(Debug, Clone)]
pub struct ApiKeyAuth {
    key_name: String,
    key_value: String,
    location: ApiKeyLocation,
}

/// Where to place the API key.
#[derive(Debug, Clone)]
pub enum ApiKeyLocation {
    /// Place in HTTP header
    Header,
    /// Place in query parameter
    Query,
}

impl ApiKeyAuth {
    /// Create API key authentication via header.
    pub fn header(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key_name: name.into(),
            key_value: value.into(),
            location: ApiKeyLocation::Header,
        }
    }

    /// Create API key authentication via query parameter.
    pub fn query(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key_name: name.into(),
            key_value: value.into(),
            location: ApiKeyLocation::Query,
        }
    }
}

#[async_trait]
impl Authentication for ApiKeyAuth {
    async fn sign(
        &self,
        _method: &http::Method,
        _path: &str,
        headers: &mut HeaderMap,
        _body: Option<&[u8]>,
    ) -> TransportResult<Option<String>> {
        match self.location {
            ApiKeyLocation::Header => {
                let header_value = http::header::HeaderValue::from_str(&self.key_value)
                    .map_err(|e| TransportError::auth(e.to_string()))?;
                let header_name = http::header::HeaderName::from_bytes(self.key_name.as_bytes())
                    .map_err(|e| TransportError::auth(e.to_string()))?;
                headers.insert(header_name, header_value);
                Ok(None)
            }
            ApiKeyLocation::Query => Ok(Some(format!("{}={}", self.key_name, self.key_value))),
        }
    }
}

/// Bearer token authentication.
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
    async fn sign(
        &self,
        _method: &http::Method,
        _path: &str,
        headers: &mut HeaderMap,
        _body: Option<&[u8]>,
    ) -> TransportResult<Option<String>> {
        let auth_value = format!("Bearer {}", self.token);
        let header_value = http::header::HeaderValue::from_str(&auth_value)
            .map_err(|e| TransportError::auth(e.to_string()))?;
        headers.insert(http::header::AUTHORIZATION, header_value);
        Ok(None)
    }
}

/// HMAC algorithm for signing.
#[derive(Debug, Clone, Copy)]
pub enum HmacAlgorithm {
    /// SHA-256
    Sha256,
    /// SHA-512
    Sha512,
}

/// HMAC signature authentication (commonly used by exchanges like Binance, OKX).
#[derive(Clone)]
pub struct HmacAuth {
    api_key: String,
    secret_key: String,
    algorithm: HmacAlgorithm,
    /// Header name for API key
    api_key_header: String,
    /// Header name for signature
    signature_header: Option<String>,
    /// Query parameter name for signature (if not using header)
    signature_param: Option<String>,
    /// Header/Query name for timestamp
    timestamp_param: String,
    /// Whether to include timestamp in query
    timestamp_in_query: bool,
}

impl std::fmt::Debug for HmacAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HmacAuth")
            .field("api_key", &"***")
            .field("secret_key", &"***")
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

impl HmacAuth {
    /// Create HMAC authentication with default settings (Binance-style).
    pub fn new(api_key: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            secret_key: secret_key.into(),
            algorithm: HmacAlgorithm::Sha256,
            api_key_header: "X-MBX-APIKEY".to_string(),
            signature_header: None,
            signature_param: Some("signature".to_string()),
            timestamp_param: "timestamp".to_string(),
            timestamp_in_query: true,
        }
    }

    /// Set the HMAC algorithm.
    pub fn algorithm(mut self, algorithm: HmacAlgorithm) -> Self {
        self.algorithm = algorithm;
        self
    }

    /// Set the API key header name.
    pub fn api_key_header(mut self, name: impl Into<String>) -> Self {
        self.api_key_header = name.into();
        self
    }

    /// Set signature to be placed in header.
    pub fn signature_header(mut self, name: impl Into<String>) -> Self {
        self.signature_header = Some(name.into());
        self.signature_param = None;
        self
    }

    /// Set signature to be placed in query parameter.
    pub fn signature_param(mut self, name: impl Into<String>) -> Self {
        self.signature_param = Some(name.into());
        self.signature_header = None;
        self
    }

    /// Set timestamp parameter name.
    pub fn timestamp_param(mut self, name: impl Into<String>) -> Self {
        self.timestamp_param = name.into();
        self
    }

    /// Set whether timestamp should be in query.
    pub fn timestamp_in_query(mut self, in_query: bool) -> Self {
        self.timestamp_in_query = in_query;
        self
    }

    fn get_timestamp_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn sign_message(&self, message: &str) -> TransportResult<String> {
        use hmac::{Hmac, Mac};

        match self.algorithm {
            HmacAlgorithm::Sha256 => {
                type HmacSha256 = Hmac<sha2::Sha256>;
                let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
                    .map_err(|_| TransportError::auth("Invalid secret key length"))?;
                mac.update(message.as_bytes());
                Ok(hex::encode(mac.finalize().into_bytes()))
            }
            HmacAlgorithm::Sha512 => {
                type HmacSha512 = Hmac<sha2::Sha512>;
                let mut mac = HmacSha512::new_from_slice(self.secret_key.as_bytes())
                    .map_err(|_| TransportError::auth("Invalid secret key length"))?;
                mac.update(message.as_bytes());
                Ok(hex::encode(mac.finalize().into_bytes()))
            }
        }
    }
}

#[async_trait]
impl Authentication for HmacAuth {
    async fn sign(
        &self,
        _method: &http::Method,
        _path: &str,
        headers: &mut HeaderMap,
        body: Option<&[u8]>,
    ) -> TransportResult<Option<String>> {
        let timestamp = Self::get_timestamp_ms();

        // Add API key header
        let api_key_value = http::header::HeaderValue::from_str(&self.api_key)
            .map_err(|e| TransportError::auth(e.to_string()))?;
        let api_key_name = http::header::HeaderName::from_bytes(self.api_key_header.as_bytes())
            .map_err(|e| TransportError::auth(e.to_string()))?;
        headers.insert(api_key_name, api_key_value);

        // Build message to sign
        let body_str = body
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();

        let message = if self.timestamp_in_query {
            format!("{}={}&{}", self.timestamp_param, timestamp, body_str)
        } else {
            body_str.clone()
        };

        let signature = self.sign_message(&message)?;

        // Add signature
        if let Some(ref header_name) = self.signature_header {
            let sig_value = http::header::HeaderValue::from_str(&signature)
                .map_err(|e| TransportError::auth(e.to_string()))?;
            let sig_name = http::header::HeaderName::from_bytes(header_name.as_bytes())
                .map_err(|e| TransportError::auth(e.to_string()))?;
            headers.insert(sig_name, sig_value);

            if !self.timestamp_in_query {
                let ts_value = http::header::HeaderValue::from_str(&timestamp.to_string())
                    .map_err(|e| TransportError::auth(e.to_string()))?;
                let ts_name = http::header::HeaderName::from_bytes(self.timestamp_param.as_bytes())
                    .map_err(|e| TransportError::auth(e.to_string()))?;
                headers.insert(ts_name, ts_value);
            }
            Ok(None)
        } else if let Some(ref param_name) = self.signature_param {
            Ok(Some(format!(
                "{}={}&{}={}",
                self.timestamp_param, timestamp, param_name, signature
            )))
        } else {
            Ok(None)
        }
    }
}

/// Composite authentication that combines two authenticators.
///
/// For more than two authenticators, nest `CompositeAuth` instances:
/// ```rust
/// use hpx_transport::auth::{ApiKeyAuth, BearerAuth, CompositeAuth, NoAuth};
///
/// let auth = CompositeAuth::new(
///     ApiKeyAuth::header("X-API-KEY", "key"),
///     BearerAuth::new("token"),
/// );
/// ```
#[derive(Debug, Clone)]
pub struct CompositeAuth<A, B> {
    first: A,
    second: B,
}

impl<A, B> CompositeAuth<A, B>
where
    A: Authentication,
    B: Authentication,
{
    /// Create a new composite authentication from two authenticators.
    pub fn new(first: A, second: B) -> Self {
        Self { first, second }
    }
}

#[async_trait]
impl<A, B> Authentication for CompositeAuth<A, B>
where
    A: Authentication,
    B: Authentication,
{
    async fn sign(
        &self,
        method: &http::Method,
        path: &str,
        headers: &mut HeaderMap,
        body: Option<&[u8]>,
    ) -> TransportResult<Option<String>> {
        let q1 = self.first.sign(method, path, headers, body).await?;
        let q2 = self.second.sign(method, path, headers, body).await?;

        match (q1, q2) {
            (Some(a), Some(b)) => Ok(Some(format!("{}&{}", a, b))),
            (Some(a), None) => Ok(Some(a)),
            (None, Some(b)) => Ok(Some(b)),
            (None, None) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_no_auth() {
        let auth = NoAuth;
        let mut headers = HeaderMap::new();
        let result = auth
            .sign(&http::Method::GET, "/test", &mut headers, None)
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_api_key_header() {
        let auth = ApiKeyAuth::header("X-API-KEY", "my-key");
        let mut headers = HeaderMap::new();
        let result = auth
            .sign(&http::Method::GET, "/test", &mut headers, None)
            .await;
        assert!(result.is_ok());
        assert!(headers.contains_key("X-API-KEY"));
    }

    #[tokio::test]
    async fn test_api_key_query() {
        let auth = ApiKeyAuth::query("api_key", "my-key");
        let mut headers = HeaderMap::new();
        let result = auth
            .sign(&http::Method::GET, "/test", &mut headers, None)
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some("api_key=my-key".to_string()));
    }

    #[tokio::test]
    async fn test_bearer_auth() {
        let auth = BearerAuth::new("my-token");
        let mut headers = HeaderMap::new();
        let result = auth
            .sign(&http::Method::GET, "/test", &mut headers, None)
            .await;
        assert!(result.is_ok());
        assert!(headers.contains_key(http::header::AUTHORIZATION));
    }
}
