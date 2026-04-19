//! Error types for the download engine.

/// Errors that can occur during download operations.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// HTTP-level error from the underlying client.
    #[cfg(feature = "http")]
    #[error("HTTP error: {0}")]
    Http(#[from] hpx::Error),

    /// I/O error during file operations.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Storage backend error.
    #[error("Storage error: {0}")]
    Storage(String),

    /// Checksum verification failed.
    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Expected checksum value.
        expected: String,
        /// Actual checksum value.
        actual: String,
    },

    /// Download with the given ID was not found.
    #[error("Download not found: {0}")]
    NotFound(uuid::Uuid),

    /// Server does not support byte-range requests.
    #[error("Server does not support range requests")]
    NoRangeSupport,

    /// Metalink document parse error.
    #[error("Metalink parse error: {0}")]
    MetalinkParse(String),

    /// Rate limit has been exceeded.
    #[error("Rate limit exceeded")]
    RateLimited,

    /// A download with this ID already exists.
    #[error("Download already exists: {0}")]
    AlreadyExists(uuid::Uuid),

    /// Download is not in the expected state.
    #[error("Download is not in {expected} state, current state: {actual}")]
    InvalidState {
        /// Expected state.
        expected: String,
        /// Actual current state.
        actual: String,
    },

    /// Download or engine configuration is invalid.
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    /// A segment download task panicked.
    #[error("Segment task join error: {0}")]
    TaskJoin(String),

    /// A segment failed after exhausting all retry attempts.
    #[error("Segment [{start}-{end}] failed after {attempts} attempts: {source}")]
    SegmentRetryExhausted {
        /// Start byte of the failed segment.
        start: u64,
        /// End byte of the failed segment.
        end: u64,
        /// Number of attempts made.
        attempts: u32,
        /// The underlying error from the last attempt.
        source: Box<Self>,
    },

    /// No segments to download.
    #[error("No segments provided")]
    NoSegments,
}
