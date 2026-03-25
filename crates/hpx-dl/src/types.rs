//! Core domain types for the download engine.

use std::{collections::HashMap, fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Unique identifier for a download job.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DownloadId(pub uuid::Uuid);

impl DownloadId {
    /// Create a new random download ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for DownloadId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for DownloadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DownloadId").field(&self.0).finish()
    }
}

impl fmt::Display for DownloadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<uuid::Uuid> for DownloadId {
    fn from(value: uuid::Uuid) -> Self {
        Self(value)
    }
}

/// Current state of a download job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    /// Download is queued but not yet started.
    Queued,
    /// Establishing connection to the server.
    Connecting,
    /// Actively downloading data.
    Downloading,
    /// Download is paused by the user.
    Paused,
    /// Download finished successfully.
    Completed,
    /// Download failed with an error.
    Failed,
}

impl fmt::Display for DownloadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Connecting => write!(f, "connecting"),
            Self::Downloading => write!(f, "downloading"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Scheduling priority for a download job.
///
/// Higher values are dequeued first from the priority queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum DownloadPriority {
    /// Low priority.
    Low = 0,
    /// Normal priority (default).
    #[default]
    Normal = 1,
    /// High priority.
    High = 2,
    /// Critical priority, processed before all others.
    Critical = 3,
}

/// Events emitted by the download engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEvent {
    /// A new download was added to the queue.
    Added {
        /// The download identifier.
        id: DownloadId,
    },
    /// A download started.
    Started {
        /// The download identifier.
        id: DownloadId,
    },
    /// Download progress update.
    Progress {
        /// The download identifier.
        id: DownloadId,
        /// Bytes downloaded so far.
        downloaded: u64,
        /// Total bytes if known.
        total: Option<u64>,
    },
    /// The download state changed.
    StateChanged {
        /// The download identifier.
        id: DownloadId,
        /// The new state.
        state: DownloadState,
    },
    /// Download completed successfully.
    Completed {
        /// The download identifier.
        id: DownloadId,
    },
    /// Download failed.
    Failed {
        /// The download identifier.
        id: DownloadId,
        /// Error description.
        error: String,
    },
    /// Download was removed from the queue.
    Removed {
        /// The download identifier.
        id: DownloadId,
    },
}

/// Hash algorithm for checksum verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    /// MD5 hash.
    Md5,
    /// SHA-1 hash.
    Sha1,
    /// SHA-256 hash.
    Sha256,
}

/// Checksum specification for integrity verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecksumSpec {
    /// Hash algorithm to use.
    pub algorithm: HashAlgorithm,
    /// Expected hash value (hex-encoded).
    pub expected: String,
}

/// Proxy protocol kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyKind {
    /// HTTP proxy.
    Http,
    /// HTTPS proxy.
    Https,
    /// SOCKS5 proxy.
    Socks5,
}

/// Proxy configuration for downloads.
///
/// When set on a [`DownloadRequest`], a per-download client will be
/// created with this proxy, delegating to `hpx`'s proxy infrastructure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Proxy URL (e.g., `"http://proxy:8080"`, `"socks5://proxy:1080"`).
    pub url: String,
    /// Proxy protocol kind.
    pub kind: ProxyKind,
}

#[cfg(feature = "http")]
impl TryFrom<ProxyConfig> for hpx::Proxy {
    type Error = hpx::Error;

    fn try_from(config: ProxyConfig) -> Result<Self, Self::Error> {
        match config.kind {
            ProxyKind::Http => Self::http(&config.url),
            ProxyKind::Https => Self::https(&config.url),
            ProxyKind::Socks5 => Self::all(&config.url),
        }
    }
}

/// Specification for a download request.
///
/// Use [`DownloadRequest::builder`] to construct via the builder pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// URL to download from.
    pub url: String,
    /// Local destination path.
    pub destination: PathBuf,
    /// Scheduling priority.
    pub priority: DownloadPriority,
    /// Optional checksum for integrity verification.
    pub checksum: Option<ChecksumSpec>,
    /// Custom HTTP headers.
    pub headers: HashMap<String, String>,
    /// Maximum number of concurrent connections.
    pub max_connections: Option<usize>,
    /// Speed limit in bytes per second.
    pub speed_limit: Option<u64>,
    /// Mirror URLs for fallback.
    pub mirrors: Vec<String>,
    /// Optional proxy configuration for this download.
    /// If set, a per-download client with this proxy will be created.
    pub proxy: Option<ProxyConfig>,
}

impl DownloadRequest {
    /// Create a new builder for a download request.
    #[must_use]
    pub fn builder(
        url: impl Into<String>,
        destination: impl Into<PathBuf>,
    ) -> DownloadRequestBuilder {
        DownloadRequestBuilder {
            url: url.into(),
            destination: destination.into(),
            priority: DownloadPriority::default(),
            checksum: None,
            headers: HashMap::new(),
            max_connections: None,
            speed_limit: None,
            mirrors: Vec::new(),
            proxy: None,
        }
    }
}

/// Builder for [`DownloadRequest`].
#[derive(Debug)]
pub struct DownloadRequestBuilder {
    url: String,
    destination: PathBuf,
    priority: DownloadPriority,
    checksum: Option<ChecksumSpec>,
    headers: HashMap<String, String>,
    max_connections: Option<usize>,
    speed_limit: Option<u64>,
    mirrors: Vec<String>,
    proxy: Option<ProxyConfig>,
}

impl DownloadRequestBuilder {
    /// Set the download priority.
    #[must_use]
    pub const fn priority(mut self, priority: DownloadPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set the checksum specification.
    #[must_use]
    pub fn checksum(mut self, checksum: ChecksumSpec) -> Self {
        self.checksum = Some(checksum);
        self
    }

    /// Add a custom HTTP header.
    #[must_use]
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Set the maximum number of concurrent connections.
    #[must_use]
    pub const fn max_connections(mut self, max: usize) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Set the speed limit in bytes per second.
    #[must_use]
    pub const fn speed_limit(mut self, limit: u64) -> Self {
        self.speed_limit = Some(limit);
        self
    }

    /// Add a mirror URL for fallback.
    #[must_use]
    pub fn mirror(mut self, url: impl Into<String>) -> Self {
        self.mirrors.push(url.into());
        self
    }

    /// Set multiple mirror URLs at once.
    #[must_use]
    pub fn mirrors(mut self, urls: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.mirrors.extend(urls.into_iter().map(Into::into));
        self
    }

    /// Set the proxy configuration for this download.
    #[must_use]
    pub fn proxy(mut self, proxy: ProxyConfig) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Build the [`DownloadRequest`].
    #[must_use]
    pub fn build(self) -> DownloadRequest {
        DownloadRequest {
            url: self.url,
            destination: self.destination,
            priority: self.priority,
            checksum: self.checksum,
            headers: self.headers,
            max_connections: self.max_connections,
            speed_limit: self.speed_limit,
            mirrors: self.mirrors,
            proxy: self.proxy,
        }
    }
}

/// Snapshot of a download's current status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadStatus {
    /// The download identifier.
    pub id: DownloadId,
    /// Source URL.
    pub url: String,
    /// Current state.
    pub state: DownloadState,
    /// Bytes downloaded so far.
    pub bytes_downloaded: u64,
    /// Total bytes if known.
    pub total_bytes: Option<u64>,
    /// Scheduling priority.
    pub priority: DownloadPriority,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_id_creation() {
        let id = DownloadId::new();
        let _display = format!("{id}");
        let from_uuid = DownloadId::from(uuid::Uuid::new_v4());
        assert_ne!(id, from_uuid);
    }

    #[test]
    fn download_id_serialization_roundtrip() {
        let id = DownloadId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        let back: DownloadId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, back);
    }

    #[test]
    fn download_state_serialization_roundtrip() {
        for state in [
            DownloadState::Queued,
            DownloadState::Connecting,
            DownloadState::Downloading,
            DownloadState::Paused,
            DownloadState::Completed,
            DownloadState::Failed,
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            let back: DownloadState = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(state, back);
        }
    }

    #[test]
    fn download_priority_ordering() {
        assert!(DownloadPriority::Critical > DownloadPriority::High);
        assert!(DownloadPriority::High > DownloadPriority::Normal);
        assert!(DownloadPriority::Normal > DownloadPriority::Low);
    }

    #[test]
    fn download_priority_default() {
        assert_eq!(DownloadPriority::default(), DownloadPriority::Normal);
    }

    #[test]
    fn download_event_construction() {
        let id = DownloadId::new();
        let _added = DownloadEvent::Added { id };
        let _started = DownloadEvent::Started { id };
        let _progress = DownloadEvent::Progress {
            id,
            downloaded: 1024,
            total: Some(4096),
        };
        let _state_changed = DownloadEvent::StateChanged {
            id,
            state: DownloadState::Downloading,
        };
        let _completed = DownloadEvent::Completed { id };
        let _failed = DownloadEvent::Failed {
            id,
            error: "connection reset".to_string(),
        };
        let _removed = DownloadEvent::Removed { id };
    }

    #[test]
    fn download_request_builder() {
        let request = DownloadRequest::builder("https://example.com/file.bin", "/tmp/file.bin")
            .priority(DownloadPriority::High)
            .checksum(ChecksumSpec {
                algorithm: HashAlgorithm::Sha256,
                expected: "abc123".to_string(),
            })
            .header("Authorization", "Bearer token")
            .max_connections(4)
            .speed_limit(1_000_000)
            .mirror("https://mirror.example.com/file.bin")
            .build();

        assert_eq!(request.url, "https://example.com/file.bin");
        assert_eq!(request.destination, PathBuf::from("/tmp/file.bin"));
        assert_eq!(request.priority, DownloadPriority::High);
        assert!(request.checksum.is_some());
        assert_eq!(request.headers.len(), 1);
        assert_eq!(request.max_connections, Some(4));
        assert_eq!(request.speed_limit, Some(1_000_000));
        assert_eq!(request.mirrors.len(), 1);
    }

    #[test]
    fn download_request_builder_defaults() {
        let request = DownloadRequest::builder("https://example.com", "/tmp/out").build();

        assert_eq!(request.priority, DownloadPriority::Normal);
        assert!(request.checksum.is_none());
        assert!(request.headers.is_empty());
        assert!(request.max_connections.is_none());
        assert!(request.speed_limit.is_none());
        assert!(request.mirrors.is_empty());
        assert!(request.proxy.is_none());
    }

    #[test]
    fn checksum_spec_construction() {
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Sha256,
            expected: "deadbeef".to_string(),
        };
        assert_eq!(spec.algorithm, HashAlgorithm::Sha256);
        assert_eq!(spec.expected, "deadbeef");
    }

    #[test]
    fn download_status_construction() {
        let id = DownloadId::new();
        let status = DownloadStatus {
            id,
            url: "https://example.com/file".to_string(),
            state: DownloadState::Downloading,
            bytes_downloaded: 500,
            total_bytes: Some(1000),
            priority: DownloadPriority::Normal,
        };
        assert_eq!(status.id, id);
        assert_eq!(status.state, DownloadState::Downloading);
        assert_eq!(status.bytes_downloaded, 500);
    }

    #[test]
    fn download_status_serialization() {
        let status = DownloadStatus {
            id: DownloadId::new(),
            url: "https://example.com/file".to_string(),
            state: DownloadState::Completed,
            bytes_downloaded: 1000,
            total_bytes: Some(1000),
            priority: DownloadPriority::Low,
        };
        let json = serde_json::to_string(&status).expect("serialize");
        let back: DownloadStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(status.id, back.id);
        assert_eq!(status.state, back.state);
        assert_eq!(status.bytes_downloaded, back.bytes_downloaded);
    }

    // --- ProxyConfig and ProxyKind tests ---

    #[test]
    fn proxy_kind_equality() {
        assert_eq!(ProxyKind::Http, ProxyKind::Http);
        assert_eq!(ProxyKind::Https, ProxyKind::Https);
        assert_eq!(ProxyKind::Socks5, ProxyKind::Socks5);
        assert_ne!(ProxyKind::Http, ProxyKind::Https);
        assert_ne!(ProxyKind::Http, ProxyKind::Socks5);
        assert_ne!(ProxyKind::Https, ProxyKind::Socks5);
    }

    #[test]
    fn proxy_kind_serialization_roundtrip() {
        for kind in [ProxyKind::Http, ProxyKind::Https, ProxyKind::Socks5] {
            let json = serde_json::to_string(&kind).expect("serialize");
            let back: ProxyKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn proxy_config_construction() {
        let config = ProxyConfig {
            url: "http://proxy:8080".to_string(),
            kind: ProxyKind::Http,
        };
        assert_eq!(config.url, "http://proxy:8080");
        assert_eq!(config.kind, ProxyKind::Http);
    }

    #[test]
    fn proxy_config_serialization_roundtrip() {
        let config = ProxyConfig {
            url: "socks5://proxy:1080".to_string(),
            kind: ProxyKind::Socks5,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let back: ProxyConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(config.url, back.url);
        assert_eq!(config.kind, back.kind);
    }

    #[test]
    fn proxy_config_clone() {
        let config = ProxyConfig {
            url: "https://proxy:443".to_string(),
            kind: ProxyKind::Https,
        };
        let cloned = config.clone();
        assert_eq!(config.url, cloned.url);
        assert_eq!(config.kind, cloned.kind);
    }

    #[test]
    fn download_request_with_proxy() {
        let proxy = ProxyConfig {
            url: "http://proxy:8080".to_string(),
            kind: ProxyKind::Http,
        };
        let request = DownloadRequest::builder("https://example.com/file", "/tmp/file")
            .proxy(proxy.clone())
            .build();

        assert!(request.proxy.is_some());
        let req_proxy = request.proxy.expect("proxy present");
        assert_eq!(req_proxy.url, "http://proxy:8080");
        assert_eq!(req_proxy.kind, ProxyKind::Http);
    }

    #[test]
    fn download_request_without_proxy_defaults_to_none() {
        let request = DownloadRequest::builder("https://example.com", "/tmp/out").build();
        assert!(request.proxy.is_none());
    }

    #[test]
    fn download_request_builder_proxy_chaining() {
        let request = DownloadRequest::builder("https://example.com/file", "/tmp/file")
            .priority(DownloadPriority::High)
            .proxy(ProxyConfig {
                url: "socks5://proxy:1080".to_string(),
                kind: ProxyKind::Socks5,
            })
            .max_connections(4)
            .build();

        assert_eq!(request.priority, DownloadPriority::High);
        assert_eq!(request.max_connections, Some(4));
        assert!(request.proxy.is_some());
        let p = request.proxy.expect("proxy present");
        assert_eq!(p.kind, ProxyKind::Socks5);
    }

    #[test]
    fn download_request_serialization_with_proxy() {
        let request = DownloadRequest::builder("https://example.com/file", "/tmp/file")
            .proxy(ProxyConfig {
                url: "http://proxy:8080".to_string(),
                kind: ProxyKind::Http,
            })
            .build();

        let json = serde_json::to_string(&request).expect("serialize");
        let back: DownloadRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(request.url, back.url);
        assert!(back.proxy.is_some());
        let back_proxy = back.proxy.expect("proxy present");
        assert_eq!(back_proxy.url, "http://proxy:8080");
        assert_eq!(back_proxy.kind, ProxyKind::Http);
    }

    #[test]
    fn proxy_config_to_hpx_proxy_http() {
        let config = ProxyConfig {
            url: "http://proxy:8080".to_string(),
            kind: ProxyKind::Http,
        };
        let proxy = hpx::Proxy::try_from(config).expect("valid proxy config");
        // Verify it can be used to build a client (proxy is valid)
        let _client = hpx::Client::builder().proxy(proxy).build().expect("client");
    }

    #[test]
    fn proxy_config_to_hpx_proxy_https() {
        let config = ProxyConfig {
            url: "http://proxy:8080".to_string(),
            kind: ProxyKind::Https,
        };
        let proxy = hpx::Proxy::try_from(config).expect("valid proxy config");
        let _client = hpx::Client::builder().proxy(proxy).build().expect("client");
    }

    #[test]
    fn proxy_config_to_hpx_proxy_socks5() {
        let config = ProxyConfig {
            url: "socks5://proxy:1080".to_string(),
            kind: ProxyKind::Socks5,
        };
        let proxy = hpx::Proxy::try_from(config).expect("valid proxy config");
        let _client = hpx::Client::builder().proxy(proxy).build().expect("client");
    }
}
