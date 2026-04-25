//! Pluggable storage backends for download metadata persistence.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{DownloadError, DownloadId, DownloadPriority, DownloadRequest, DownloadState};

/// Persistent record for a single download job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    /// Unique identifier for this download.
    pub id: DownloadId,
    /// Full request payload needed to resume the download after restart.
    pub request: DownloadRequest,
    /// Source URL.
    pub url: String,
    /// Local destination path.
    pub destination: PathBuf,
    /// Current download state.
    pub state: DownloadState,
    /// Scheduling priority.
    pub priority: DownloadPriority,
    /// ETag from the server response.
    pub etag: Option<String>,
    /// Last-Modified header from the server response.
    pub last_modified: Option<String>,
    /// Content-Length from the server response.
    pub content_length: Option<u64>,
    /// Total bytes downloaded so far.
    pub bytes_downloaded: u64,
    /// Last failure recorded for this download.
    pub last_error: Option<String>,
    /// Unix timestamp (seconds) when the record was created.
    pub created_at: i64,
    /// Unix timestamp (seconds) when the record was last updated.
    pub updated_at: i64,
    /// Per-segment progress state.
    pub segments: Vec<SegmentState>,
}

impl DownloadRecord {
    /// Keep convenience fields in sync with the canonical request payload.
    pub fn sync_request_fields(&mut self) {
        self.url = self.request.url.clone();
        self.destination = self.request.destination.clone();
        self.priority = self.request.priority;
    }
}

/// Status of an individual segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SegmentStatus {
    /// Segment has not been started.
    Pending,
    /// Segment is actively downloading.
    Downloading,
    /// Segment finished successfully.
    Completed,
    /// Segment failed.
    Failed,
}

/// Progress state for a single segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentState {
    /// Zero-based segment index.
    pub index: u32,
    /// Byte offset where this segment starts (inclusive).
    pub start: u64,
    /// Byte offset where this segment ends (inclusive).
    pub end: u64,
    /// Current status of this segment.
    pub state: SegmentStatus,
    /// Bytes downloaded within this segment.
    pub bytes_downloaded: u64,
}

/// Trait for storage backends that persist download metadata.
#[async_trait]
pub trait Storage: Send + Sync {
    /// Persist a download record.
    ///
    /// Returns [`DownloadError::AlreadyExists`] if a record with the same ID
    /// already exists.
    async fn save(&self, download: &DownloadRecord) -> Result<(), DownloadError>;

    /// Load a download record by ID.
    ///
    /// Returns `Ok(None)` when no record exists for the given ID.
    async fn load(&self, id: DownloadId) -> Result<Option<DownloadRecord>, DownloadError>;

    /// List all persisted download records.
    async fn list(&self) -> Result<Vec<DownloadRecord>, DownloadError>;

    /// Delete a download record by ID.
    ///
    /// Returns `Ok(())` even if the record does not exist.
    async fn delete(&self, id: DownloadId) -> Result<(), DownloadError>;

    /// Update the segment progress for an existing download.
    ///
    /// Returns [`DownloadError::NotFound`] when no record exists for the given
    /// ID.
    async fn update_progress(
        &self,
        id: DownloadId,
        segments: &[SegmentState],
    ) -> Result<(), DownloadError>;

    /// Insert or replace a download record atomically.
    async fn upsert(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
        let mut normalized = download.clone();
        normalized.sync_request_fields();

        if self.load(normalized.id).await?.is_some() {
            self.delete(normalized.id).await?;
        }

        self.save(&normalized).await
    }
}

// ---------------------------------------------------------------------------
// MemoryStorage – in-memory reference implementation (test feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "test")]
mod memory;

#[cfg(feature = "test")]
pub use memory::MemoryStorage;

// ---------------------------------------------------------------------------
// SqliteStorage – SQLite-backed persistent storage
// ---------------------------------------------------------------------------

#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStorage;
