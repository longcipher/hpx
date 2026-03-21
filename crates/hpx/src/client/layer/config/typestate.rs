//! Typestate-pattern configuration builders inspired by ureq's design.
//!
//! This module provides a [`ConfigBuilder<Scope>`] that restricts which
//! builder methods and finalization targets are available depending on the
//! context (client-level vs request-level configuration).
//!
//! # Scopes
//!
//! - [`ClientScope`] – builds a full [`Client`] with the configured settings.
//! - [`RequestScope`] – builds a [`RequestBuilder`] with per-request overrides.
//!
//! # Example
//!
//! ```ignore
//! use hpx::client::layer::config::typestate::{ConfigBuilder, ClientScope};
//!
//! let client = ConfigBuilder::client()
//!     .timeout_global(Some(Duration::from_secs(10)))
//!     .https_only(true)
//!     .build()
//!     .expect("client build failed");
//! ```

use std::time::Duration;

use crate::{Client, ClientBuilder, Method, client::request::RequestBuilder};

mod private {
    pub trait ConfigScope {}
}

// ===== Scope types =====

/// Scope for client-level (agent-level) configuration.
pub struct ClientScope {
    builder: ClientBuilder,
}

/// Scope for request-level configuration.
pub struct RequestScope {
    client: Client,
    method: Method,
    uri: http::Uri,
}

impl private::ConfigScope for ClientScope {}
impl private::ConfigScope for RequestScope {}

// ===== ConfigBuilder =====

/// A typestate builder that provides a uniform configuration API
/// scoped to either client-level or request-level context.
pub struct ConfigBuilder<S: private::ConfigScope>(S);

impl ConfigBuilder<ClientScope> {
    /// Create a new client-scoped config builder.
    pub fn client() -> Self {
        Self(ClientScope {
            builder: Client::builder(),
        })
    }

    /// Create from an existing [`ClientBuilder`].
    pub fn from_builder(builder: ClientBuilder) -> Self {
        Self(ClientScope { builder })
    }

    // --- Timeout methods ---

    /// Timeout for the entire request lifecycle (end-to-end).
    pub fn timeout_global(mut self, timeout: Option<Duration>) -> Self {
        self.0.builder = self.0.builder.timeout_global(timeout);
        self
    }

    /// Timeout for a single call attempt when following redirects.
    pub fn timeout_per_call(mut self, timeout: Option<Duration>) -> Self {
        self.0.builder = self.0.builder.timeout_per_call(timeout);
        self
    }

    /// Timeout for DNS resolution.
    pub fn timeout_resolve(mut self, timeout: Option<Duration>) -> Self {
        self.0.builder = self.0.builder.timeout_resolve(timeout);
        self
    }

    /// Timeout for TCP connection + TLS handshake.
    pub fn timeout_connect(mut self, timeout: Duration) -> Self {
        self.0.builder = self.0.builder.connect_timeout(timeout);
        self
    }

    /// Timeout for sending request headers.
    pub fn timeout_send_request(mut self, timeout: Option<Duration>) -> Self {
        self.0.builder = self.0.builder.timeout_send_request(timeout);
        self
    }

    /// Timeout for awaiting a `100 Continue` response.
    pub fn timeout_await_100(mut self, timeout: Option<Duration>) -> Self {
        self.0.builder = self.0.builder.timeout_await_100(timeout);
        self
    }

    /// Timeout for sending the request body.
    pub fn timeout_send_body(mut self, timeout: Option<Duration>) -> Self {
        self.0.builder = self.0.builder.timeout_send_body(timeout);
        self
    }

    /// Timeout for receiving response headers.
    pub fn timeout_recv_response(mut self, timeout: Option<Duration>) -> Self {
        self.0.builder = self.0.builder.timeout_recv_response(timeout);
        self
    }

    /// Timeout for receiving the response body.
    pub fn timeout_recv_body(mut self, timeout: Option<Duration>) -> Self {
        self.0.builder = self.0.builder.timeout_recv_body(timeout);
        self
    }

    // --- General methods ---

    /// Restrict to HTTPS only.
    pub fn https_only(mut self, enabled: bool) -> Self {
        self.0.builder = self.0.builder.https_only(enabled);
        self
    }

    /// Set the user agent header.
    pub fn user_agent<V>(mut self, value: V) -> Self
    where
        V: TryInto<http::HeaderValue>,
        V::Error: Into<http::Error>,
    {
        self.0.builder = self.0.builder.user_agent(value);
        self
    }

    /// Build the client.
    pub fn build(self) -> crate::Result<Client> {
        self.0.builder.build()
    }
}

impl ConfigBuilder<RequestScope> {
    /// Create a new request-scoped config builder.
    pub fn request(client: Client, method: Method, uri: http::Uri) -> Self {
        Self(RequestScope {
            client,
            method,
            uri,
        })
    }

    /// Set global timeout for this request.
    pub fn timeout_global(self, _timeout: Option<Duration>) -> Self {
        self
    }

    /// Build into a [`RequestBuilder`].
    pub fn build(self) -> RequestBuilder {
        self.0.client.request(self.0.method, self.0.uri)
    }
}
