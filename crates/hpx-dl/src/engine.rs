//! Download engine: central coordinator for download operations.
//!
//! The [`DownloadEngine`] owns download lifecycle, persistence, scheduling,
//! and event fan-out. Use [`EngineBuilder`] to construct one with the desired
//! [`EngineConfig`] and HTTP client.

use std::{
    collections::HashMap as StdHashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use scc::HashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(feature = "sqlite")]
use crate::storage::SqliteStorage;
use crate::{
    CompositeLimiter, DownloadState, SegmentDownloader, SpeedLimiter,
    checksum::verify_checksum,
    error::DownloadError,
    event::EventBroadcaster,
    persistence::PersistenceHandle,
    queue::{PriorityQueue, QueueEntry},
    segment::{
        RemoteInfo, ResumeState, SegmentProgressUpdate, calculate_segments,
        probe_remote_with_headers, resume_state_from_segments,
    },
    storage::{DownloadRecord, SegmentState, SegmentStatus, Storage},
    types::{DownloadEvent, DownloadId, DownloadRequest, DownloadStatus},
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
    storage: Option<Arc<dyn Storage>>,
}

impl std::fmt::Debug for EngineBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineBuilder")
            .field("client", &self.client.as_ref().map(|_| "Client"))
            .field("config", &self.config)
            .field("storage", &self.storage.as_ref().map(|_| "Storage"))
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

    /// Inject a custom storage backend.
    #[must_use]
    pub fn storage(mut self, storage: Arc<dyn Storage>) -> Self {
        self.storage = Some(storage);
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
    pub fn build(self) -> Result<DownloadEngine, DownloadError> {
        let client = self.client.unwrap_or_default();
        let events = Arc::new(EventBroadcaster::new(256));
        let downloads = Arc::new(HashMap::new());
        let config = self.config;
        let storage = build_storage_backend(&config.storage_path, self.storage)?;

        for mut record in load_records_from_storage(&storage)? {
            if is_active_state(record.state) || record.state == DownloadState::Queued {
                record.state = DownloadState::Paused;
                record.last_error = None;
                record.updated_at = current_timestamp();
                persist_record_direct(&storage, &record)?;
            }

            let _ = downloads.insert_sync(record.id, DownloadEntry::from_record(record));
        }

        let persistence = Arc::new(PersistenceHandle::start(Arc::clone(&storage))?);

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
            persistence,
        });

        Ok(DownloadEngine { inner })
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
    /// Unix timestamp (seconds) when the entry was created.
    #[serde(default = "current_timestamp")]
    created_at: i64,
    /// Unix timestamp (seconds) when the entry was last updated.
    #[serde(default = "current_timestamp")]
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedDownload {
    id: DownloadId,
    entry: DownloadEntry,
}

impl DownloadEntry {
    fn new(request: DownloadRequest) -> Self {
        let now = current_timestamp();
        Self {
            request,
            state: DownloadState::Queued,
            bytes_downloaded: 0,
            total_bytes: None,
            etag: None,
            last_modified: None,
            last_error: None,
            segments: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn from_record(mut record: DownloadRecord) -> Self {
        record.sync_request_fields();
        Self {
            request: record.request,
            state: record.state,
            bytes_downloaded: record.bytes_downloaded,
            total_bytes: record.content_length,
            etag: record.etag,
            last_modified: record.last_modified,
            last_error: record.last_error,
            segments: record.segments,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }

    fn to_record(&self, id: DownloadId) -> DownloadRecord {
        DownloadRecord {
            id,
            request: self.request.clone(),
            url: self.request.url.clone(),
            destination: self.request.destination.clone(),
            state: self.state,
            priority: self.request.priority,
            etag: self.etag.clone(),
            last_modified: self.last_modified.clone(),
            content_length: self.total_bytes,
            bytes_downloaded: self.bytes_downloaded,
            last_error: self.last_error.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            segments: self.segments.clone(),
        }
    }

    fn touch(&mut self) {
        self.updated_at = current_timestamp();
    }
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
    persistence: Arc<PersistenceHandle>,
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
    fn entry_snapshot(&self, id: DownloadId) -> Result<DownloadEntry, DownloadError> {
        self.downloads
            .read_sync(&id, |_, entry| entry.clone())
            .ok_or(DownloadError::NotFound(id.0))
    }

    fn replace_entry(
        &self,
        id: DownloadId,
        next_entry: DownloadEntry,
    ) -> Result<(), DownloadError> {
        let mut replacement = Some(next_entry);
        let updated = self.downloads.update_sync(&id, move |_, entry| {
            if let Some(replacement) = replacement.take() {
                *entry = replacement;
            }
        });

        if updated.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }

        Ok(())
    }

    async fn transition_state(
        &self,
        id: DownloadId,
        state: DownloadState,
    ) -> Result<(), DownloadError> {
        let mut entry = self.entry_snapshot(id)?;
        entry.state = state;
        if state != DownloadState::Failed {
            entry.last_error = None;
        }
        entry.touch();

        let record = entry.to_record(id);
        Arc::clone(&self.persistence).upsert_async(record).await?;
        self.replace_entry(id, entry)?;

        let _ = self.events.emit(DownloadEvent::StateChanged { id, state });
        Ok(())
    }

    async fn update_after_probe(
        &self,
        id: DownloadId,
        remote: &RemoteInfo,
        segment_states: Vec<SegmentState>,
        bytes_downloaded: u64,
    ) -> Result<(), DownloadError> {
        let mut entry = self.entry_snapshot(id)?;
        entry.total_bytes = remote.content_length;
        entry.bytes_downloaded = bytes_downloaded;
        entry.etag.clone_from(&remote.etag);
        entry.last_modified.clone_from(&remote.last_modified);
        entry.last_error = None;
        entry.segments = segment_states;
        entry.touch();

        let record = entry.to_record(id);
        Arc::clone(&self.persistence).upsert_async(record).await?;
        self.replace_entry(id, entry)
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

    async fn mark_segment_completed(
        &self,
        id: DownloadId,
        index: u32,
        bytes_downloaded: u64,
    ) -> Result<(), DownloadError> {
        let mut entry = self.entry_snapshot(id)?;
        if let Some(segment) = entry
            .segments
            .iter_mut()
            .find(|segment| segment.index == index)
        {
            segment.state = SegmentStatus::Completed;
            segment.bytes_downloaded = bytes_downloaded;
        }
        entry.touch();

        let record = entry.to_record(id);
        Arc::clone(&self.persistence).upsert_async(record).await?;
        self.replace_entry(id, entry)
    }

    async fn complete_download(&self, id: DownloadId, bytes: u64) -> Result<(), DownloadError> {
        let mut entry = self.entry_snapshot(id)?;
        entry.state = DownloadState::Completed;
        entry.bytes_downloaded = bytes;
        entry.last_error = None;
        for segment in &mut entry.segments {
            segment.state = SegmentStatus::Completed;
            segment.bytes_downloaded = segment.end - segment.start + 1;
        }
        entry.touch();

        let record = entry.to_record(id);
        Arc::clone(&self.persistence).upsert_async(record).await?;
        self.replace_entry(id, entry)?;

        let _ = self.events.emit(DownloadEvent::Completed { id });
        Ok(())
    }

    async fn fail_download(
        &self,
        id: DownloadId,
        error: &DownloadError,
    ) -> Result<(), DownloadError> {
        let message = error.to_string();
        let mut entry = self.entry_snapshot(id)?;
        entry.state = DownloadState::Failed;
        entry.last_error = Some(message.clone());
        entry.touch();

        let record = entry.to_record(id);
        Arc::clone(&self.persistence).upsert_async(record).await?;
        self.replace_entry(id, entry)?;

        let _ = self
            .events
            .emit(DownloadEvent::Failed { id, error: message });
        Ok(())
    }

    fn requeue(&self, id: DownloadId) -> Result<(), DownloadError> {
        let mut entry = self.entry_snapshot(id)?;
        if entry.state != DownloadState::Paused && entry.state != DownloadState::Failed {
            return Err(DownloadError::InvalidState {
                expected: "paused or failed".to_string(),
                actual: entry.state.to_string(),
            });
        }

        entry.state = DownloadState::Queued;
        entry.last_error = None;
        entry.touch();

        let request = entry.request.clone();
        let priority = request.priority;
        self.persistence.upsert(entry.to_record(id))?;
        self.replace_entry(id, entry)?;

        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(QueueEntry::new(id, priority, request, 0));

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

        let mut entry = self.entry_snapshot(id)?;
        entry.state = DownloadState::Paused;
        entry.last_error = None;
        entry.touch();

        self.persistence.upsert(entry.to_record(id))?;
        self.replace_entry(id, entry)?;

        let _ = self.events.emit(DownloadEvent::StateChanged {
            id,
            state: DownloadState::Paused,
        });
        Ok(())
    }

    fn remove_download(&self, id: DownloadId) -> Result<(), DownloadError> {
        let _ = self.entry_snapshot(id)?;

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

        self.persistence.delete(id)?;

        let removed = self.downloads.remove_sync(&id);
        if removed.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }

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
            let _ = self.fail_download(id, &error).await;
        }
    }

    async fn download_from_candidate(
        &self,
        id: DownloadId,
        request: &DownloadRequest,
        candidate_url: &str,
    ) -> Result<(), DownloadError> {
        self.transition_state(id, DownloadState::Connecting).await?;

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
        self.update_after_probe(id, &remote, segment_states, bytes_completed)
            .await?;

        ensure_destination_parent(&request.destination).await?;

        if total_bytes == 0 {
            tokio::fs::File::create(&request.destination).await?;
            if let Some(checksum) = &request.checksum {
                verify_checksum(checksum, &request.destination).await?;
            }
            self.complete_download(id, 0).await?;
            return Ok(());
        }

        self.transition_state(id, DownloadState::Downloading)
            .await?;
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
                        let _ = self.mark_segment_completed(id, index, bytes_downloaded).await;
                    }

                    match result {
                        Ok(bytes) => {
                            if let Some(checksum) = &request.checksum {
                                verify_checksum(checksum, &request.destination).await?;
                            }
                            self.complete_download(id, bytes_completed.saturating_add(bytes))
                                .await?;
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
                        let _ = self.mark_segment_completed(id, index, bytes_downloaded).await;
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

        Ok(resume_state_from_segments(
            &entry.etag,
            &entry.last_modified,
            &remote.etag,
            &remote.last_modified,
            &entry.segments,
        ))
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
                    entry.touch();
                }
            });

            if let Ok(entry) = self.entry_snapshot(id) {
                let _ = self.persistence.upsert(entry.to_record(id));
            }
        }
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
        let entry = DownloadEntry::new(request.clone());

        self.inner.persistence.upsert(entry.to_record(id))?;

        let inserted = self.inner.downloads.insert_sync(id, entry);
        if inserted.is_err() {
            let _ = self.inner.persistence.delete(id);
            return Err(DownloadError::AlreadyExists(id.0));
        }

        self.inner
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(QueueEntry::new(id, request.priority, request, 0));

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

fn build_storage_backend(
    storage_path: &Path,
    storage: Option<Arc<dyn Storage>>,
) -> Result<Arc<dyn Storage>, DownloadError> {
    if let Some(storage) = storage {
        return Ok(storage);
    }

    #[cfg(feature = "sqlite")]
    {
        if let Some(parent) = storage_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let legacy_records = load_legacy_snapshot(storage_path)?;
        if legacy_records.is_some() {
            backup_legacy_snapshot(storage_path)?;
        }
        let storage: Arc<dyn Storage> =
            Arc::new(block_on_storage(SqliteStorage::new(storage_path))?);

        if let Some(records) = legacy_records {
            for record in &records {
                persist_record_direct(&storage, record)?;
            }
        }

        Ok(storage)
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = storage_path;
        Err(DownloadError::InvalidConfiguration(
            "engine storage requires the sqlite feature or an injected Storage backend".to_string(),
        ))
    }
}

fn load_records_from_storage(
    storage: &Arc<dyn Storage>,
) -> Result<Vec<DownloadRecord>, DownloadError> {
    block_on_storage(storage.list())
}

fn persist_record_direct(
    storage: &Arc<dyn Storage>,
    record: &DownloadRecord,
) -> Result<(), DownloadError> {
    block_on_storage(storage.upsert(record))
}

fn block_on_storage<T>(
    future: impl Future<Output = Result<T, DownloadError>> + Send,
) -> Result<T, DownloadError>
where
    T: Send,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        return std::thread::scope(|scope| {
            let join_handle = scope.spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| DownloadError::Storage(error.to_string()))?;
                runtime.block_on(future)
            });

            join_handle.join().map_err(storage_thread_panic)?
        });
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| DownloadError::Storage(error.to_string()))?;
    runtime.block_on(future)
}

fn storage_thread_panic(panic: Box<dyn std::any::Any + Send + 'static>) -> DownloadError {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return DownloadError::Storage((*message).to_string());
    }

    if let Some(message) = panic.downcast_ref::<String>() {
        return DownloadError::Storage(message.clone());
    }

    DownloadError::Storage("storage worker panicked".to_string())
}

fn load_legacy_snapshot(path: &Path) -> Result<Option<Vec<DownloadRecord>>, DownloadError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(DownloadError::Io(error)),
    };

    let first_non_whitespace = bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace());
    if first_non_whitespace.is_none() {
        return Ok(None);
    }
    if first_non_whitespace != Some(b'[') {
        return Ok(None);
    }

    let downloads = serde_json::from_slice::<Vec<PersistedDownload>>(&bytes).map_err(|error| {
        DownloadError::Storage(format!(
            "failed to migrate legacy snapshot {}: {error}",
            path.display()
        ))
    })?;

    Ok(Some(
        downloads
            .into_iter()
            .map(|persisted| persisted.entry.to_record(persisted.id))
            .collect(),
    ))
}

fn backup_legacy_snapshot(path: &Path) -> Result<(), DownloadError> {
    let backup_path = path.with_extension("legacy-json.bak");
    if backup_path.exists() {
        std::fs::remove_file(&backup_path)?;
    }
    std::fs::rename(path, backup_path)?;
    Ok(())
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
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
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;

    use super::*;
    use crate::{
        DownloadPriority,
        storage::{DownloadRecord, Storage},
    };

    #[derive(Debug, Default)]
    struct TestStorage {
        records: Mutex<HashMap<DownloadId, DownloadRecord>>,
        fail_save: bool,
    }

    impl TestStorage {
        fn with_record(record: DownloadRecord) -> Self {
            let mut records = HashMap::new();
            records.insert(record.id, record);
            Self {
                records: Mutex::new(records),
                fail_save: false,
            }
        }

        fn failing_save() -> Self {
            Self {
                records: Mutex::new(HashMap::new()),
                fail_save: true,
            }
        }
    }

    #[async_trait]
    impl Storage for TestStorage {
        async fn save(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
            if self.fail_save {
                return Err(DownloadError::Storage("save failed".to_string()));
            }

            let mut records = self.records.lock().unwrap();
            if records.insert(download.id, download.clone()).is_some() {
                return Err(DownloadError::AlreadyExists(download.id.0));
            }
            Ok(())
        }

        async fn load(&self, id: DownloadId) -> Result<Option<DownloadRecord>, DownloadError> {
            let records = self.records.lock().unwrap();
            Ok(records.get(&id).cloned())
        }

        async fn list(&self) -> Result<Vec<DownloadRecord>, DownloadError> {
            let records = self.records.lock().unwrap();
            Ok(records.values().cloned().collect())
        }

        async fn delete(&self, id: DownloadId) -> Result<(), DownloadError> {
            let mut records = self.records.lock().unwrap();
            records.remove(&id);
            Ok(())
        }

        async fn update_progress(
            &self,
            id: DownloadId,
            segments: &[SegmentState],
        ) -> Result<(), DownloadError> {
            let mut records = self.records.lock().unwrap();
            let Some(record) = records.get_mut(&id) else {
                return Err(DownloadError::NotFound(id.0));
            };

            record.segments = segments.to_vec();
            record.bytes_downloaded = record
                .segments
                .iter()
                .map(|segment| segment.bytes_downloaded)
                .sum();
            Ok(())
        }
    }

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
        let engine = EngineBuilder::new().build().expect("build engine");
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
            .build()
            .expect("build engine");

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
        let now = 1_700_000_000;
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
                created_at: now,
                updated_at: now,
            },
        }];
        std::fs::write(
            &storage_path,
            serde_json::to_vec(&persisted).expect("serialize"),
        )
        .expect("persist state");

        let engine = EngineBuilder::new()
            .storage_path(&storage_path)
            .build()
            .expect("build engine");
        let statuses = engine.list().expect("status list");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].state, DownloadState::Paused);
    }

    #[test]
    fn builder_rehydrates_persisted_downloads_from_storage() {
        let id = DownloadId::new();
        let now = 1_700_000_000;
        let request = DownloadRequest::builder("https://example.com/file.bin", "/tmp/file.bin")
            .priority(DownloadPriority::Normal)
            .build();
        let storage = Arc::new(TestStorage::with_record(DownloadRecord {
            id,
            request: request.clone(),
            url: request.url.clone(),
            destination: request.destination.clone(),
            state: DownloadState::Downloading,
            priority: request.priority,
            etag: Some("etag".to_string()),
            last_modified: Some("now".to_string()),
            content_length: Some(128),
            bytes_downloaded: 64,
            last_error: None,
            created_at: now,
            updated_at: now,
            segments: Vec::new(),
        }));

        let engine = EngineBuilder::new()
            .storage(storage)
            .build()
            .expect("build with custom storage");

        let statuses = engine.list().expect("status list");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].id, id);
        assert_eq!(statuses[0].state, DownloadState::Paused);
    }

    #[test]
    fn add_returns_storage_error_when_persistence_fails() {
        let engine = EngineBuilder::new()
            .storage(Arc::new(TestStorage::failing_save()))
            .build()
            .expect("build with failing storage");

        let request =
            DownloadRequest::builder("https://example.com/file.bin", "/tmp/file.bin").build();
        let error = engine
            .add(request)
            .expect_err("add should surface storage failure");
        assert!(
            matches!(error, DownloadError::Storage(message) if message.contains("save failed"))
        );
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
