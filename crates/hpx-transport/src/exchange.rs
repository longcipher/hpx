//! Exchange client abstraction for REST APIs.
//!
//! This module provides a unified interface for interacting with
//! cryptocurrency exchange REST APIs.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use hpx::Client;
use serde::{Serialize, de::DeserializeOwned};
use url::{Url, form_urlencoded};

use crate::{
    auth::Authentication,
    error::{TransportError, TransportResult},
    typed::TypedResponse,
};

/// Configuration for the REST client.
#[derive(Debug, Clone)]
pub struct RestConfig {
    /// Base URL for all requests
    pub base_url: String,
    /// Request timeout
    pub timeout: Duration,
    /// User agent string
    pub user_agent: String,
    /// Optional rotating proxy pool shared by all REST requests.
    pub proxy_pool: Option<hpx::proxy_pool::ProxyPool>,
}

impl RestConfig {
    /// Create a new REST configuration.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            timeout: Duration::from_secs(30),
            user_agent: format!("hpx-transport/{}", env!("CARGO_PKG_VERSION")),
            proxy_pool: None,
        }
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the user agent.
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Set a rotating proxy pool for this REST client.
    pub fn proxy_pool(mut self, proxy_pool: hpx::proxy_pool::ProxyPool) -> Self {
        self.proxy_pool = Some(proxy_pool);
        self
    }
}

/// Trait for exchange clients.
#[async_trait]
pub trait ExchangeClient: Send + Sync {
    /// The authentication type used by this client.
    type Auth: Authentication;

    /// Get the underlying HTTP client.
    fn http(&self) -> &Client;

    /// Get the authentication provider.
    fn auth(&self) -> &Self::Auth;

    /// Get the base URL.
    fn base_url(&self) -> &str;

    /// Send a GET request.
    async fn get<T: DeserializeOwned + Send>(
        &self,
        path: &str,
    ) -> TransportResult<TypedResponse<T>>;

    /// Send a GET request with query parameters.
    async fn get_with_query<Q: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        query: &Q,
    ) -> TransportResult<TypedResponse<T>>;

    /// Send a POST request.
    async fn post<B: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        body: &B,
    ) -> TransportResult<TypedResponse<T>>;

    /// Send a PUT request.
    async fn put<B: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        body: &B,
    ) -> TransportResult<TypedResponse<T>>;

    /// Send a DELETE request.
    async fn delete<T: DeserializeOwned + Send>(
        &self,
        path: &str,
    ) -> TransportResult<TypedResponse<T>>;
}

/// A generic REST client implementation.
pub struct RestClient<A: Authentication> {
    client: Client,
    auth: A,
    config: RestConfig,
}

impl<A: Authentication> std::fmt::Debug for RestClient<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestClient")
            .field("base_url", &self.config.base_url)
            .field("timeout", &self.config.timeout)
            .finish()
    }
}

impl<A: Authentication> RestClient<A> {
    /// Create a new REST client.
    pub fn new(config: RestConfig, auth: A) -> TransportResult<Self> {
        let mut builder = Client::builder()
            .timeout(config.timeout)
            .user_agent(&config.user_agent);

        if let Some(proxy_pool) = config.proxy_pool.clone() {
            builder = builder.proxy_pool(proxy_pool);
        }

        let client = builder
            .build()
            .map_err(|e| TransportError::config(e.to_string()))?;

        Ok(Self {
            client,
            auth,
            config,
        })
    }

    /// Get the configuration.
    pub fn config(&self) -> &RestConfig {
        &self.config
    }

    fn build_url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!(
                "{}/{}",
                self.config.base_url.trim_end_matches('/'),
                path.trim_start_matches('/')
            )
        }
    }

    fn append_query(url: &mut String, query: &str) -> TransportResult<()> {
        if query.is_empty() {
            return Ok(());
        }

        let mut parsed =
            Url::parse(url).map_err(|e| TransportError::config(format!("Invalid URL: {e}")))?;
        {
            let mut qp = parsed.query_pairs_mut();
            for (k, v) in form_urlencoded::parse(query.as_bytes()) {
                qp.append_pair(&k, &v);
            }
        }
        *url = parsed.into();
        Ok(())
    }

    fn path_and_query(url: &str) -> TransportResult<String> {
        let parsed =
            Url::parse(url).map_err(|e| TransportError::config(format!("Invalid URL: {e}")))?;
        let mut path_and_query = parsed.path().to_string();
        if let Some(query) = parsed.query() {
            path_and_query.push('?');
            path_and_query.push_str(query);
        }
        Ok(path_and_query)
    }

    async fn send_request<T: DeserializeOwned>(
        &self,
        method: http::Method,
        path: &str,
        body: Option<&[u8]>,
        query: Option<&str>,
    ) -> TransportResult<TypedResponse<T>> {
        let start = Instant::now();
        let mut url = self.build_url(path);

        // Add query parameters first so auth signing can see the final request context.
        if let Some(q) = query {
            Self::append_query(&mut url, q)?;
        }

        // Sign request
        let mut headers = http::header::HeaderMap::new();
        let signing_path = Self::path_and_query(&url)?;
        if let Some(auth_query) = self
            .auth
            .sign(&method, &signing_path, &mut headers, body)
            .await?
        {
            Self::append_query(&mut url, &auth_query)?;
        }

        // Build request
        let mut req = match method {
            http::Method::GET => self.client.get(&url),
            http::Method::POST => self.client.post(&url),
            http::Method::PUT => self.client.put(&url),
            http::Method::DELETE => self.client.delete(&url),
            http::Method::PATCH => self.client.patch(&url),
            _ => {
                return Err(TransportError::config(format!(
                    "Unsupported method: {}",
                    method
                )));
            }
        };

        // Add headers
        for (name, value) in headers.iter() {
            req = req.header(name.clone(), value.clone());
        }

        // Add body
        if let Some(b) = body {
            req = req
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(b.to_vec());
        }

        // Send request
        let resp = req.send().await?;
        let status = resp.status();
        let latency = start.elapsed();

        // Check status
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(TransportError::api(status, body_text));
        }

        // Parse response
        let bytes = resp.bytes().await?;
        let data: T = if bytes.is_empty() {
            serde_json::from_slice(b"null").or_else(|_| serde_json::from_slice(&bytes))?
        } else {
            serde_json::from_slice(&bytes)?
        };

        Ok(TypedResponse::new(data, status, latency).with_raw_body(bytes))
    }
}

#[async_trait]
impl<A: Authentication + 'static> ExchangeClient for RestClient<A> {
    type Auth = A;

    fn http(&self) -> &Client {
        &self.client
    }

    fn auth(&self) -> &Self::Auth {
        &self.auth
    }

    fn base_url(&self) -> &str {
        &self.config.base_url
    }

    async fn get<T: DeserializeOwned + Send>(
        &self,
        path: &str,
    ) -> TransportResult<TypedResponse<T>> {
        self.send_request(http::Method::GET, path, None, None).await
    }

    async fn get_with_query<Q: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        query: &Q,
    ) -> TransportResult<TypedResponse<T>> {
        let query_str = serde_urlencoded::to_string(query)
            .map_err(|e| TransportError::config(e.to_string()))?;
        self.send_request(http::Method::GET, path, None, Some(&query_str))
            .await
    }

    async fn post<B: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        body: &B,
    ) -> TransportResult<TypedResponse<T>> {
        let body_bytes = serde_json::to_vec(body)?;
        self.send_request(http::Method::POST, path, Some(&body_bytes), None)
            .await
    }

    async fn put<B: Serialize + Send + Sync, T: DeserializeOwned + Send>(
        &self,
        path: &str,
        body: &B,
    ) -> TransportResult<TypedResponse<T>> {
        let body_bytes = serde_json::to_vec(body)?;
        self.send_request(http::Method::PUT, path, Some(&body_bytes), None)
            .await
    }

    async fn delete<T: DeserializeOwned + Send>(
        &self,
        path: &str,
    ) -> TransportResult<TypedResponse<T>> {
        self.send_request(http::Method::DELETE, path, None, None)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::auth::{Authentication, NoAuth};

    #[test]
    fn test_rest_config() {
        let config = RestConfig::new("https://api.example.com")
            .timeout(Duration::from_secs(60))
            .user_agent("test-agent");

        assert_eq!(config.base_url, "https://api.example.com");
        assert_eq!(config.timeout, Duration::from_secs(60));
        assert_eq!(config.user_agent, "test-agent");
        assert!(config.proxy_pool.is_none());
    }

    #[test]
    fn test_rest_config_proxy_pool() {
        let pool = hpx::proxy_pool::ProxyPool::with_strategy(
            vec![hpx::Proxy::all("http://proxy.local:8080").expect("proxy URI should parse")],
            hpx::proxy_pool::ProxyPoolStrategy::StickyFailover,
        )
        .expect("proxy pool should build");

        let config = RestConfig::new("https://api.example.com").proxy_pool(pool.clone());
        assert!(config.proxy_pool.is_some());
    }

    #[test]
    fn test_build_url() {
        let config = RestConfig::new("https://api.example.com");
        let client = RestClient::new(config, NoAuth).unwrap();

        assert_eq!(
            client.build_url("/v1/orders"),
            "https://api.example.com/v1/orders"
        );
        assert_eq!(
            client.build_url("v1/orders"),
            "https://api.example.com/v1/orders"
        );
        assert_eq!(
            client.build_url("https://other.com/path"),
            "https://other.com/path"
        );
    }

    #[test]
    fn test_path_and_query() {
        let path = RestClient::<NoAuth>::path_and_query(
            "https://api.example.com/v1/orders?symbol=BTCUSDT&limit=5",
        )
        .unwrap();
        assert_eq!(path, "/v1/orders?symbol=BTCUSDT&limit=5");
    }

    #[derive(Clone, Default)]
    struct RecordingAuth {
        path: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl Authentication for RecordingAuth {
        async fn sign(
            &self,
            _method: &http::Method,
            path: &str,
            _headers: &mut http::HeaderMap,
            _body: Option<&[u8]>,
        ) -> TransportResult<Option<String>> {
            *self.path.lock().expect("lock poisoned") = Some(path.to_string());
            Ok(None)
        }
    }

    #[tokio::test]
    async fn test_sign_receives_path_with_business_query() {
        use std::convert::Infallible;

        use http_body_util::Full;
        use hyper::{Request, Response, body::Bytes, server::conn::http1, service::service_fn};
        use hyper_util::rt::TokioIo;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(|_req: Request<hyper::body::Incoming>| async move {
                        Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(b"{}"))))
                    });
                    let _ = http1::Builder::new().serve_connection(io, svc).await;
                });
            }
        });

        let auth = RecordingAuth::default();
        let config = RestConfig::new(format!("http://{addr}"));
        let client = RestClient::new(config, auth.clone()).unwrap();

        #[derive(Serialize)]
        struct Query {
            symbol: &'static str,
            limit: u32,
        }

        let _resp: TypedResponse<serde_json::Value> = client
            .get_with_query(
                "/v1/orders",
                &Query {
                    symbol: "BTCUSDT",
                    limit: 5,
                },
            )
            .await
            .unwrap();

        let signed_path = auth.path.lock().expect("lock poisoned").clone().unwrap();
        assert!(signed_path.starts_with("/v1/orders?"));
        assert!(signed_path.contains("symbol=BTCUSDT"));
        assert!(signed_path.contains("limit=5"));
    }
}
