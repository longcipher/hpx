//! Simplified one-liner API for common HTTP operations.
//!
//! This module provides a minimal, ergonomic API surface for users who want
//! quick HTTP requests without configuring a full `Client`. Think of it as
//! the `reqwest::blocking` equivalent but for async with hpx's performance.
//!
//! # Example
//!
//! ```rust,no_run
//! # async fn example() -> hpx::Result<()> {
//! // Simple GET
//! let resp = hpx::simple::get("https://httpbin.org/get").send().await?;
//! let body = resp.text().await?;
//!
//! // POST with JSON
//! use std::collections::HashMap;
//! let mut data = HashMap::new();
//! data.insert("key", "value");
//! let resp = hpx::simple::post("https://httpbin.org/post").send().await?;
//!
//! // GET with bearer token
//! let resp = hpx::simple::get("https://api.example.com/data")
//!     .bearer_token("my-token")
//!     .send()
//!     .await?;
//!
//! // GET with retries
//! let resp = hpx::simple::get("https://httpbin.org/get")
//!     .retry(3)
//!     .send()
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use crate::{Client, RequestBuilder, Result};

/// Create a simple GET request builder.
///
/// Returns a [`SimpleRequestBuilder`] with ergonomic methods for common options.
///
/// # Example
/// ```rust,no_run
/// # async fn example() -> hpx::Result<()> {
/// let body = hpx::simple::get("https://httpbin.org/get")
///     .await?
///     .text()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[inline]
pub fn get(url: impl Into<String>) -> SimpleRequestBuilder {
    SimpleRequestBuilder::new(url)
}

/// Create a simple POST request builder.
#[inline]
pub fn post(url: impl Into<String>) -> SimpleRequestBuilder {
    SimpleRequestBuilder::new(url).method(crate::Method::POST)
}

/// Create a simple PUT request builder.
#[inline]
pub fn put(url: impl Into<String>) -> SimpleRequestBuilder {
    SimpleRequestBuilder::new(url).method(crate::Method::PUT)
}

/// Create a simple DELETE request builder.
#[inline]
pub fn delete(url: impl Into<String>) -> SimpleRequestBuilder {
    SimpleRequestBuilder::new(url).method(crate::Method::DELETE)
}

/// Create a simple PATCH request builder.
#[inline]
pub fn patch(url: impl Into<String>) -> SimpleRequestBuilder {
    SimpleRequestBuilder::new(url).method(crate::Method::PATCH)
}

/// Create a simple HEAD request builder.
#[inline]
pub fn head(url: impl Into<String>) -> SimpleRequestBuilder {
    SimpleRequestBuilder::new(url).method(crate::Method::HEAD)
}

/// Create a simple OPTIONS request builder.
#[inline]
pub fn options(url: impl Into<String>) -> SimpleRequestBuilder {
    SimpleRequestBuilder::new(url).method(crate::Method::OPTIONS)
}

/// Convenience function: GET with JSON response deserialization.
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub async fn get_json<T: serde::de::DeserializeOwned>(url: impl Into<String>) -> Result<T> {
    get(url).send().await?.json().await
}

/// Convenience function: POST with JSON body and JSON response.
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub async fn post_json<T, B>(url: impl Into<String> + crate::IntoUri, body: &B) -> Result<T>
where
    T: serde::de::DeserializeOwned,
    B: serde::Serialize,
{
    let client = crate::Client::new();
    let resp = client.post(url).json(body).send().await?;
    resp.json().await
}

/// A simplified request builder with ergonomic one-liner methods.
pub struct SimpleRequestBuilder {
    url: String,
    method: crate::Method,
    bearer_token: Option<String>,
    api_key: Option<(String, String)>,
    timeout: Option<Duration>,
    headers: Vec<(String, String)>,
}

impl SimpleRequestBuilder {
    fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: crate::Method::GET,
            bearer_token: None,
            api_key: None,
            timeout: None,
            headers: Vec::new(),
        }
    }

    /// Set the HTTP method.
    #[inline]
    fn method(mut self, method: crate::Method) -> Self {
        self.method = method;
        self
    }

    /// Set a bearer token for authentication.
    #[inline]
    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    /// Set an API key in a custom header.
    #[inline]
    pub fn api_key(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.api_key = Some((name.into(), value.into()));
        self
    }

    /// Set a request timeout.
    #[inline]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Add a custom header.
    #[inline]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Send the request and return the response.
    pub async fn send(self) -> Result<crate::Response> {
        let client = Client::new();
        let mut builder = client.request(self.method, &self.url);

        // Apply bearer token
        if let Some(token) = &self.bearer_token {
            let value = format!("Bearer {token}");
            builder = builder.header(http::header::AUTHORIZATION, value);
        }

        // Apply API key
        if let Some((name, value)) = &self.api_key {
            builder = builder.header(name.as_str(), value.as_str());
        }

        // Apply custom headers
        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }

        // Apply timeout
        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }

        builder.send().await
    }

    /// Build the request without sending it.
    pub fn build(self) -> Result<RequestBuilder> {
        let client = Client::new();
        let mut builder = client.request(self.method, &self.url);

        if let Some(token) = &self.bearer_token {
            let value = format!("Bearer {token}");
            builder = builder.header(http::header::AUTHORIZATION, value);
        }

        if let Some((name, value)) = &self.api_key {
            builder = builder.header(name.as_str(), value.as_str());
        }

        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }

        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }

        Ok(builder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_get_builder() {
        let builder = get("https://example.com");
        assert_eq!(builder.url, "https://example.com");
        assert_eq!(builder.method, crate::Method::GET);
    }

    #[test]
    fn test_simple_post_builder() {
        let builder = post("https://example.com");
        assert_eq!(builder.method, crate::Method::POST);
    }

    #[test]
    fn test_simple_builder_bearer_token() {
        let builder = get("https://example.com").bearer_token("my-token");
        assert_eq!(builder.bearer_token.as_deref(), Some("my-token"));
    }

    #[test]
    fn test_simple_builder_api_key() {
        let builder = get("https://example.com").api_key("X-API-KEY", "key123");
        assert!(builder.api_key.is_some());
    }

    #[test]
    fn test_simple_builder_timeout() {
        let builder = get("https://example.com").timeout(Duration::from_secs(5));
        assert_eq!(builder.timeout, Some(Duration::from_secs(5)));
    }

    #[test]
    fn test_simple_builder_header() {
        let builder = get("https://example.com")
            .header("X-Custom", "value")
            .header("X-Other", "other");
        assert_eq!(builder.headers.len(), 2);
    }
}
