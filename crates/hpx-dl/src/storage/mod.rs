//! Pluggable storage backends for download metadata persistence.

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
    /// Current download state.
    pub state: DownloadState,
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
    /// Source URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.request.url
    }

    /// Local destination path.
    #[must_use]
    pub fn destination(&self) -> &std::path::Path {
        &self.request.destination
    }

    /// Scheduling priority.
    #[must_use]
    pub const fn priority(&self) -> DownloadPriority {
        self.request.priority
    }

    /// Strip sensitive credentials (proxy userinfo) before persisting.
    pub fn sanitize_for_persistence(&mut self) {
        if let Some(ref mut proxy) = self.request.proxy
            && let Ok(mut url) = url::Url::parse(&proxy.url)
        {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            proxy.url = url.into();
        }
    }
}

/// Status of an individual segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SegmentStatus {
    /// Segment has not been started.
    Pending,
    /// Segment is actively downloading.
    ///
    /// Currently unused by the engine — segments transition directly from
    /// `Pending` to `Completed`. Reserved for future per-segment progress
    /// tracking.
    Downloading,
    /// Segment finished successfully.
    Completed,
    /// Segment failed.
    ///
    /// Currently unused by the engine — failures are tracked at the download
    /// level. Reserved for future per-segment error handling.
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
    ///
    /// The default implementation is **not atomic** — it performs a
    /// load-then-delete-then-save sequence that has a TOCTOU window under
    /// concurrent access. Implementors **must** override this method with
    /// an atomic operation (e.g., SQL `INSERT ... ON CONFLICT DO UPDATE`).
    async fn upsert(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
        if self.load(download.id).await?.is_some() {
            self.delete(download.id).await?;
        }

        self.save(download).await
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use super::*;
    use crate::types::{DownloadPriority, ProxyConfig, ProxyKind};

    fn make_record_with_proxy(proxy_url: &str) -> DownloadRecord {
        DownloadRecord {
            id: DownloadId::new(),
            request: DownloadRequest {
                url: "https://example.com/file".to_string(),
                destination: PathBuf::from("/tmp/file"),
                priority: DownloadPriority::Normal,
                checksum: None,
                headers: HashMap::new(),
                max_connections: None,
                speed_limit: None,
                mirrors: Vec::new(),
                proxy: Some(ProxyConfig {
                    url: proxy_url.to_string(),
                    kind: ProxyKind::Http,
                }),
            },
            state: crate::DownloadState::Queued,
            etag: None,
            last_modified: None,
            content_length: None,
            bytes_downloaded: 0,
            last_error: None,
            created_at: 0,
            updated_at: 0,
            segments: Vec::new(),
        }
    }

    #[test]
    fn sanitize_strips_proxy_credentials() {
        let mut record = make_record_with_proxy("http://user:pass@proxy:8080");
        record.sanitize_for_persistence();
        let url = &record.request.proxy.as_ref().unwrap().url;
        assert!(!url.contains("user"), "username should be stripped: {url}");
        assert!(!url.contains("pass"), "password should be stripped: {url}");
        assert!(url.contains("proxy:8080"), "host:port should remain: {url}");
    }

    #[test]
    fn sanitize_preserves_proxy_without_credentials() {
        let mut record = make_record_with_proxy("http://proxy:8080");
        record.sanitize_for_persistence();
        let url = &record.request.proxy.as_ref().unwrap().url;
        assert!(url.contains("proxy:8080"), "host:port should remain: {url}");
    }
}
