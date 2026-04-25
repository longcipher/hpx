//! In-memory storage backend for testing.

use std::sync::Arc;

use async_trait::async_trait;
use scc::HashMap;

use super::{DownloadRecord, SegmentState, Storage};
use crate::{DownloadError, DownloadId};

/// Thread-safe in-memory storage backed by [`scc::HashMap`].
///
/// Intended for unit tests and as a reference implementation.
#[derive(Debug)]
pub struct MemoryStorage {
    records: Arc<HashMap<DownloadId, DownloadRecord>>,
}

impl MemoryStorage {
    /// Create a new empty in-memory storage.
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Arc::new(HashMap::new()),
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn save(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
        let id = download.id;
        let mut record = download.clone();
        record.sync_request_fields();
        let records = Arc::clone(&self.records);
        let result = tokio::task::spawn_blocking(move || records.insert_sync(id, record))
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;
        match result {
            Ok(()) => {
                tracing::debug!(id = %id, "saved download record");
                Ok(())
            }
            Err(_) => Err(DownloadError::AlreadyExists(id.0)),
        }
    }

    async fn load(&self, id: DownloadId) -> Result<Option<DownloadRecord>, DownloadError> {
        let records = Arc::clone(&self.records);
        tokio::task::spawn_blocking(move || records.read_sync(&id, |_, record| record.clone()))
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))
    }

    async fn list(&self) -> Result<Vec<DownloadRecord>, DownloadError> {
        let records = Arc::clone(&self.records);
        tokio::task::spawn_blocking(move || {
            let len = records.len();
            let mut result = Vec::with_capacity(len);
            records.iter_sync(|_, record| {
                result.push(record.clone());
                true
            });
            result
        })
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))
    }

    async fn delete(&self, id: DownloadId) -> Result<(), DownloadError> {
        let records = Arc::clone(&self.records);
        let removed = tokio::task::spawn_blocking(move || records.remove_sync(&id))
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;
        if removed.is_some() {
            tracing::debug!(id = %id, "deleted download record");
        }
        Ok(())
    }

    async fn update_progress(
        &self,
        id: DownloadId,
        segments: &[SegmentState],
    ) -> Result<(), DownloadError> {
        let segments = segments.to_vec();
        let records = Arc::clone(&self.records);
        let updated = tokio::task::spawn_blocking(move || {
            records.update_sync(&id, |_, record| {
                record.segments = segments;
                record.bytes_downloaded = record.segments.iter().map(|s| s.bytes_downloaded).sum();
            })
        })
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))?;
        if updated.is_some() {
            tracing::debug!(id = %id, "updated progress");
            Ok(())
        } else {
            Err(DownloadError::NotFound(id.0))
        }
    }

    async fn upsert(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
        let id = download.id;
        let mut record = download.clone();
        record.sync_request_fields();
        let records = Arc::clone(&self.records);
        tokio::task::spawn_blocking(move || {
            records.remove_sync(&id);
            records
                .insert_sync(id, record)
                .map_err(|_| DownloadError::Storage("upsert failed".to_string()))
        })
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))??;

        tracing::debug!(id = %id, "upserted download record");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{DownloadPriority, DownloadRequest, DownloadState};

    fn sample_record() -> DownloadRecord {
        let id = DownloadId::new();
        let now = 1_700_000_000;
        let request = DownloadRequest::builder("https://example.com/file.bin", "/tmp/file.bin")
            .priority(DownloadPriority::Normal)
            .build();
        DownloadRecord {
            id,
            request: request.clone(),
            url: request.url.clone(),
            destination: request.destination.clone(),
            state: DownloadState::Queued,
            priority: request.priority,
            etag: Some("\"abc123\"".to_string()),
            last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string()),
            content_length: Some(10_000),
            bytes_downloaded: 0,
            last_error: None,
            created_at: now,
            updated_at: now,
            segments: Vec::new(),
        }
    }

    #[tokio::test]
    async fn save_and_load() {
        let storage = MemoryStorage::new();
        let record = sample_record();
        let id = record.id;

        storage.save(&record).await.unwrap();
        let loaded = storage.load(id).await.unwrap().unwrap();
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.url, record.url);
        assert_eq!(loaded.destination, record.destination);
        assert_eq!(loaded.request.url, record.request.url);
        assert_eq!(loaded.request.destination, record.request.destination);
    }

    #[tokio::test]
    async fn save_duplicate_returns_already_exists() {
        let storage = MemoryStorage::new();
        let record = sample_record();

        storage.save(&record).await.unwrap();
        let err = storage.save(&record).await.unwrap_err();
        assert!(matches!(err, DownloadError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn load_missing_returns_none() {
        let storage = MemoryStorage::new();
        let missing_id = DownloadId::new();
        let result = storage.load(missing_id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_returns_all_records() {
        let storage = MemoryStorage::new();

        let r1 = sample_record();
        let mut r2 = sample_record();
        r2.id = DownloadId::new();
        r2.request = DownloadRequest::builder("https://example.org/other.bin", "/tmp/file.bin")
            .priority(DownloadPriority::Normal)
            .build();
        r2.sync_request_fields();

        storage.save(&r1).await.unwrap();
        storage.save(&r2).await.unwrap();

        let records = storage.list().await.unwrap();
        assert_eq!(records.len(), 2);
    }

    #[tokio::test]
    async fn list_empty_storage() {
        let storage = MemoryStorage::new();
        let records = storage.list().await.unwrap();
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn delete_existing_record() {
        let storage = MemoryStorage::new();
        let record = sample_record();
        let id = record.id;

        storage.save(&record).await.unwrap();
        storage.delete(id).await.unwrap();

        let loaded = storage.load(id).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn delete_missing_is_ok() {
        let storage = MemoryStorage::new();
        let missing_id = DownloadId::new();
        storage.delete(missing_id).await.unwrap();
    }

    #[tokio::test]
    async fn update_progress_success() {
        let storage = MemoryStorage::new();
        let record = sample_record();
        let id = record.id;

        storage.save(&record).await.unwrap();

        let segments = vec![
            SegmentState {
                index: 0,
                start: 0,
                end: 5000,
                state: super::super::SegmentStatus::Completed,
                bytes_downloaded: 5000,
            },
            SegmentState {
                index: 1,
                start: 5000,
                end: 10_000,
                state: super::super::SegmentStatus::Downloading,
                bytes_downloaded: 2000,
            },
        ];

        storage.update_progress(id, &segments).await.unwrap();

        let loaded = storage.load(id).await.unwrap().unwrap();
        assert_eq!(loaded.segments.len(), 2);
        assert_eq!(loaded.bytes_downloaded, 7000);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_record() {
        let storage = MemoryStorage::new();
        let mut record = sample_record();
        let id = record.id;

        storage.save(&record).await.unwrap();

        record.request =
            DownloadRequest::builder("https://example.org/other.bin", "/tmp/other.bin")
                .priority(DownloadPriority::High)
                .build();
        record.sync_request_fields();
        record.state = DownloadState::Paused;

        storage.upsert(&record).await.unwrap();

        let loaded = storage.load(id).await.unwrap().unwrap();
        assert_eq!(loaded.request.url, "https://example.org/other.bin");
        assert_eq!(loaded.destination, PathBuf::from("/tmp/other.bin"));
        assert_eq!(loaded.state, DownloadState::Paused);
        assert_eq!(loaded.priority, DownloadPriority::High);
    }

    #[tokio::test]
    async fn update_progress_not_found() {
        let storage = MemoryStorage::new();
        let missing_id = DownloadId::new();
        let err = storage.update_progress(missing_id, &[]).await.unwrap_err();
        assert!(matches!(err, DownloadError::NotFound(_)));
    }

    #[tokio::test]
    async fn full_lifecycle() {
        let storage = MemoryStorage::new();
        let mut record = sample_record();
        record.state = DownloadState::Queued;
        let id = record.id;

        // save
        storage.save(&record).await.unwrap();

        // update progress
        let segments = vec![SegmentState {
            index: 0,
            start: 0,
            end: 10_000,
            state: super::super::SegmentStatus::Completed,
            bytes_downloaded: 10_000,
        }];
        storage.update_progress(id, &segments).await.unwrap();

        // verify progress
        let loaded = storage.load(id).await.unwrap().unwrap();
        assert_eq!(loaded.bytes_downloaded, 10_000);

        // delete
        storage.delete(id).await.unwrap();
        assert!(storage.load(id).await.unwrap().is_none());
    }
}
