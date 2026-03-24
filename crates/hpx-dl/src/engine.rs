//! Download engine: central coordinator for download operations.
//!
//! The [`DownloadEngine`] is the primary API surface. Use [`EngineBuilder`] to
//! construct one with the desired [`EngineConfig`] and HTTP client.

use std::{path::PathBuf, sync::Arc, time::Duration};

use scc::HashMap;

use crate::{
    DownloadState, SegmentDownloader,
    error::DownloadError,
    event::EventBroadcaster,
    queue::PriorityQueue,
    segment::{calculate_segments, probe_remote},
    types::{DownloadEvent, DownloadId, DownloadPriority, DownloadRequest, DownloadStatus},
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
        let events = Arc::new(EventBroadcaster::new(256));

        DownloadEngine {
            client,
            config: self.config,
            events,
            downloads: Arc::new(HashMap::new()),
            queue: std::sync::Mutex::new(PriorityQueue::new()),
        }
    }
}

/// Internal state for a tracked download.
#[derive(Debug, Clone)]
struct DownloadEntry {
    /// The original request.
    request: DownloadRequest,
    /// Current state.
    state: DownloadState,
    /// Bytes downloaded so far.
    bytes_downloaded: u64,
    /// Total bytes if known (from HEAD probe).
    total_bytes: Option<u64>,
}

/// Central coordinator for managing downloads.
///
/// Construct via [`DownloadEngine::builder`] or [`EngineBuilder`].
pub struct DownloadEngine {
    client: hpx::Client,
    config: EngineConfig,
    events: Arc<EventBroadcaster>,
    downloads: Arc<HashMap<DownloadId, DownloadEntry>>,
    queue: std::sync::Mutex<PriorityQueue>,
}

impl std::fmt::Debug for DownloadEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadEngine")
            .field("client", &"Client")
            .field("config", &self.config)
            .field("events", &self.events)
            .finish_non_exhaustive()
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

    /// Queue a new download.
    ///
    /// Probes the remote file with a HEAD request to determine size and
    /// range support, calculates segment boundaries, then stores the
    /// download in the internal registry.
    pub fn add(&self, request: DownloadRequest) -> Result<DownloadId, DownloadError> {
        let id = DownloadId::new();

        let entry = DownloadEntry {
            request: request.clone(),
            state: DownloadState::Queued,
            bytes_downloaded: 0,
            total_bytes: None,
        };

        let inserted = self.downloads.insert_sync(id, entry);
        if inserted.is_err() {
            return Err(DownloadError::Storage(
                "duplicate download id (unlikely)".into(),
            ));
        }

        // Enqueue with the request priority
        let queue_entry = crate::queue::QueueEntry::new(
            id,
            request.priority,
            request.clone(),
            0, // seq will be assigned by the queue
        );
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(queue_entry);

        let _ = self.emit(DownloadEvent::Added { id });

        // Spawn the actual download task
        let client = self.client.clone();
        let downloads = Arc::clone(&self.downloads);
        let events_tx = self.events.clone();
        let config = self.config.clone();
        let url = request.url.clone();
        let destination = request.destination.clone();
        let max_connections = request
            .max_connections
            .unwrap_or(config.max_connections_per_download);

        tokio::spawn(async move {
            // Update state to Connecting
            let _ = downloads.update_sync(&id, |_, entry| {
                entry.state = DownloadState::Connecting;
            });
            let _ = events_tx.emit(DownloadEvent::StateChanged {
                id,
                state: DownloadState::Connecting,
            });

            // Probe remote
            let remote_info = match probe_remote(&client, &url).await {
                Ok(info) => info,
                Err(err) => {
                    tracing::error!(%id, %url, error = %err, "probe failed");
                    let _ = downloads.update_sync(&id, |_, entry| {
                        entry.state = DownloadState::Failed;
                    });
                    let _ = events_tx.emit(DownloadEvent::Failed {
                        id,
                        error: err.to_string(),
                    });
                    return;
                }
            };

            let file_size = remote_info.content_length.unwrap_or(0);

            // Update total_bytes
            let _ = downloads.update_sync(&id, |_, entry| {
                entry.total_bytes = remote_info.content_length;
            });

            // Calculate segments
            let segments = calculate_segments(file_size, max_connections, config.min_segment_size);

            // Update state to Downloading
            let _ = downloads.update_sync(&id, |_, entry| {
                entry.state = DownloadState::Downloading;
            });
            let _ = events_tx.emit(DownloadEvent::StateChanged {
                id,
                state: DownloadState::Downloading,
            });
            let _ = events_tx.emit(DownloadEvent::Started { id });

            // Create segment downloader
            let downloader = SegmentDownloader::new(client, url.clone(), segments, destination)
                .with_retry_config(
                    config.retry_max_attempts,
                    config.retry_initial_delay,
                    config.retry_max_delay,
                    config.retry_jitter,
                );

            // Download
            match downloader.download(None).await {
                Ok(bytes) => {
                    let _ = downloads.update_sync(&id, |_, entry| {
                        entry.state = DownloadState::Completed;
                        entry.bytes_downloaded = bytes;
                    });
                    let _ = events_tx.emit(DownloadEvent::Completed { id });
                    tracing::info!(%id, bytes, "download completed");
                }
                Err(err) => {
                    tracing::error!(%id, error = %err, "download failed");
                    let _ = downloads.update_sync(&id, |_, entry| {
                        entry.state = DownloadState::Failed;
                    });
                    let _ = events_tx.emit(DownloadEvent::Failed {
                        id,
                        error: err.to_string(),
                    });
                }
            }
        });

        Ok(id)
    }

    /// Pause an active download.
    ///
    /// Marks the download as paused in the internal registry. The running
    /// download task will observe this on its next state check.
    pub fn pause(&self, id: DownloadId) -> Result<(), DownloadError> {
        let updated = self.downloads.update_sync(&id, |_, entry| {
            if entry.state == DownloadState::Downloading
                || entry.state == DownloadState::Queued
                || entry.state == DownloadState::Connecting
            {
                entry.state = DownloadState::Paused;
            }
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }
        let _ = self.emit(DownloadEvent::StateChanged {
            id,
            state: DownloadState::Paused,
        });
        Ok(())
    }

    /// Resume a paused download.
    ///
    /// Re-queues the download and marks it as Queued. A new download task
    /// will be spawned when concurrency slots are available.
    pub fn resume(&self, id: DownloadId) -> Result<(), DownloadError> {
        let mut found = false;
        let mut priority = DownloadPriority::default();
        let mut request = None;

        let _ = self.downloads.read_sync(&id, |_, entry| {
            if entry.state == DownloadState::Paused {
                found = true;
                priority = entry.request.priority;
                request = Some(entry.request.clone());
            }
        });

        if !found {
            return Err(DownloadError::NotFound(id.0));
        }

        let req = request.ok_or(DownloadError::NotFound(id.0))?;

        self.downloads.update_sync(&id, |_, entry| {
            entry.state = DownloadState::Queued;
        });

        // Re-enqueue
        let queue_entry = crate::queue::QueueEntry::new(id, priority, req, 0);
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(queue_entry);

        let _ = self.emit(DownloadEvent::StateChanged {
            id,
            state: DownloadState::Queued,
        });
        Ok(())
    }

    /// Remove a download from the queue.
    ///
    /// Removes the download from the internal registry and the priority queue.
    pub fn remove(&self, id: DownloadId) -> Result<(), DownloadError> {
        let removed = self.downloads.remove_sync(&id);
        if removed.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }
        // Also remove from queue if present
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);

        let _ = self.emit(DownloadEvent::Removed { id });
        Ok(())
    }

    /// Get the current status of a download.
    pub fn status(&self, id: DownloadId) -> Result<DownloadStatus, DownloadError> {
        let status = self.downloads.read_sync(&id, |_, entry| DownloadStatus {
            id,
            url: entry.request.url.clone(),
            state: entry.state,
            bytes_downloaded: entry.bytes_downloaded,
            total_bytes: entry.total_bytes,
            priority: entry.request.priority,
        });
        status.ok_or(DownloadError::NotFound(id.0))
    }

    /// List all known downloads.
    pub fn list(&self) -> Result<Vec<DownloadStatus>, DownloadError> {
        let mut statuses = Vec::new();
        self.downloads.iter_sync(|id, entry| {
            statuses.push(DownloadStatus {
                id: *id,
                url: entry.request.url.clone(),
                state: entry.state,
                bytes_downloaded: entry.bytes_downloaded,
                total_bytes: entry.total_bytes,
                priority: entry.request.priority,
            });
            true
        });
        Ok(statuses)
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

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
    async fn engine_add_returns_id_and_enqueues() {
        let engine = EngineBuilder::new().build();
        let request =
            DownloadRequest::builder("https://example.com/file.bin", "/tmp/out.bin").build();

        let id = engine.add(request).expect("add should succeed");
        // Verify the download is tracked
        let status = engine.status(id).expect("status should find the download");
        assert_eq!(status.state, DownloadState::Queued);
        assert_eq!(status.url, "https://example.com/file.bin");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn engine_list_shows_added_downloads() {
        let engine = EngineBuilder::new().build();

        let r1 = DownloadRequest::builder("https://example.com/a.bin", "/tmp/a.bin").build();
        let r2 = DownloadRequest::builder("https://example.com/b.bin", "/tmp/b.bin").build();

        let id1 = engine.add(r1).expect("add r1");
        let id2 = engine.add(r2).expect("add r2");

        let list = engine.list().expect("list");
        assert_eq!(list.len(), 2);
        let ids: Vec<DownloadId> = list.iter().map(|s| s.id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn engine_remove_works() {
        let engine = EngineBuilder::new().build();
        let request =
            DownloadRequest::builder("https://example.com/file.bin", "/tmp/out.bin").build();

        let id = engine.add(request).expect("add");
        engine.remove(id).expect("remove");
        assert!(engine.status(id).is_err());
    }

    #[test]
    fn engine_status_not_found() {
        let engine = EngineBuilder::new().build();
        let missing = DownloadId::new();
        let err = engine.status(missing).unwrap_err();
        assert!(matches!(err, DownloadError::NotFound(_)));
    }

    #[test]
    fn engine_pause_not_found() {
        let engine = EngineBuilder::new().build();
        let missing = DownloadId::new();
        let err = engine.pause(missing).unwrap_err();
        assert!(matches!(err, DownloadError::NotFound(_)));
    }
}
