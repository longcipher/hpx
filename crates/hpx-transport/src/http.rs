//! High-performance HTTP client implementation using wreq and tower.

use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use hpx::Client;
use http::{Method as HttpMethod, StatusCode, header::HeaderMap};
use parking_lot::RwLock;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use tower::{Service, ServiceBuilder, ServiceExt};

use crate::{
    auth::{Authentication, NoAuth},
    error::{ErrorFactory, NetworkError, ProtocolError, TransportError, TransportResult},
    transport::{Method, Request, Response, Transport, TransportMetrics},
};

/// Configuration for the HTTP client.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Base URL for all requests
    pub base_url: String,

    /// Default timeout for requests
    pub timeout: Duration,

    /// User agent string
    pub user_agent: String,

    /// Maximum number of connections in the pool
    pub max_connections: usize,

    /// Connection timeout
    pub connect_timeout: Duration,

    /// Pool idle timeout
    pub pool_idle_timeout: Duration,

    /// Maximum number of redirects to follow
    pub max_redirects: usize,

    /// Enable HTTP/2
    pub http2: bool,

    /// Enable TCP keepalive
    pub tcp_keepalive: bool,

    /// TCP keepalive timeout
    pub tcp_keepalive_timeout: Duration,

    /// Enable compression
    pub compression: bool,

    /// Default headers to include with all requests
    pub default_headers: HashMap<String, String>,

    /// Enable request/response logging
    pub logging: bool,

    /// Enable metrics collection
    pub metrics: bool,
}

impl HttpConfig {
    /// Create a new HTTP configuration with defaults.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            timeout: Duration::from_secs(30),
            user_agent: "hpx-transport/0.2.0".to_string(),
            max_connections: 100,
            connect_timeout: Duration::from_secs(10),
            pool_idle_timeout: Duration::from_secs(90),
            max_redirects: 10,
            http2: true,
            tcp_keepalive: true,
            tcp_keepalive_timeout: Duration::from_secs(60),
            compression: true,
            default_headers: HashMap::new(),
            logging: true,
            metrics: true,
        }
    }

    /// Create a builder for the HTTP configuration.
    pub fn builder(base_url: impl Into<String>) -> HttpConfigBuilder {
        HttpConfigBuilder::new(base_url)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> TransportResult<()> {
        if self.base_url.is_empty() {
            return Err(TransportError::config_error("Base URL cannot be empty"));
        }

        if self.timeout.is_zero() {
            return Err(TransportError::config_error("Timeout cannot be zero"));
        }

        if self.max_connections == 0 {
            return Err(TransportError::config_error(
                "Max connections cannot be zero",
            ));
        }

        // Try to parse the base URL
        url::Url::parse(&self.base_url)
            .map_err(|e| TransportError::config_error(format!("Invalid base URL: {e}")))?;

        Ok(())
    }
}

/// Builder for HTTP configuration.
pub struct HttpConfigBuilder {
    config: HttpConfig,
}

impl HttpConfigBuilder {
    /// Create a new builder.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            config: HttpConfig::new(base_url),
        }
    }

    /// Set the timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Set the user agent.
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }

    /// Set the maximum number of connections.
    pub fn max_connections(mut self, max_connections: usize) -> Self {
        self.config.max_connections = max_connections;
        self
    }

    /// Set the connection timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = timeout;
        self
    }

    /// Set the pool idle timeout.
    pub fn pool_idle_timeout(mut self, timeout: Duration) -> Self {
        self.config.pool_idle_timeout = timeout;
        self
    }

    /// Set the maximum number of redirects.
    pub fn max_redirects(mut self, max_redirects: usize) -> Self {
        self.config.max_redirects = max_redirects;
        self
    }

    /// Enable or disable HTTP/2.
    pub fn http2(mut self, enabled: bool) -> Self {
        self.config.http2 = enabled;
        self
    }

    /// Enable or disable TCP keepalive.
    pub fn tcp_keepalive(mut self, enabled: bool) -> Self {
        self.config.tcp_keepalive = enabled;
        self
    }

    /// Set the TCP keepalive timeout.
    pub fn tcp_keepalive_timeout(mut self, timeout: Duration) -> Self {
        self.config.tcp_keepalive_timeout = timeout;
        self
    }

    /// Enable or disable compression.
    pub fn compression(mut self, enabled: bool) -> Self {
        self.config.compression = enabled;
        self
    }

    /// Add a default header.
    pub fn default_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.config
            .default_headers
            .insert(name.into(), value.into());
        self
    }

    /// Add multiple default headers.
    pub fn default_headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (name, value) in headers {
            self.config
                .default_headers
                .insert(name.into(), value.into());
        }
        self
    }

    /// Enable or disable logging.
    pub fn logging(mut self, enabled: bool) -> Self {
        self.config.logging = enabled;
        self
    }

    /// Enable or disable metrics.
    pub fn metrics(mut self, enabled: bool) -> Self {
        self.config.metrics = enabled;
        self
    }

    /// Build the configuration.
    pub fn build(self) -> TransportResult<HttpConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

/// HTTP client implementation.
#[derive(Clone)]
pub struct HttpClient<A = NoAuth> {
    config: HttpConfig,
    _auth: PhantomData<A>,
    metrics: Arc<HttpClientMetrics>,
    // We use a shared service that implements Transport logic
    service: BoxCloneSyncService<Request, Response, TransportError>,
}

impl<A> std::fmt::Debug for HttpClient<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("config", &self.config)
            .finish()
    }
}

impl<A> HttpClient<A>
where
    A: Authentication + 'static,
{
    /// Create a new HTTP client.
    pub fn new(config: HttpConfig, _auth: A) -> TransportResult<Self> {
        config.validate()?;

        // Create hpx client
        let client = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .user_agent(&config.user_agent)
            .build()
            .map_err(|e| TransportError::Config {
                message: format!("Failed to build client: {}", e),
            })?;

        let metrics = Arc::new(HttpClientMetrics::new());

        // Create the base transport service
        let transport = HttpTransport {
            client,
            config: config.clone(),
            metrics: Arc::clone(&metrics),
        };

        // Build middleware stack
        let service = ServiceBuilder::new()
            .layer(tower::buffer::BufferLayer::new(1024))
            .service(transport)
            .map_err(|e| {
                TransportError::Middleware(Box::new(
                    crate::error::MiddlewareError::ExecutionFailed {
                        middleware: "buffer".to_string(),
                        message: e.to_string(),
                    },
                ))
            });

        Ok(Self {
            config,
            _auth: PhantomData,
            metrics,
            service: BoxCloneSyncService::new(service),
        })
    }

    /// Get the client configuration.
    pub fn config(&self) -> &HttpConfig {
        &self.config
    }

    /// Get client metrics.
    pub fn metrics(&self) -> &HttpClientMetrics {
        &self.metrics
    }

    /// Send a GET request.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> TransportResult<T> {
        let request = self.build_request(Method::Get, path, None::<&()>)?;
        let response = self.send_request(request).await?;
        self.parse_response(response).await
    }

    /// Send a GET request with query parameters.
    pub async fn get_with_query<Q: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        query: &Q,
    ) -> TransportResult<T> {
        let mut request = self.build_request(Method::Get, path, None::<&()>)?;
        self.add_query_params(&mut request, query)?;
        let response = self.send_request(request).await?;
        self.parse_response(response).await
    }

    /// Send a POST request.
    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> TransportResult<T> {
        let request = self.build_request(Method::Post, path, Some(body))?;
        let response = self.send_request(request).await?;
        self.parse_response(response).await
    }

    /// Send a PUT request.
    pub async fn put<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> TransportResult<T> {
        let request = self.build_request(Method::Put, path, Some(body))?;
        let response = self.send_request(request).await?;
        self.parse_response(response).await
    }

    /// Send a DELETE request.
    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> TransportResult<T> {
        let request = self.build_request(Method::Delete, path, None::<&()>)?;
        let response = self.send_request(request).await?;
        self.parse_response(response).await
    }

    /// Send a custom request.
    pub async fn send<T: DeserializeOwned>(&self, request: Request) -> TransportResult<T> {
        let response = self.send_request(request).await?;
        self.parse_response(response).await
    }

    /// Send a request and return the raw response.
    pub async fn send_raw(&self, request: Request) -> TransportResult<Response> {
        self.send_request(request).await
    }

    /// Build a request with the given method, path, and optional body.
    pub fn build_request<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> TransportResult<Request> {
        let url = if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!(
                "{}/{}",
                self.config.base_url.trim_end_matches('/'),
                path.trim_start_matches('/')
            )
        };

        let mut request = Request::new(method, url).timeout(self.config.timeout);

        // Add default headers
        for (name, value) in &self.config.default_headers {
            request = request.header(name.clone(), value.clone());
        }

        // Add body if provided
        if let Some(body) = body {
            request = request.json(body)?;
        }

        Ok(request)
    }

    /// Add query parameters to a request.
    pub fn add_query_params<Q: Serialize>(
        &self,
        request: &mut Request,
        query: &Q,
    ) -> TransportResult<()> {
        let query_value = serde_json::to_value(query)?;

        if let serde_json::Value::Object(map) = query_value {
            for (key, value) in map {
                let value_str = match value {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => serde_json::to_string(&value)?,
                };
                request.query.push((key, value_str));
            }
        }

        Ok(())
    }

    /// Send a request through the transport layer.
    async fn send_request(&self, request: Request) -> TransportResult<Response> {
        self.metrics.requests_sent.fetch_add(1, Ordering::Relaxed);
        let start_time = SystemTime::now();

        let result = self.service.clone().oneshot(request).await;

        let duration = start_time.elapsed().unwrap_or(Duration::ZERO);

        match result {
            Ok(mut response) => {
                self.metrics
                    .requests_successful
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics.update_response_time(duration);
                response = response.with_duration(duration);
                Ok(response)
            }
            Err(e) => {
                self.metrics.requests_failed.fetch_add(1, Ordering::Relaxed);
                *self.metrics.last_error_at.write() = Some(SystemTime::now());
                Err(e)
            }
        }
    }

    /// Parse a response into the expected type.
    pub async fn parse_response<T: DeserializeOwned>(
        &self,
        response: Response,
    ) -> TransportResult<T> {
        if !response.is_success() {
            return Err(TransportError::Protocol(Box::new(ProtocolError::Http {
                status: http::StatusCode::from_u16(response.status)
                    .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
                body: response.text().ok(),
            })));
        }

        response.json()
    }
}

/// Internal HTTP transport implementation using wreq.
#[derive(Clone)]
struct HttpTransport {
    client: Client,
    #[allow(dead_code)]
    config: HttpConfig,
    metrics: Arc<HttpClientMetrics>,
}

impl Service<Request> for HttpTransport {
    type Response = Response;
    type Error = TransportError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let this = self.clone();
        Box::pin(async move { this.send(req).await })
    }
}

#[async_trait]
impl Transport for HttpTransport {
    type Error = TransportError;

    async fn send(&self, request: Request) -> Result<Response, Self::Error> {
        // Convert crate::transport::Request to http::Request
        let method: HttpMethod = request.method.clone().into();
        let url = request.url.clone();

        let mut req_builder = http::Request::builder().method(method).uri(url);

        // Add headers
        for (name, value) in &request.headers {
            req_builder = req_builder.header(name, value);
        }

        if !request.query.is_empty() {
            let mut url_obj = url::Url::parse(&request.url)
                .map_err(|e| TransportError::config_error(format!("Invalid URL: {e}")))?;

            {
                let mut pairs = url_obj.query_pairs_mut();
                for (k, v) in &request.query {
                    pairs.append_pair(k, v);
                }
            }

            req_builder = req_builder.uri(url_obj.as_str());
        }

        let body = request.body.unwrap_or_default();

        let http_request = req_builder
            .body(body)
            .map_err(|e| TransportError::config_error(format!("Failed to build request: {e}")))?;

        // Send request using hpx
        let response = self
            .client
            .execute(http_request.into())
            .await
            .map_err(|e| TransportError::Network(Box::new(NetworkError::Hpx { source: e })))?;

        // Convert response
        let status = response.status().as_u16();
        let mut headers = HashMap::new();

        for (name, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                headers.insert(name.to_string(), value_str.to_string());
            }
        }

        let body = response
            .bytes()
            .await
            .map_err(|e| TransportError::Network(Box::new(NetworkError::Hpx { source: e })))?;

        let response = Response {
            request_id: request.id,
            status,
            headers,
            body,
            received_at: SystemTime::now(),
            duration: Duration::ZERO,
            metadata: HashMap::new(),
        };

        Ok(response)
    }

    fn error_from_status(&self, status: u16) -> Self::Error {
        TransportError::Protocol(Box::new(ProtocolError::Http {
            status: http::StatusCode::from_u16(status)
                .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
            body: None,
        }))
    }

    fn metrics(&self) -> TransportMetrics {
        let requests_sent = self.metrics.requests_sent.load(Ordering::Relaxed);
        let requests_successful = self.metrics.requests_successful.load(Ordering::Relaxed);
        let requests_failed = self.metrics.requests_failed.load(Ordering::Relaxed);

        TransportMetrics {
            requests_sent,
            requests_successful,
            requests_failed,
            avg_response_time: self.metrics.avg_response_time(),
            active_connections: 0,
            pool_utilization: 0.0,
            last_error_at: *self.metrics.last_error_at.read(),
        }
    }
}

/// HTTP client metrics.
pub struct HttpClientMetrics {
    pub requests_sent: AtomicU64,
    pub requests_successful: AtomicU64,
    pub requests_failed: AtomicU64,
    response_times: RwLock<VecDeque<Duration>>,
    pub last_error_at: RwLock<Option<SystemTime>>,
}

impl HttpClientMetrics {
    fn new() -> Self {
        Self {
            requests_sent: AtomicU64::new(0),
            requests_successful: AtomicU64::new(0),
            requests_failed: AtomicU64::new(0),
            response_times: RwLock::new(VecDeque::with_capacity(1000)),
            last_error_at: RwLock::new(None),
        }
    }

    fn update_response_time(&self, duration: Duration) {
        if let Some(mut times) = self.response_times.try_write() {
            if times.len() >= 1000 {
                times.pop_front();
            }
            times.push_back(duration);
        }
    }

    fn avg_response_time(&self) -> Duration {
        let times = self.response_times.read();
        if times.is_empty() {
            Duration::ZERO
        } else {
            let total: Duration = times.iter().sum();
            total / times.len() as u32
        }
    }

    /// Get the success rate.
    pub fn success_rate(&self) -> f64 {
        let total = self.requests_sent.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            self.requests_successful.load(Ordering::Relaxed) as f64 / total as f64
        }
    }

    /// Get the error rate.
    pub fn error_rate(&self) -> f64 {
        let total = self.requests_sent.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            self.requests_failed.load(Ordering::Relaxed) as f64 / total as f64
        }
    }
}

// --- Merged from rest.rs ---

#[derive(Debug, Clone)]
pub struct RequestConfig {
    pub timeout: Option<Duration>,
    pub retry_attempts: Option<u8>,
    pub retry_delay: Option<Duration>,
}

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            timeout: None,
            retry_attempts: Some(3),
            retry_delay: Some(Duration::from_millis(500)),
        }
    }
}

// RequestHandler trait using associated type for auth
pub trait RequestHandler<Req: Serialize, Resp: DeserializeOwned> {
    type Auth: Copy + Send + Sync + 'static;
    type BuildError: std::error::Error + Send + Sync + 'static;
    type ResponseError: std::error::Error + Send + Sync + 'static;

    fn build_request(
        &self,
        auth: Self::Auth,
        builder: http::request::Builder,
        request_body: Option<&Req>,
    ) -> Result<http::Request<Vec<u8>>, Self::BuildError>;

    fn handle_response(
        &self,
        status: StatusCode,
        headers: HeaderMap,
        response_body: Bytes,
    ) -> Result<Resp, Self::ResponseError>;
}

#[derive(Error, Debug)]
pub enum RequestError<BuildErr, RespErr> {
    #[error("HTTP request failed: {0}")]
    HpxError(#[source] hpx::Error),

    #[error("Failed to build request: {0}")]
    BuildError(BuildErr),

    #[error("Failed to handle response: {0}")]
    ResponseError(RespErr),

    #[error("Request cancelled")]
    Cancelled,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct RestClient {
    client: Client,
    pub default_timeout: Duration,
    pub host: String,
}

impl Debug for RestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestClient")
            .field("default_timeout", &self.default_timeout)
            .field("host", &self.host)
            .finish()
    }
}

impl RestClient {
    pub fn new(client: Client, timeout: Duration, host: String) -> Self {
        Self {
            client,
            default_timeout: timeout,
            host,
        }
    }

    pub fn builder() -> RestClientBuilder {
        RestClientBuilder::default()
    }
}

#[derive(Default)]
pub struct RestClientBuilder {
    timeout: Option<Duration>,
    host: Option<String>,
}

impl RestClientBuilder {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn host(mut self, host: String) -> Self {
        self.host = Some(host);
        self
    }

    pub fn build(self) -> Result<RestClient, TransportError> {
        let client = Client::default();
        Ok(RestClient {
            client,
            default_timeout: self.timeout.unwrap_or(Duration::from_secs(30)),
            host: self.host.unwrap_or_default(),
        })
    }
}

/// A `Service` that is `Clone`, `Send`, and `Sync`.
pub trait CloneSyncService<R>: Service<R> + Send + Sync {
    fn clone_box(
        &self,
    ) -> Box<
        dyn CloneSyncService<
                R,
                Response = Self::Response,
                Error = Self::Error,
                Future = Self::Future,
            > + Send
            + Sync,
    >;
}

impl<T, R> CloneSyncService<R> for T
where
    T: Service<R> + Clone + Send + Sync + 'static,
{
    fn clone_box(
        &self,
    ) -> Box<
        dyn CloneSyncService<
                R,
                Response = Self::Response,
                Error = Self::Error,
                Future = Self::Future,
            > + Send
            + Sync,
    > {
        Box::new(self.clone())
    }
}

/// A boxed `Service` that is `Clone`, `Send`, and `Sync`.
pub struct BoxCloneSyncService<T, U, E>(
    Box<
        dyn CloneSyncService<T, Response = U, Error = E, Future = BoxFuture<'static, Result<U, E>>>
            + Send
            + Sync,
    >,
);

impl<T, U, E> BoxCloneSyncService<T, U, E> {
    pub fn new<S>(inner: S) -> Self
    where
        S: Service<T, Response = U, Error = E> + Clone + Send + Sync + 'static,
        S::Future: Send + 'static,
    {
        let inner = Box::new(BoxCloneSyncServiceWrapper(inner));
        Self(inner)
    }
}

struct BoxCloneSyncServiceWrapper<S>(S);

impl<S, T, U, E> Service<T> for BoxCloneSyncServiceWrapper<S>
where
    S: Service<T, Response = U, Error = E> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
{
    type Response = U;
    type Error = E;
    type Future = BoxFuture<'static, Result<U, E>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, req: T) -> Self::Future {
        let fut = self.0.call(req);
        Box::pin(fut)
    }
}

impl<S> Clone for BoxCloneSyncServiceWrapper<S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T, U, E> Service<T> for BoxCloneSyncService<T, U, E> {
    type Response = U;
    type Error = E;
    type Future = BoxFuture<'static, Result<U, E>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, req: T) -> Self::Future {
        self.0.call(req)
    }
}

impl<T, U, E> Clone for BoxCloneSyncService<T, U, E> {
    fn clone(&self) -> Self {
        Self(self.0.clone_box())
    }
}

impl<T, U, E> Debug for BoxCloneSyncService<T, U, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxCloneSyncService").finish()
    }
}
