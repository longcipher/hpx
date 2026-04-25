//! SQLite-backed storage implementation with crash recovery.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};

use super::{DownloadRecord, SegmentState, SegmentStatus, Storage};
use crate::{DownloadError, DownloadId, DownloadPriority, DownloadRequest, DownloadState};

/// Safely convert `u64` to `i64` for SQLite binding.
///
/// Download sizes exceeding `i64::MAX` (~9.2 EB) are practically impossible,
/// so this returns an error rather than silently truncating.
fn to_i64(value: u64) -> Result<i64, DownloadError> {
    i64::try_from(value)
        .map_err(|_| DownloadError::Storage(format!("value {value} exceeds i64 range for SQLite")))
}

/// Safely convert `i64` from SQLite to `u64`.
fn to_u64(value: i64) -> u64 {
    // SQLite stores non-negative values for byte offsets/counters.
    // If negative, treat as 0 (defensive).
    u64::try_from(value).unwrap_or(0)
}

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
                request_json TEXT,
                state TEXT NOT NULL,
                priority TEXT NOT NULL,
                etag TEXT,
                last_modified TEXT,
                content_length INTEGER,
                bytes_downloaded INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            ",
        )
        .execute(pool)
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))?;

        Self::ensure_request_json_column(pool).await?;

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

    async fn ensure_request_json_column(pool: &SqlitePool) -> Result<(), DownloadError> {
        let rows = sqlx::query("PRAGMA table_info(downloads)")
            .fetch_all(pool)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        let has_request_json = rows
            .iter()
            .any(|row| row.get::<String, _>("name") == "request_json");
        if !has_request_json {
            sqlx::query("ALTER TABLE downloads ADD COLUMN request_json TEXT")
                .execute(pool)
                .await
                .map_err(|e| DownloadError::Storage(e.to_string()))?;
        }

        let has_last_error = rows
            .iter()
            .any(|row| row.get::<String, _>("name") == "last_error");
        if !has_last_error {
            sqlx::query("ALTER TABLE downloads ADD COLUMN last_error TEXT")
                .execute(pool)
                .await
                .map_err(|e| DownloadError::Storage(e.to_string()))?;
        }

        Ok(())
    }
}

fn serialize_request(request: &DownloadRequest) -> Result<String, DownloadError> {
    serde_json::to_string(request).map_err(|e| DownloadError::Storage(e.to_string()))
}

fn deserialize_request(
    request_json: Option<&str>,
    url: String,
    destination: PathBuf,
    priority: DownloadPriority,
) -> Result<DownloadRequest, DownloadError> {
    match request_json {
        Some(json) => serde_json::from_str(json).map_err(|e| DownloadError::Storage(e.to_string())),
        None => Ok(DownloadRequest {
            url,
            destination,
            priority,
            checksum: None,
            headers: HashMap::new(),
            max_connections: None,
            speed_limit: None,
            mirrors: Vec::new(),
            proxy: None,
        }),
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
        let request_json = serialize_request(&download.request)?;
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
            INSERT INTO downloads (id, url, destination, request_json, state, priority, etag,
                last_modified, content_length, bytes_downloaded, last_error, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(download.id.0.to_string())
        .bind(&download.request.url)
        .bind(download.request.destination.to_string_lossy().as_ref())
        .bind(&request_json)
        .bind(download.state.to_string())
        .bind(priority_str(download.request.priority))
        .bind(&download.etag)
        .bind(&download.last_modified)
        .bind(download.content_length.map(to_i64).transpose()?)
        .bind(to_i64(download.bytes_downloaded)?)
        .bind(&download.last_error)
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
            .bind(to_i64(segment.start)?)
            .bind(to_i64(segment.end)?)
            .bind(segment_status_str(segment.state))
            .bind(to_i64(segment.bytes_downloaded)?)
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
            "SELECT id, url, destination, request_json, state, priority, etag, last_modified, \
               content_length, bytes_downloaded, last_error, created_at, updated_at \
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
        let url: String = row.get("url");
        let destination = PathBuf::from(row.get::<String, _>("destination"));
        let priority = parse_priority(&priority_str)?;
        let request_json = row.get::<Option<String>, _>("request_json");
        let request = deserialize_request(request_json.as_deref(), url, destination, priority)?;
        let segments = self.load_segments(id).await?;

        Ok(Some(DownloadRecord {
            id,
            url: request.url.clone(),
            destination: request.destination.clone(),
            state: parse_download_state(&state_str)?,
            priority: request.priority,
            request,
            etag: row.get("etag"),
            last_modified: row.get("last_modified"),
            content_length: row.get::<Option<i64>, _>("content_length").map(to_u64),
            bytes_downloaded: to_u64(row.get::<i64, _>("bytes_downloaded")),
            last_error: row.get("last_error"),
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
            .bind(to_i64(bytes_downloaded)?)
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
            .bind(to_i64(segment.start)?)
            .bind(to_i64(segment.end)?)
            .bind(segment_status_str(segment.state))
            .bind(to_i64(segment.bytes_downloaded)?)
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

    async fn upsert(&self, download: &DownloadRecord) -> Result<(), DownloadError> {
        let request_json = serialize_request(&download.request)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        sqlx::query(
            "
            INSERT INTO downloads (id, url, destination, request_json, state, priority, etag,
                last_modified, content_length, bytes_downloaded, last_error, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                url = excluded.url,
                destination = excluded.destination,
                request_json = excluded.request_json,
                state = excluded.state,
                priority = excluded.priority,
                etag = excluded.etag,
                last_modified = excluded.last_modified,
                content_length = excluded.content_length,
                bytes_downloaded = excluded.bytes_downloaded,
                last_error = excluded.last_error,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at
            ",
        )
        .bind(download.id.0.to_string())
        .bind(&download.request.url)
        .bind(download.request.destination.to_string_lossy().as_ref())
        .bind(&request_json)
        .bind(download.state.to_string())
        .bind(priority_str(download.request.priority))
        .bind(&download.etag)
        .bind(&download.last_modified)
        .bind(download.content_length.map(to_i64).transpose()?)
        .bind(to_i64(download.bytes_downloaded)?)
        .bind(&download.last_error)
        .bind(download.created_at)
        .bind(download.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| DownloadError::Storage(e.to_string()))?;

        sqlx::query("DELETE FROM segments WHERE download_id = ?")
            .bind(download.id.0.to_string())
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
            .bind(to_i64(segment.start)?)
            .bind(to_i64(segment.end)?)
            .bind(segment_status_str(segment.state))
            .bind(to_i64(segment.bytes_downloaded)?)
            .execute(&mut *tx)
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DownloadError::Storage(e.to_string()))?;

        tracing::debug!(id = %download.id, "upserted download record");
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
                start: to_u64(row.get::<i64, _>("start")),
                end: to_u64(row.get::<i64, _>("end")),
                state: parse_segment_status(&state_str)?,
                bytes_downloaded: to_u64(row.get::<i64, _>("bytes_downloaded")),
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

    fn sample_record_with_segments() -> DownloadRecord {
        let id = DownloadId::new();
        let now = 1_700_000_000;
        let request = DownloadRequest::builder("https://example.com/large.bin", "/tmp/large.bin")
            .priority(DownloadPriority::High)
            .build();
        DownloadRecord {
            id,
            request: request.clone(),
            url: request.url.clone(),
            destination: request.destination.clone(),
            state: DownloadState::Downloading,
            priority: request.priority,
            etag: Some("\"def456\"".to_string()),
            last_modified: None,
            content_length: Some(20_000),
            bytes_downloaded: 7_000,
            last_error: None,
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
        assert_eq!(loaded.request.url, record.request.url);
        assert_eq!(loaded.request.destination, record.request.destination);
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
        let request = DownloadRequest::builder("https://example.com/file", "/tmp/file")
            .priority(DownloadPriority::Low)
            .build();
        let record = DownloadRecord {
            id,
            request: request.clone(),
            url: request.url.clone(),
            destination: request.destination.clone(),
            state: DownloadState::Queued,
            priority: request.priority,
            etag: None,
            last_modified: None,
            content_length: None,
            bytes_downloaded: 0,
            last_error: None,
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
        r2.request = DownloadRequest::builder("https://example.org/other.bin", "/tmp/file.bin")
            .priority(DownloadPriority::Normal)
            .build();
        r2.sync_request_fields();

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

    #[tokio::test]
    async fn upsert_replaces_existing_request_payload() {
        let storage = memory_storage().await;
        let mut record = sample_record();
        let id = record.id;

        storage.save(&record).await.expect("save");

        record.request =
            DownloadRequest::builder("https://example.org/replaced.bin", "/tmp/replaced.bin")
                .priority(DownloadPriority::Critical)
                .build();
        record.sync_request_fields();
        record.state = DownloadState::Paused;

        storage.upsert(&record).await.expect("upsert");

        let loaded = storage.load(id).await.expect("load").expect("exists");
        assert_eq!(loaded.request.url, "https://example.org/replaced.bin");
        assert_eq!(loaded.url, "https://example.org/replaced.bin");
        assert_eq!(loaded.destination, PathBuf::from("/tmp/replaced.bin"));
        assert_eq!(loaded.priority, DownloadPriority::Critical);
        assert_eq!(loaded.state, DownloadState::Paused);
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
            record.request.priority = priority;
            record.sync_request_fields();
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
