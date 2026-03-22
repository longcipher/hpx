//! SQLite-backed storage implementation with crash recovery.

use std::path::Path;

use async_trait::async_trait;
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};

use super::{DownloadRecord, SegmentState, SegmentStatus, Storage};
use crate::{DownloadError, DownloadId, DownloadPriority, DownloadState};

/// SQLite-backed storage implementation.
///
/// Uses WAL journal mode for concurrent read/write access.
/// Loads in-progress downloads on startup for crash recovery.
#[derive(Debug)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Create a new SQLite storage at the given path.
    ///
    /// Creates the database file and runs migrations if needed.
    /// Enables WAL mode for crash recovery support.
    pub async fn new(path: &Path) -> Result<Self, DownloadError> {
        let db_url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        Self::migrate(&pool).await?;

        Ok(Self { pool })
    }

    /// Create a new SQLite storage from an existing pool.
    ///
    /// Useful for in-memory testing with `sqlite::memory:`.
    #[cfg(test)]
    async fn from_pool(pool: SqlitePool) -> Result<Self, DownloadError> {
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        Self::migrate(&pool).await?;

        Ok(Self { pool })
    }

    async fn migrate(pool: &SqlitePool) -> Result<(), DownloadError> {
        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS downloads (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                destination TEXT NOT NULL,
                state TEXT NOT NULL,
                priority TEXT NOT NULL,
                etag TEXT,
                last_modified TEXT,
                content_length INTEGER,
                bytes_downloaded INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            ",
        )
        .execute(pool)
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS segments (
                download_id TEXT NOT NULL,
                idx INTEGER NOT NULL,
                start INTEGER NOT NULL,
                end INTEGER NOT NULL,
                state TEXT NOT NULL,
                bytes_downloaded INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (download_id, idx),
                FOREIGN KEY (download_id) REFERENCES downloads(id)
            )
            ",
        )
        .execute(pool)
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))?;

        Ok(())
    }
}

fn parse_download_state(s: &str) -> Result<DownloadState, DownloadError> {
    match s {
        "queued" => Ok(DownloadState::Queued),
        "connecting" => Ok(DownloadState::Connecting),
        "downloading" => Ok(DownloadState::Downloading),
        "paused" => Ok(DownloadState::Paused),
        "completed" => Ok(DownloadState::Completed),
        "failed" => Ok(DownloadState::Failed),
        other => Err(DownloadError::Storage(format!(
            "unknown download state: {other}"
        ))),
    }
}

fn parse_priority(s: &str) -> Result<DownloadPriority, DownloadError> {
    match s {
        "low" => Ok(DownloadPriority::Low),
        "normal" => Ok(DownloadPriority::Normal),
        "high" => Ok(DownloadPriority::High),
        "critical" => Ok(DownloadPriority::Critical),
        other => Err(DownloadError::Storage(format!("unknown priority: {other}"))),
    }
}

fn parse_segment_status(s: &str) -> Result<SegmentStatus, DownloadError> {
    match s {
        "pending" => Ok(SegmentStatus::Pending),
        "downloading" => Ok(SegmentStatus::Downloading),
        "completed" => Ok(SegmentStatus::Completed),
        "failed" => Ok(SegmentStatus::Failed),
        other => Err(DownloadError::Storage(format!(
            "unknown segment status: {other}"
        ))),
    }
}

const fn priority_str(p: DownloadPriority) -> &'static str {
    match p {
        DownloadPriority::Low => "low",
        DownloadPriority::Normal => "normal",
        DownloadPriority::High => "high",
        DownloadPriority::Critical => "critical",
    }
}

const fn segment_status_str(s: SegmentStatus) -> &'static str {
    match s {
        SegmentStatus::Pending => "pending",
        SegmentStatus::Downloading => "downloading",
        SegmentStatus::Completed => "completed",
        SegmentStatus::Failed => "failed",
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn save(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        // Check for existing record
        let existing = sqlx::query("SELECT id FROM downloads WHERE id = ?")
            .bind(download.id.0.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        if existing.is_some() {
            return Err(DownloadError::AlreadyExists(download.id.0));
        }

        sqlx::query(
            "
            INSERT INTO downloads (id, url, destination, state, priority, etag, last_modified,
                content_length, bytes_downloaded, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(download.id.0.to_string())
        .bind(&download.url)
        .bind(download.destination.to_string_lossy().as_ref())
        .bind(download.state.to_string())
        .bind(priority_str(download.priority))
        .bind(&download.etag)
        .bind(&download.last_modified)
        .bind(download.content_length.map(|v| v as i64))
        .bind(download.bytes_downloaded as i64)
        .bind(download.created_at)
        .bind(download.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))?;

        for segment in &download.segments {
            sqlx::query(
                "
                INSERT INTO segments (download_id, idx, start, end, state, bytes_downloaded)
                VALUES (?, ?, ?, ?, ?, ?)
                ",
            )
            .bind(download.id.0.to_string())
            .bind(i64::from(segment.index))
            .bind(segment.start as i64)
            .bind(segment.end as i64)
            .bind(segment_status_str(segment.state))
            .bind(segment.bytes_downloaded as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        tracing::debug!(id = %download.id, "saved download record");
        Ok(())
    }

    async fn load(&self, id: DownloadId) -> Result<Option<DownloadRecord>, DownloadError> {
        let row = sqlx::query(
            "SELECT id, url, destination, state, priority, etag, last_modified, \
             content_length, bytes_downloaded, created_at, updated_at \
             FROM downloads WHERE id = ?",
        )
        .bind(id.0.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let state_str: String = row.get("state");
        let priority_str: String = row.get("priority");
        let segments = self.load_segments(id).await?;

        Ok(Some(DownloadRecord {
            id,
            url: row.get("url"),
            destination: std::path::PathBuf::from(row.get::<String, _>("destination")),
            state: parse_download_state(&state_str)?,
            priority: parse_priority(&priority_str)?,
            etag: row.get("etag"),
            last_modified: row.get("last_modified"),
            content_length: row
                .get::<Option<i64>, _>("content_length")
                .map(|v| v as u64),
            bytes_downloaded: row.get::<i64, _>("bytes_downloaded") as u64,
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            segments,
        }))
    }

    async fn list(&self) -> Result<Vec<DownloadRecord>, DownloadError> {
        let rows = sqlx::query("SELECT id FROM downloads ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let id_str: String = row.get("id");
            let uuid = uuid::Uuid::parse_str(&id_str)
                .map_err(|e| DownloadError::Storage(e.to_string()))?;
            let id = DownloadId(uuid);
            if let Some(record) = self.load(id).await? {
                records.push(record);
            }
        }

        Ok(records)
    }

    async fn delete(&self, id: DownloadId) -> Result<(), DownloadError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        sqlx::query("DELETE FROM segments WHERE download_id = ?")
            .bind(id.0.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        sqlx::query("DELETE FROM downloads WHERE id = ?")
            .bind(id.0.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        tracing::debug!(id = %id, "deleted download record");
        Ok(())
    }

    async fn update_progress(
        &self,
        id: DownloadId,
        segments: &[SegmentState],
    ) -> Result<(), DownloadError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        // Check record exists
        let existing = sqlx::query("SELECT id FROM downloads WHERE id = ?")
            .bind(id.0.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        if existing.is_none() {
            return Err(DownloadError::NotFound(id.0));
        }

        let bytes_downloaded: u64 = segments.iter().map(|s| s.bytes_downloaded).sum();

        // Update bytes_downloaded on the download
        sqlx::query("UPDATE downloads SET bytes_downloaded = ?, updated_at = ? WHERE id = ?")
            .bind(bytes_downloaded as i64)
            .bind(chrono_timestamp())
            .bind(id.0.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        // Delete old segments and insert new ones
        sqlx::query("DELETE FROM segments WHERE download_id = ?")
            .bind(id.0.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        for segment in segments {
            sqlx::query(
                "
                INSERT INTO segments (download_id, idx, start, end, state, bytes_downloaded)
                VALUES (?, ?, ?, ?, ?, ?)
                ",
            )
            .bind(id.0.to_string())
            .bind(i64::from(segment.index))
            .bind(segment.start as i64)
            .bind(segment.end as i64)
            .bind(segment_status_str(segment.state))
            .bind(segment.bytes_downloaded as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        tracing::debug!(id = %id, "updated progress");
        Ok(())
    }
}

impl SqliteStorage {
    async fn load_segments(&self, id: DownloadId) -> Result<Vec<SegmentState>, DownloadError> {
        let rows = sqlx::query(
            "SELECT idx, start, end, state, bytes_downloaded \
             FROM segments WHERE download_id = ? ORDER BY idx",
        )
        .bind(id.0.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))?;

        let mut segments = Vec::with_capacity(rows.len());
        for row in rows {
            let state_str: String = row.get("state");
            segments.push(SegmentState {
                index: row.get::<i64, _>("idx") as u32,
                start: row.get::<i64, _>("start") as u64,
                end: row.get::<i64, _>("end") as u64,
                state: parse_segment_status(&state_str)?,
                bytes_downloaded: row.get::<i64, _>("bytes_downloaded") as u64,
            });
        }

        Ok(segments)
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    async fn memory_storage() -> SqliteStorage {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect to in-memory sqlite");
        SqliteStorage::from_pool(pool)
            .await
            .expect("create storage from pool")
    }

    fn sample_record() -> DownloadRecord {
        let id = DownloadId::new();
        let now = 1_700_000_000;
        DownloadRecord {
            id,
            url: "https://example.com/file.bin".to_string(),
            destination: PathBuf::from("/tmp/file.bin"),
            state: DownloadState::Queued,
            priority: DownloadPriority::Normal,
            etag: Some("\"abc123\"".to_string()),
            last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string()),
            content_length: Some(10_000),
            bytes_downloaded: 0,
            created_at: now,
            updated_at: now,
            segments: Vec::new(),
        }
    }

    fn sample_record_with_segments() -> DownloadRecord {
        let id = DownloadId::new();
        let now = 1_700_000_000;
        DownloadRecord {
            id,
            url: "https://example.com/large.bin".to_string(),
            destination: PathBuf::from("/tmp/large.bin"),
            state: DownloadState::Downloading,
            priority: DownloadPriority::High,
            etag: Some("\"def456\"".to_string()),
            last_modified: None,
            content_length: Some(20_000),
            bytes_downloaded: 7_000,
            created_at: now,
            updated_at: now,
            segments: vec![
                SegmentState {
                    index: 0,
                    start: 0,
                    end: 10_000,
                    state: SegmentStatus::Completed,
                    bytes_downloaded: 10_000,
                },
                SegmentState {
                    index: 1,
                    start: 10_000,
                    end: 20_000,
                    state: SegmentStatus::Downloading,
                    bytes_downloaded: 2_000,
                },
            ],
        }
    }

    // --- Schema and WAL tests ---

    #[tokio::test]
    async fn schema_creates_tables() {
        let storage = memory_storage().await;

        // Verify downloads table exists by inserting and querying
        let record = sample_record();
        storage.save(&record).await.expect("save record");
        let loaded = storage.load(record.id).await.expect("load record");
        assert!(loaded.is_some());
    }

    #[tokio::test]
    async fn wal_mode_enabled() {
        let storage = memory_storage().await;

        let row = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&storage.pool)
            .await
            .expect("query pragma");

        let mode: String = row.get(0);
        // In-memory SQLite may not persist WAL but the pragma should succeed
        assert!(
            mode == "memory" || mode == "wal",
            "expected wal or memory, got {mode}"
        );
    }

    // --- save tests ---

    #[tokio::test]
    async fn save_and_load() {
        let storage = memory_storage().await;
        let record = sample_record();
        let id = record.id;

        storage.save(&record).await.expect("save");
        let loaded = storage
            .load(id)
            .await
            .expect("load")
            .expect("record exists");
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.url, record.url);
        assert_eq!(loaded.destination, record.destination);
        assert_eq!(loaded.state, DownloadState::Queued);
        assert_eq!(loaded.priority, DownloadPriority::Normal);
        assert_eq!(loaded.etag, record.etag);
        assert_eq!(loaded.last_modified, record.last_modified);
        assert_eq!(loaded.content_length, record.content_length);
        assert_eq!(loaded.bytes_downloaded, 0);
        assert_eq!(loaded.created_at, record.created_at);
        assert_eq!(loaded.updated_at, record.updated_at);
    }

    #[tokio::test]
    async fn save_with_segments() {
        let storage = memory_storage().await;
        let record = sample_record_with_segments();
        let id = record.id;

        storage.save(&record).await.expect("save");
        let loaded = storage
            .load(id)
            .await
            .expect("load")
            .expect("record exists");
        assert_eq!(loaded.segments.len(), 2);
        assert_eq!(loaded.segments[0].index, 0);
        assert_eq!(loaded.segments[0].start, 0);
        assert_eq!(loaded.segments[0].end, 10_000);
        assert_eq!(loaded.segments[0].state, SegmentStatus::Completed);
        assert_eq!(loaded.segments[0].bytes_downloaded, 10_000);
        assert_eq!(loaded.segments[1].index, 1);
        assert_eq!(loaded.segments[1].state, SegmentStatus::Downloading);
    }

    #[tokio::test]
    async fn save_duplicate_returns_already_exists() {
        let storage = memory_storage().await;
        let record = sample_record();

        storage.save(&record).await.expect("first save");
        let err = storage.save(&record).await.unwrap_err();
        assert!(matches!(err, DownloadError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn save_with_none_optional_fields() {
        let storage = memory_storage().await;
        let id = DownloadId::new();
        let record = DownloadRecord {
            id,
            url: "https://example.com/file".to_string(),
            destination: PathBuf::from("/tmp/file"),
            state: DownloadState::Queued,
            priority: DownloadPriority::Low,
            etag: None,
            last_modified: None,
            content_length: None,
            bytes_downloaded: 0,
            created_at: 1_000_000,
            updated_at: 1_000_000,
            segments: Vec::new(),
        };

        storage.save(&record).await.expect("save");
        let loaded = storage.load(id).await.expect("load").expect("exists");
        assert!(loaded.etag.is_none());
        assert!(loaded.last_modified.is_none());
        assert!(loaded.content_length.is_none());
    }

    // --- load tests ---

    #[tokio::test]
    async fn load_missing_returns_none() {
        let storage = memory_storage().await;
        let missing_id = DownloadId::new();
        let result = storage.load(missing_id).await.expect("load");
        assert!(result.is_none());
    }

    // --- list tests ---

    #[tokio::test]
    async fn list_returns_all_records() {
        let storage = memory_storage().await;

        let r1 = sample_record();
        let mut r2 = sample_record();
        r2.id = DownloadId::new();
        r2.url = "https://example.org/other.bin".to_string();

        storage.save(&r1).await.expect("save r1");
        storage.save(&r2).await.expect("save r2");

        let records = storage.list().await.expect("list");
        assert_eq!(records.len(), 2);
    }

    #[tokio::test]
    async fn list_empty_storage() {
        let storage = memory_storage().await;
        let records = storage.list().await.expect("list");
        assert!(records.is_empty());
    }

    // --- delete tests ---

    #[tokio::test]
    async fn delete_existing_record() {
        let storage = memory_storage().await;
        let record = sample_record();
        let id = record.id;

        storage.save(&record).await.expect("save");
        storage.delete(id).await.expect("delete");

        let loaded = storage.load(id).await.expect("load");
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn delete_removes_segments() {
        let storage = memory_storage().await;
        let record = sample_record_with_segments();
        let id = record.id;

        storage.save(&record).await.expect("save");
        storage.delete(id).await.expect("delete");

        let loaded = storage.load(id).await.expect("load");
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn delete_missing_is_ok() {
        let storage = memory_storage().await;
        let missing_id = DownloadId::new();
        storage.delete(missing_id).await.expect("delete missing");
    }

    // --- update_progress tests ---

    #[tokio::test]
    async fn update_progress_success() {
        let storage = memory_storage().await;
        let record = sample_record();
        let id = record.id;

        storage.save(&record).await.expect("save");

        let segments = vec![
            SegmentState {
                index: 0,
                start: 0,
                end: 5000,
                state: SegmentStatus::Completed,
                bytes_downloaded: 5000,
            },
            SegmentState {
                index: 1,
                start: 5000,
                end: 10_000,
                state: SegmentStatus::Downloading,
                bytes_downloaded: 2000,
            },
        ];

        storage
            .update_progress(id, &segments)
            .await
            .expect("update progress");

        let loaded = storage.load(id).await.expect("load").expect("exists");
        assert_eq!(loaded.segments.len(), 2);
        assert_eq!(loaded.bytes_downloaded, 7000);
    }

    #[tokio::test]
    async fn update_progress_not_found() {
        let storage = memory_storage().await;
        let missing_id = DownloadId::new();
        let err = storage.update_progress(missing_id, &[]).await.unwrap_err();
        assert!(matches!(err, DownloadError::NotFound(_)));
    }

    // --- Full lifecycle ---

    #[tokio::test]
    async fn full_lifecycle() {
        let storage = memory_storage().await;
        let record = sample_record();
        let id = record.id;

        // save
        storage.save(&record).await.expect("save");

        // update progress
        let segments = vec![SegmentState {
            index: 0,
            start: 0,
            end: 10_000,
            state: SegmentStatus::Completed,
            bytes_downloaded: 10_000,
        }];
        storage
            .update_progress(id, &segments)
            .await
            .expect("update progress");

        // verify progress
        let loaded = storage.load(id).await.expect("load").expect("exists");
        assert_eq!(loaded.bytes_downloaded, 10_000);
        assert_eq!(loaded.segments.len(), 1);

        // delete
        storage.delete(id).await.expect("delete");
        assert!(storage.load(id).await.expect("load").is_none());
    }

    // --- Priority and state variants ---

    #[tokio::test]
    async fn save_all_priorities() {
        let storage = memory_storage().await;
        for priority in [
            DownloadPriority::Low,
            DownloadPriority::Normal,
            DownloadPriority::High,
            DownloadPriority::Critical,
        ] {
            let mut record = sample_record();
            record.id = DownloadId::new();
            record.priority = priority;
            storage.save(&record).await.expect("save");
            let loaded = storage
                .load(record.id)
                .await
                .expect("load")
                .expect("exists");
            assert_eq!(loaded.priority, priority);
        }
    }

    #[tokio::test]
    async fn save_all_states() {
        let storage = memory_storage().await;
        for state in [
            DownloadState::Queued,
            DownloadState::Connecting,
            DownloadState::Downloading,
            DownloadState::Paused,
            DownloadState::Completed,
            DownloadState::Failed,
        ] {
            let mut record = sample_record();
            record.id = DownloadId::new();
            record.state = state;
            storage.save(&record).await.expect("save");
            let loaded = storage
                .load(record.id)
                .await
                .expect("load")
                .expect("exists");
            assert_eq!(loaded.state, state);
        }
    }

    #[tokio::test]
    async fn save_all_segment_statuses() {
        let storage = memory_storage().await;
        let mut record = sample_record();
        record.segments = vec![
            SegmentState {
                index: 0,
                start: 0,
                end: 100,
                state: SegmentStatus::Pending,
                bytes_downloaded: 0,
            },
            SegmentState {
                index: 1,
                start: 100,
                end: 200,
                state: SegmentStatus::Downloading,
                bytes_downloaded: 50,
            },
            SegmentState {
                index: 2,
                start: 200,
                end: 300,
                state: SegmentStatus::Completed,
                bytes_downloaded: 100,
            },
            SegmentState {
                index: 3,
                start: 300,
                end: 400,
                state: SegmentStatus::Failed,
                bytes_downloaded: 0,
            },
        ];
        storage.save(&record).await.expect("save");
        let loaded = storage
            .load(record.id)
            .await
            .expect("load")
            .expect("exists");
        assert_eq!(loaded.segments.len(), 4);
        assert_eq!(loaded.segments[0].state, SegmentStatus::Pending);
        assert_eq!(loaded.segments[1].state, SegmentStatus::Downloading);
        assert_eq!(loaded.segments[2].state, SegmentStatus::Completed);
        assert_eq!(loaded.segments[3].state, SegmentStatus::Failed);
    }

    // --- Crash recovery simulation ---

    #[tokio::test]
    async fn crash_recovery_with_temp_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("test.db");

        let id;
        {
            // Create storage, save a record, drop pool (simulate crash)
            let storage = SqliteStorage::new(&db_path).await.expect("new storage");
            let record = sample_record_with_segments();
            id = record.id;
            storage.save(&record).await.expect("save");
            // storage is dropped here, simulating crash
        }

        // Reopen storage at same path
        let storage = SqliteStorage::new(&db_path).await.expect("reopen storage");
        let loaded = storage
            .load(id)
            .await
            .expect("load")
            .expect("record persists");
        assert_eq!(loaded.url, "https://example.com/large.bin");
        assert_eq!(loaded.segments.len(), 2);
        assert_eq!(loaded.state, DownloadState::Downloading);
        assert_eq!(loaded.priority, DownloadPriority::High);
        assert_eq!(loaded.bytes_downloaded, 7_000);
    }

    #[tokio::test]
    async fn crash_recovery_multiple_records() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("test.db");

        let (id1, id2);
        {
            let storage = SqliteStorage::new(&db_path).await.expect("new storage");
            let r1 = sample_record();
            let mut r2 = sample_record();
            r2.id = DownloadId::new();
            id1 = r1.id;
            id2 = r2.id;
            storage.save(&r1).await.expect("save r1");
            storage.save(&r2).await.expect("save r2");
        }

        let storage = SqliteStorage::new(&db_path).await.expect("reopen");
        let records = storage.list().await.expect("list");
        assert_eq!(records.len(), 2);
        let ids: Vec<DownloadId> = records.iter().map(|r| r.id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }
}
