//! Download engine: central coordinator for download operations.
//!
//! The [`DownloadEngine`] owns download lifecycle, persistence, scheduling,
//! and event fan-out. Use [`EngineBuilder`] to construct one with the desired
//! [`EngineConfig`] and HTTP client.

use std::{
    collections::HashMap as StdHashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use scc::HashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    CompositeLimiter, DownloadState, SegmentDownloader, SpeedLimiter,
    checksum::verify_checksum,
    error::DownloadError,
    event::EventBroadcaster,
    queue::{PriorityQueue, QueueEntry},
    segment::{
        RemoteInfo, ResumeState, SegmentProgressUpdate, calculate_segments,
        compare_content_headers, probe_remote_with_headers,
    },
    storage::{SegmentState, SegmentStatus},
    types::{DownloadEvent, DownloadId, DownloadPriority, DownloadRequest, DownloadStatus},
};

const SCHEDULER_IDLE_COALESCE_DELAY: Duration = Duration::from_millis(50);

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
            min_segment_size: 1024 * 1024,
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
        let downloads = Arc::new(HashMap::new());
        let config = self.config;
        let storage_path = config.storage_path.clone();

        for persisted in load_persisted_downloads(&storage_path) {
            let mut entry = persisted.entry;
            if is_active_state(entry.state) || entry.state == DownloadState::Queued {
                entry.state = DownloadState::Paused;
            }
            let _ = downloads.insert_sync(persisted.id, entry);
        }

        let inner = Arc::new(EngineInner {
            client,
            config: config.clone(),
            events,
            downloads,
            queue: Mutex::new(PriorityQueue::new()),
            active_tasks: Mutex::new(StdHashMap::new()),
            scheduler_running: AtomicBool::new(false),
            concurrency: Arc::new(Semaphore::new(config.max_concurrent_downloads.max(1))),
            global_limiter: config
                .global_speed_limit
                .map(|limit| Arc::new(SpeedLimiter::depleted(limit))),
        });

        let _ = inner.persist_snapshot();

        DownloadEngine { inner }
    }
}

/// Internal state for a tracked download.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DownloadEntry {
    /// The original request.
    request: DownloadRequest,
    /// Current state.
    state: DownloadState,
    /// Bytes downloaded so far.
    bytes_downloaded: u64,
    /// Total bytes if known (from HEAD probe).
    total_bytes: Option<u64>,
    /// ETag from the most recent successful probe.
    etag: Option<String>,
    /// Last-Modified from the most recent successful probe.
    last_modified: Option<String>,
    /// Last failure recorded for this download.
    last_error: Option<String>,
    /// Per-segment completion state used for resume.
    segments: Vec<SegmentState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedDownload {
    id: DownloadId,
    entry: DownloadEntry,
}

struct EngineInner {
    client: hpx::Client,
    config: EngineConfig,
    events: Arc<EventBroadcaster>,
    downloads: Arc<HashMap<DownloadId, DownloadEntry>>,
    queue: Mutex<PriorityQueue>,
    active_tasks: Mutex<StdHashMap<DownloadId, tokio::task::JoinHandle<()>>>,
    scheduler_running: AtomicBool,
    concurrency: Arc<Semaphore>,
    global_limiter: Option<Arc<SpeedLimiter>>,
}

impl std::fmt::Debug for EngineInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineInner")
            .field("client", &"Client")
            .field("config", &self.config)
            .field("events", &self.events)
            .field(
                "active_tasks",
                &self.active_tasks.lock().map_or(0, |m| m.len()),
            )
            .finish_non_exhaustive()
    }
}

impl EngineInner {
    fn snapshot_downloads(&self) -> Vec<PersistedDownload> {
        let mut snapshot = Vec::new();
        self.downloads.iter_sync(|id, entry| {
            snapshot.push(PersistedDownload {
                id: *id,
                entry: entry.clone(),
            });
            true
        });
        snapshot
    }

    fn persist_snapshot(&self) -> Result<(), DownloadError> {
        let snapshot = self.snapshot_downloads();
        let payload = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| DownloadError::Storage(error.to_string()))?;

        if let Some(parent) = self.config.storage_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let temp_path = self.config.storage_path.with_extension("tmp");
        std::fs::write(&temp_path, payload)?;
        std::fs::rename(temp_path, &self.config.storage_path)?;
        Ok(())
    }

    fn transition_state(&self, id: DownloadId, state: DownloadState) -> Result<(), DownloadError> {
        let updated = self.downloads.update_sync(&id, |_, entry| {
            entry.state = state;
            if state != DownloadState::Failed {
                entry.last_error = None;
            }
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }
        let _ = self.events.emit(DownloadEvent::StateChanged { id, state });
        let _ = self.persist_snapshot();
        Ok(())
    }

    fn update_after_probe(
        &self,
        id: DownloadId,
        remote: &RemoteInfo,
        segment_states: Vec<SegmentState>,
        bytes_downloaded: u64,
    ) -> Result<(), DownloadError> {
        let updated = self.downloads.update_sync(&id, |_, entry| {
            entry.total_bytes = remote.content_length;
            entry.bytes_downloaded = bytes_downloaded;
            entry.etag.clone_from(&remote.etag);
            entry.last_modified.clone_from(&remote.last_modified);
            entry.last_error = None;
            entry.segments = segment_states;
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }
        let _ = self.persist_snapshot();
        Ok(())
    }

    fn record_progress(&self, id: DownloadId, delta: u64) -> Result<(), DownloadError> {
        let mut total = None;
        let mut downloaded = None;
        let updated = self.downloads.update_sync(&id, |_, entry| {
            entry.bytes_downloaded = entry.bytes_downloaded.saturating_add(delta);
            total = entry.total_bytes;
            downloaded = Some(entry.bytes_downloaded);
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }
        let _ = self.events.emit(DownloadEvent::Progress {
            id,
            downloaded: downloaded.unwrap_or(0),
            total,
        });
        Ok(())
    }

    fn mark_segment_completed(
        &self,
        id: DownloadId,
        index: u32,
        bytes_downloaded: u64,
    ) -> Result<(), DownloadError> {
        let updated = self.downloads.update_sync(&id, |_, entry| {
            if let Some(segment) = entry
                .segments
                .iter_mut()
                .find(|segment| segment.index == index)
            {
                segment.state = SegmentStatus::Completed;
                segment.bytes_downloaded = bytes_downloaded;
            }
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }
        let _ = self.persist_snapshot();
        Ok(())
    }

    fn complete_download(&self, id: DownloadId, bytes: u64) -> Result<(), DownloadError> {
        let updated = self.downloads.update_sync(&id, |_, entry| {
            entry.state = DownloadState::Completed;
            entry.bytes_downloaded = bytes;
            entry.last_error = None;
            for segment in &mut entry.segments {
                segment.state = SegmentStatus::Completed;
                segment.bytes_downloaded = segment.end - segment.start + 1;
            }
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }
        let _ = self.events.emit(DownloadEvent::Completed { id });
        let _ = self.persist_snapshot();
        Ok(())
    }

    fn fail_download(&self, id: DownloadId, error: &DownloadError) -> Result<(), DownloadError> {
        let message = error.to_string();
        let updated = self.downloads.update_sync(&id, |_, entry| {
            entry.state = DownloadState::Failed;
            entry.last_error = Some(message.clone());
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }
        let _ = self
            .events
            .emit(DownloadEvent::Failed { id, error: message });
        let _ = self.persist_snapshot();
        Ok(())
    }

    fn requeue(&self, id: DownloadId) -> Result<(), DownloadError> {
        let mut request = None;
        let mut priority = DownloadPriority::default();

        let updated = self.downloads.update_sync(&id, |_, entry| {
            if entry.state != DownloadState::Paused && entry.state != DownloadState::Failed {
                return;
            }
            entry.state = DownloadState::Queued;
            entry.last_error = None;
            request = Some(entry.request.clone());
            priority = entry.request.priority;
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }

        let request = request.ok_or_else(|| DownloadError::InvalidState {
            expected: "paused or failed".to_string(),
            actual: self
                .downloads
                .read_sync(&id, |_, entry| entry.state.to_string())
                .unwrap_or_else(|| "missing".to_string()),
        })?;

        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(QueueEntry::new(id, priority, request, 0));

        let _ = self.persist_snapshot();
        let _ = self.events.emit(DownloadEvent::StateChanged {
            id,
            state: DownloadState::Queued,
        });
        Ok(())
    }

    fn pause_download(&self, id: DownloadId) -> Result<(), DownloadError> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);

        if let Some(handle) = self
            .active_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id)
        {
            handle.abort();
        }

        let updated = self.downloads.update_sync(&id, |_, entry| {
            entry.state = DownloadState::Paused;
            entry.last_error = None;
        });
        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }

        let _ = self.persist_snapshot();
        let _ = self.events.emit(DownloadEvent::StateChanged {
            id,
            state: DownloadState::Paused,
        });
        Ok(())
    }

    fn remove_download(&self, id: DownloadId) -> Result<(), DownloadError> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);

        if let Some(handle) = self
            .active_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id)
        {
            handle.abort();
        }

        let removed = self.downloads.remove_sync(&id);
        if removed.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }

        let _ = self.persist_snapshot();
        let _ = self.events.emit(DownloadEvent::Removed { id });
        Ok(())
    }

    fn trigger_scheduler(self: &Arc<Self>) {
        if self
            .scheduler_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let inner = Arc::clone(self);
        tokio::spawn(async move {
            inner.scheduler_loop().await;
        });
    }

    async fn scheduler_loop(self: Arc<Self>) {
        if self.should_coalesce_idle_start() {
            tokio::time::sleep(SCHEDULER_IDLE_COALESCE_DELAY).await;
        }

        loop {
            let permit = match Arc::clone(&self.concurrency).try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => break,
            };

            let next = self
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop();
            let Some(entry) = next else {
                drop(permit);
                break;
            };

            let state = self
                .downloads
                .read_sync(&entry.id, |_, item| item.state)
                .unwrap_or(DownloadState::Failed);
            if state != DownloadState::Queued {
                drop(permit);
                continue;
            }

            self.spawn_download(entry, permit);
        }

        self.scheduler_running.store(false, Ordering::Release);

        let should_retry = !self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
            && self.concurrency.available_permits() > 0;
        if should_retry {
            self.trigger_scheduler();
        }
    }

    fn spawn_download(self: &Arc<Self>, queue_entry: QueueEntry, permit: OwnedSemaphorePermit) {
        let id = queue_entry.id;
        let request = queue_entry.request;
        let inner = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let _permit = permit;
            Arc::clone(&inner).execute_download(id, request).await;
            inner
                .active_tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            inner.trigger_scheduler();
        });

        self.active_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, handle);
    }

    fn should_coalesce_idle_start(&self) -> bool {
        self.active_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
            && self.concurrency.available_permits() == self.config.max_concurrent_downloads.max(1)
    }

    async fn execute_download(self: Arc<Self>, id: DownloadId, request: DownloadRequest) {
        let candidate_urls = std::iter::once(request.url.clone()).chain(request.mirrors.clone());
        let mut last_error = None;

        for candidate_url in candidate_urls {
            match self
                .download_from_candidate(id, &request, &candidate_url)
                .await
            {
                Ok(()) => return,
                Err(error) => last_error = Some(error),
            }
        }

        if let Some(error) = last_error {
            let _ = self.fail_download(id, &error);
        }
    }

    async fn download_from_candidate(
        &self,
        id: DownloadId,
        request: &DownloadRequest,
        candidate_url: &str,
    ) -> Result<(), DownloadError> {
        self.transition_state(id, DownloadState::Connecting)?;

        let client = build_client(&self.client, request.proxy.as_ref())?;
        let remote = probe_remote_with_headers(&client, candidate_url, &request.headers).await?;
        let total_bytes = remote.content_length.unwrap_or(0);
        let max_connections = request
            .max_connections
            .unwrap_or(self.config.max_connections_per_download)
            .max(1);
        let effective_connections = if remote.accept_ranges {
            max_connections
        } else {
            1
        };
        let segments = calculate_segments(
            total_bytes,
            effective_connections,
            self.config.min_segment_size,
        );

        let resume_state = self.resume_state(id, &remote, &segments)?;
        let segment_states = build_segment_states(&segments, &resume_state);
        let bytes_completed = bytes_completed(&segment_states);
        self.update_after_probe(id, &remote, segment_states, bytes_completed)?;

        ensure_destination_parent(&request.destination).await?;

        if total_bytes == 0 {
            tokio::fs::File::create(&request.destination).await?;
            if let Some(checksum) = &request.checksum {
                verify_checksum(checksum, &request.destination).await?;
            }
            self.complete_download(id, 0)?;
            return Ok(());
        }

        self.transition_state(id, DownloadState::Downloading)?;
        let _ = self.events.emit(DownloadEvent::Started { id });
        if bytes_completed > 0 {
            let _ = self.events.emit(DownloadEvent::Progress {
                id,
                downloaded: bytes_completed,
                total: remote.content_length,
            });
        }

        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(128);
        let (segment_tx, mut segment_rx) = tokio::sync::mpsc::unbounded_channel();

        let limiter = CompositeLimiter::new(
            self.global_limiter.clone(),
            request
                .speed_limit
                .map(|limit| Arc::new(SpeedLimiter::depleted(limit))),
        );

        let mut downloader = if let Some(proxy) = request.proxy.as_ref() {
            SegmentDownloader::with_proxy(
                candidate_url.to_string(),
                segments.clone(),
                request.destination.clone(),
                proxy,
            )?
        } else {
            SegmentDownloader::new(
                client,
                candidate_url.to_string(),
                segments.clone(),
                request.destination.clone(),
            )
        };

        downloader = downloader
            .with_retry_config(
                self.config.retry_max_attempts,
                self.config.retry_initial_delay,
                self.config.retry_max_delay,
                self.config.retry_jitter,
            )
            .with_headers(request.headers.clone())
            .with_limiter(limiter)
            .with_segment_updates(segment_tx);

        let download_result = downloader.download_with_resume(resume_state, Some(progress_tx));
        tokio::pin!(download_result);

        loop {
            tokio::select! {
                result = &mut download_result => {
                    while let Ok(update) = segment_rx.try_recv() {
                        let SegmentProgressUpdate::Completed {
                            index,
                            bytes_downloaded,
                        } = update;
                        let _ = self.mark_segment_completed(id, index, bytes_downloaded);
                    }

                    match result {
                        Ok(bytes) => {
                            if let Some(checksum) = &request.checksum {
                                verify_checksum(checksum, &request.destination).await?;
                            }
                            self.complete_download(id, bytes_completed.saturating_add(bytes))?;
                            return Ok(());
                        }
                        Err(error) => {
                            return Err(error);
                        }
                    }
                }
                maybe_delta = progress_rx.recv() => {
                    if let Some(delta) = maybe_delta {
                        let _ = self.record_progress(id, delta);
                    }
                }
                maybe_update = segment_rx.recv() => {
                    if let Some(SegmentProgressUpdate::Completed { index, bytes_downloaded }) = maybe_update {
                        let _ = self.mark_segment_completed(id, index, bytes_downloaded);
                    }
                }
            }
        }
    }

    fn resume_state(
        &self,
        id: DownloadId,
        remote: &RemoteInfo,
        segments: &[crate::segment::SegmentRange],
    ) -> Result<ResumeState, DownloadError> {
        let Some(entry) = self.downloads.read_sync(&id, |_, entry| entry.clone()) else {
            return Err(DownloadError::NotFound(id.0));
        };

        let file_exists = entry.request.destination.exists();
        if !file_exists || entry.segments.is_empty() || entry.segments.len() != segments.len() {
            return Ok(ResumeState::Fresh);
        }

        if !compare_content_headers(
            &entry.etag,
            &entry.last_modified,
            &remote.etag,
            &remote.last_modified,
        ) {
            return Ok(ResumeState::ContentChanged);
        }

        let completed_segments: Vec<u32> = entry
            .segments
            .iter()
            .filter(|segment| segment.state == SegmentStatus::Completed)
            .map(|segment| segment.index)
            .collect();

        if completed_segments.is_empty() {
            return Ok(ResumeState::Fresh);
        }

        let bytes_completed = entry
            .segments
            .iter()
            .filter(|segment| segment.state == SegmentStatus::Completed)
            .map(|segment| segment.bytes_downloaded)
            .sum();

        Ok(ResumeState::CanResume {
            bytes_completed,
            completed_segments,
        })
    }

    fn shutdown(&self) {
        let handles = std::mem::take(
            &mut *self
                .active_tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (_, handle) in handles {
            handle.abort();
        }

        let mut ids = Vec::new();
        self.downloads.iter_sync(|id, _| {
            ids.push(*id);
            true
        });

        for id in ids {
            let _ = self.downloads.update_sync(&id, |_, entry| {
                if is_active_state(entry.state) || entry.state == DownloadState::Queued {
                    entry.state = DownloadState::Paused;
                    entry.last_error = None;
                }
            });
        }

        let _ = self.persist_snapshot();
    }
}

/// Central coordinator for managing downloads.
///
/// Construct via [`DownloadEngine::builder`] or [`EngineBuilder`].
pub struct DownloadEngine {
    inner: Arc<EngineInner>,
}

impl std::fmt::Debug for DownloadEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadEngine")
            .field("inner", &self.inner)
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
        self.inner.events.subscribe()
    }

    /// Emit a download event to all subscribers.
    pub(crate) fn emit(&self, event: DownloadEvent) -> Result<usize, crate::event::EventError> {
        self.inner.events.emit(event)
    }

    /// Return a reference to the engine configuration.
    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        &self.inner.config
    }

    /// Return a reference to the HTTP client.
    #[must_use]
    pub fn client(&self) -> &hpx::Client {
        &self.inner.client
    }

    /// Queue a new download.
    pub fn add(&self, request: DownloadRequest) -> Result<DownloadId, DownloadError> {
        let id = DownloadId::new();
        let entry = DownloadEntry {
            request: request.clone(),
            state: DownloadState::Queued,
            bytes_downloaded: 0,
            total_bytes: None,
            etag: None,
            last_modified: None,
            last_error: None,
            segments: Vec::new(),
        };

        let inserted = self.inner.downloads.insert_sync(id, entry);
        if inserted.is_err() {
            return Err(DownloadError::AlreadyExists(id.0));
        }

        self.inner
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(QueueEntry::new(id, request.priority, request, 0));

        let _ = self.inner.persist_snapshot();
        let _ = self.emit(DownloadEvent::Added { id });
        self.inner.trigger_scheduler();
        Ok(id)
    }

    /// Pause an active download.
    pub fn pause(&self, id: DownloadId) -> Result<(), DownloadError> {
        self.inner.pause_download(id)
    }

    /// Resume a paused or failed download.
    pub fn resume(&self, id: DownloadId) -> Result<(), DownloadError> {
        self.inner.requeue(id)?;
        self.inner.trigger_scheduler();
        Ok(())
    }

    /// Remove a download from the queue or registry.
    pub fn remove(&self, id: DownloadId) -> Result<(), DownloadError> {
        self.inner.remove_download(id)
    }

    /// Get the current status of a download.
    pub fn status(&self, id: DownloadId) -> Result<DownloadStatus, DownloadError> {
        let status = self
            .inner
            .downloads
            .read_sync(&id, |_, entry| DownloadStatus {
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
        self.inner.downloads.iter_sync(|id, entry| {
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

impl Drop for DownloadEngine {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.inner.shutdown();
        }
    }
}

fn load_persisted_downloads(path: &Path) -> Vec<PersistedDownload> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };

    match serde_json::from_slice::<Vec<PersistedDownload>>(&bytes) {
        Ok(downloads) => downloads,
        Err(error) => {
            tracing::warn!(path = %path.display(), error = %error, "ignoring invalid persisted download state");
            Vec::new()
        }
    }
}

fn build_segment_states(
    segments: &[crate::segment::SegmentRange],
    resume_state: &ResumeState,
) -> Vec<SegmentState> {
    let completed_indices: &[u32] = match resume_state {
        ResumeState::CanResume {
            completed_segments, ..
        } => completed_segments,
        _ => &[],
    };

    segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let completed = completed_indices.contains(&(index as u32));
            SegmentState {
                index: index as u32,
                start: segment.start,
                end: segment.end,
                state: if completed {
                    SegmentStatus::Completed
                } else {
                    SegmentStatus::Pending
                },
                bytes_downloaded: if completed {
                    segment.end - segment.start + 1
                } else {
                    0
                },
            }
        })
        .collect()
}

fn bytes_completed(segments: &[SegmentState]) -> u64 {
    segments
        .iter()
        .filter(|segment| segment.state == SegmentStatus::Completed)
        .map(|segment| segment.bytes_downloaded)
        .sum()
}

const fn is_active_state(state: DownloadState) -> bool {
    matches!(
        state,
        DownloadState::Connecting | DownloadState::Downloading
    )
}

fn build_client(
    base: &hpx::Client,
    proxy: Option<&crate::types::ProxyConfig>,
) -> Result<hpx::Client, DownloadError> {
    if let Some(proxy) = proxy {
        let proxy = hpx::Proxy::try_from(proxy.clone())?;
        hpx::Client::builder()
            .proxy(proxy)
            .build()
            .map_err(Into::into)
    } else {
        Ok(base.clone())
    }
}

async fn ensure_destination_parent(path: &Path) -> Result<(), DownloadError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    Ok(())
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
        assert_eq!(engine.config().retry_max_attempts, 3);
    }

    #[test]
    fn persisted_active_downloads_resume_as_paused() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage_path = temp.path().join("state.json");
        let request =
            DownloadRequest::builder("https://example.com/file.bin", temp.path().join("file.bin"))
                .build();
        let persisted = vec![PersistedDownload {
            id: DownloadId::new(),
            entry: DownloadEntry {
                request,
                state: DownloadState::Downloading,
                bytes_downloaded: 42,
                total_bytes: Some(100),
                etag: Some("etag-1".to_string()),
                last_modified: Some("now".to_string()),
                last_error: None,
                segments: Vec::new(),
            },
        }];
        std::fs::write(
            &storage_path,
            serde_json::to_vec(&persisted).expect("serialize"),
        )
        .expect("persist state");

        let engine = EngineBuilder::new().storage_path(&storage_path).build();
        let statuses = engine.list().expect("status list");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].state, DownloadState::Paused);
    }

    #[test]
    fn resume_state_marks_completed_segments() {
        let segments = vec![
            crate::segment::SegmentRange::new(0, 9),
            crate::segment::SegmentRange::new(10, 19),
        ];
        let state = ResumeState::CanResume {
            bytes_completed: 10,
            completed_segments: vec![0],
        };
        let built = build_segment_states(&segments, &state);
        assert_eq!(built[0].state, SegmentStatus::Completed);
        assert_eq!(built[0].bytes_downloaded, 10);
        assert_eq!(built[1].state, SegmentStatus::Pending);
    }
}
