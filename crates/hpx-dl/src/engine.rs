//! Download engine: central coordinator for download operations.
//!
//! The [`DownloadEngine`] is the primary API surface. Use [`EngineBuilder`] to
//! construct one with the desired [`EngineConfig`] and HTTP client.

use std::{path::PathBuf, time::Duration};

use crate::{
    error::DownloadError,
    event::EventBroadcaster,
    types::{DownloadEvent, DownloadId, DownloadRequest, DownloadStatus},
};

/// Configuration for the download engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Maximum number of concurrent downloads.
    pub max_concurrent_downloads: usize,
    /// Maximum connections per single download.
    pub max_connections_per_download: usize,
    /// Minimum segment size in bytes for splitting downloads.
    pub min_segment_size: u64,
    /// Global speed limit in bytes per second. `None` means unlimited.
    pub global_speed_limit: Option<u64>,
    /// Maximum retry attempts per download.
    pub retry_max_attempts: u32,
    /// Initial delay before first retry.
    pub retry_initial_delay: Duration,
    /// Maximum delay between retries.
    pub retry_max_delay: Duration,
    /// Jitter factor for retry backoff (0.0–1.0).
    pub retry_jitter: f64,
    /// Path to the storage database file.
    pub storage_path: PathBuf,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 5,
            max_connections_per_download: 5,
            min_segment_size: 1024 * 1024, // 1 MiB
            global_speed_limit: None,
            retry_max_attempts: 3,
            retry_initial_delay: Duration::from_secs(1),
            retry_max_delay: Duration::from_secs(30),
            retry_jitter: 0.25,
            storage_path: PathBuf::from("./hpx-dl.db"),
        }
    }
}

/// Fluent builder for [`DownloadEngine`].
///
/// Use [`DownloadEngine::builder`] or `EngineBuilder::default` to start.
#[derive(Default)]
pub struct EngineBuilder {
    client: Option<hpx::Client>,
    config: EngineConfig,
}

impl std::fmt::Debug for EngineBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineBuilder")
            .field("client", &self.client.as_ref().map(|_| "Client"))
            .field("config", &self.config)
            .finish()
    }
}

impl EngineBuilder {
    /// Create a new builder with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the HTTP client.
    #[must_use]
    pub fn client(mut self, client: hpx::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Replace the full engine configuration.
    #[must_use]
    pub fn config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the maximum number of concurrent downloads.
    #[must_use]
    pub const fn max_concurrent(mut self, n: usize) -> Self {
        self.config.max_concurrent_downloads = n;
        self
    }

    /// Set the maximum connections per download.
    #[must_use]
    pub const fn max_connections(mut self, n: usize) -> Self {
        self.config.max_connections_per_download = n;
        self
    }

    /// Set the global speed limit in bytes per second. `None` for unlimited.
    #[must_use]
    pub const fn speed_limit(mut self, limit: Option<u64>) -> Self {
        self.config.global_speed_limit = limit;
        self
    }

    /// Set the storage database path.
    #[must_use]
    pub fn storage_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.storage_path = path.into();
        self
    }

    /// Build the [`DownloadEngine`].
    ///
    /// If no client was provided, a default `hpx::Client` is constructed.
    #[must_use]
    pub fn build(self) -> DownloadEngine {
        let client = self.client.unwrap_or_default();
        let events = EventBroadcaster::new(256);

        DownloadEngine {
            client,
            config: self.config,
            events,
        }
    }
}

/// Central coordinator for managing downloads.
///
/// Construct via [`DownloadEngine::builder`] or [`EngineBuilder`].
pub struct DownloadEngine {
    client: hpx::Client,
    config: EngineConfig,
    events: EventBroadcaster,
}

impl std::fmt::Debug for DownloadEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadEngine")
            .field("client", &"Client")
            .field("config", &self.config)
            .field("events", &self.events)
            .finish()
    }
}

impl DownloadEngine {
    /// Create a builder for the engine.
    #[must_use]
    pub fn builder() -> EngineBuilder {
        EngineBuilder::new()
    }

    /// Subscribe to download events.
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<DownloadEvent> {
        self.events.subscribe()
    }

    /// Emit a download event to all subscribers.
    ///
    /// If there are no receivers, a warning is logged but no error is returned.
    pub(crate) fn emit(&self, event: DownloadEvent) -> Result<usize, crate::event::EventError> {
        self.events.emit(event)
    }

    /// Return a reference to the engine configuration.
    #[must_use]
    pub const fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Return a reference to the HTTP client.
    #[must_use]
    pub const fn client(&self) -> &hpx::Client {
        &self.client
    }

    /// Queue a new download. *(stub — not yet implemented)*
    pub fn add(&self, _request: DownloadRequest) -> Result<DownloadId, DownloadError> {
        let id = DownloadId::new();
        let _ = self.emit(DownloadEvent::Added { id });
        Err(DownloadError::Storage("not implemented".into()))
    }

    /// Pause an active download. *(stub — not yet implemented)*
    pub fn pause(&self, id: DownloadId) -> Result<(), DownloadError> {
        let _ = self.emit(DownloadEvent::StateChanged {
            id,
            state: crate::types::DownloadState::Paused,
        });
        Err(DownloadError::Storage("not implemented".into()))
    }

    /// Resume a paused download. *(stub — not yet implemented)*
    pub fn resume(&self, id: DownloadId) -> Result<(), DownloadError> {
        let _ = self.emit(DownloadEvent::StateChanged {
            id,
            state: crate::types::DownloadState::Downloading,
        });
        Err(DownloadError::Storage("not implemented".into()))
    }

    /// Remove a download from the queue. *(stub — not yet implemented)*
    pub fn remove(&self, id: DownloadId) -> Result<(), DownloadError> {
        let _ = self.emit(DownloadEvent::Removed { id });
        Err(DownloadError::Storage("not implemented".into()))
    }

    /// Get the current status of a download. *(stub — not yet implemented)*
    pub fn status(&self, _id: DownloadId) -> Result<DownloadStatus, DownloadError> {
        Err(DownloadError::Storage("not implemented".into()))
    }

    /// List all known downloads. *(stub — not yet implemented)*
    pub fn list(&self) -> Result<Vec<DownloadStatus>, DownloadError> {
        Err(DownloadError::Storage("not implemented".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_config_defaults() {
        let config = EngineConfig::default();
        assert_eq!(config.max_concurrent_downloads, 5);
        assert_eq!(config.max_connections_per_download, 5);
        assert_eq!(config.min_segment_size, 1024 * 1024);
        assert!(config.global_speed_limit.is_none());
        assert_eq!(config.retry_max_attempts, 3);
        assert_eq!(config.retry_initial_delay, Duration::from_secs(1));
        assert_eq!(config.retry_max_delay, Duration::from_secs(30));
        assert!((config.retry_jitter - 0.25).abs() < f64::EPSILON);
        assert_eq!(config.storage_path, PathBuf::from("./hpx-dl.db"));
    }

    #[test]
    fn engine_config_custom() {
        let config = EngineConfig {
            max_concurrent_downloads: 10,
            max_connections_per_download: 8,
            min_segment_size: 2 * 1024 * 1024,
            global_speed_limit: Some(1_000_000),
            retry_max_attempts: 5,
            retry_initial_delay: Duration::from_millis(500),
            retry_max_delay: Duration::from_secs(60),
            retry_jitter: 0.5,
            storage_path: PathBuf::from("/tmp/custom.db"),
        };
        assert_eq!(config.max_concurrent_downloads, 10);
        assert_eq!(config.global_speed_limit, Some(1_000_000));
        assert_eq!(config.storage_path, PathBuf::from("/tmp/custom.db"));
    }

    #[test]
    fn builder_defaults_produce_valid_engine() {
        let engine = EngineBuilder::new().build();
        assert_eq!(engine.config().max_concurrent_downloads, 5);
        assert_eq!(engine.config().max_connections_per_download, 5);
    }

    #[test]
    fn builder_shortcuts_override_defaults() {
        let engine = EngineBuilder::new()
            .max_concurrent(16)
            .max_connections(4)
            .speed_limit(Some(500_000))
            .storage_path("/tmp/test.db")
            .build();

        assert_eq!(engine.config().max_concurrent_downloads, 16);
        assert_eq!(engine.config().max_connections_per_download, 4);
        assert_eq!(engine.config().global_speed_limit, Some(500_000));
        assert_eq!(engine.config().storage_path, PathBuf::from("/tmp/test.db"));
        // Defaults preserved
        assert_eq!(engine.config().retry_max_attempts, 3);
    }

    #[test]
    fn builder_with_full_config() {
        let config = EngineConfig {
            max_concurrent_downloads: 20,
            max_connections_per_download: 10,
            min_segment_size: 4 * 1024 * 1024,
            global_speed_limit: Some(2_000_000),
            retry_max_attempts: 7,
            retry_initial_delay: Duration::from_millis(200),
            retry_max_delay: Duration::from_secs(120),
            retry_jitter: 0.1,
            storage_path: PathBuf::from("/data/dl.db"),
        };
        let engine = EngineBuilder::new().config(config.clone()).build();
        assert_eq!(engine.config().max_concurrent_downloads, 20);
        assert_eq!(engine.config().min_segment_size, 4 * 1024 * 1024);
        assert_eq!(engine.config().retry_max_attempts, 7);
    }

    #[test]
    fn engine_subscribe_returns_receiver() {
        let engine = EngineBuilder::new().build();
        let _rx = engine.subscribe();
    }

    #[tokio::test]
    async fn engine_emit_and_subscribe() {
        let engine = EngineBuilder::new().build();
        let mut rx = engine.subscribe();

        let id = DownloadId::new();
        let event = DownloadEvent::Completed { id };
        engine.emit(event.clone()).unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, event);
    }

    #[test]
    fn engine_emit_no_receivers_ok() {
        let engine = EngineBuilder::new().build();
        let id = DownloadId::new();
        let result = engine.emit(DownloadEvent::Added { id });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn engine_stubs_return_not_implemented() {
        let engine = EngineBuilder::new().build();
        let id = DownloadId::new();

        assert!(
            engine
                .add(DownloadRequest::builder("https://example.com", "/tmp/out").build())
                .is_err()
        );
        assert!(engine.pause(id).is_err());
        assert!(engine.resume(id).is_err());
        assert!(engine.status(id).is_err());
        assert!(engine.list().is_err());
    }
}
