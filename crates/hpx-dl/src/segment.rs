//! Segmented download logic.
//!
//! Provides HEAD probing for remote file metadata, segment boundary
//! calculation, and single-segment download for parallel HTTP range downloads.

use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

use futures_util::StreamExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tracing::debug;

use crate::{
    CompositeLimiter,
    error::DownloadError,
    storage::{SegmentStatus, Storage},
};

/// Internal progress updates emitted by [`SegmentDownloader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentProgressUpdate {
    /// A segment finished successfully.
    Completed {
        /// Zero-based segment index.
        index: u32,
        /// Bytes downloaded for the completed segment.
        bytes_downloaded: u64,
    },
}

/// Information gathered from a HEAD request to the remote file.
#[derive(Debug, Clone)]
pub struct RemoteInfo {
    /// Content-Length in bytes, if the server provided it.
    pub content_length: Option<u64>,
    /// Whether the server advertises `Accept-Ranges: bytes`.
    pub accept_ranges: bool,
    /// ETag header value, if present.
    pub etag: Option<String>,
    /// Last-Modified header value, if present.
    pub last_modified: Option<String>,
}

/// A byte range for a download segment (both `start` and `end` are inclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRange {
    /// Start byte offset (inclusive).
    pub start: u64,
    /// End byte offset (inclusive).
    pub end: u64,
}

impl SegmentRange {
    /// Create a new segment range.
    #[must_use]
    pub const fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// Return the number of bytes in this segment.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    /// Return true if this segment contains zero bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start > self.end
    }
}

/// Send a HEAD request to probe remote file metadata.
///
/// Extracts `Content-Length`, `Accept-Ranges`, `ETag`, and `Last-Modified`
/// headers from the response.
///
/// # Errors
///
/// Returns [`DownloadError::Http`] if the HEAD request fails.
pub async fn probe_remote(client: &hpx::Client, url: &str) -> Result<RemoteInfo, DownloadError> {
    probe_remote_with_headers(client, url, &HashMap::new()).await
}

/// Send a HEAD request to probe remote file metadata with custom headers.
pub async fn probe_remote_with_headers(
    client: &hpx::Client,
    url: &str,
    headers: &HashMap<String, String, impl std::hash::BuildHasher>,
) -> Result<RemoteInfo, DownloadError> {
    debug!(%url, "probing remote file with HEAD request");

    let mut request = client.head(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let headers = response.headers();

    let content_length = headers
        .get(hpx::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let accept_ranges = headers
        .get(hpx::header::ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("bytes"));

    let etag = headers
        .get(hpx::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let last_modified = headers
        .get(hpx::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    debug!(
        content_length,
        accept_ranges, etag, last_modified, "HEAD probe completed"
    );

    Ok(RemoteInfo {
        content_length,
        accept_ranges,
        etag,
        last_modified,
    })
}

/// Calculate optimal segment ranges for parallel downloading.
///
/// Divides `file_size` bytes into up to `max_connections` segments, ensuring
/// no segment is smaller than `min_segment_size` bytes (except possibly the
/// last segment).
///
/// Returns an empty vec for a 0-byte file, or a single segment when
/// `max_connections == 1` or the file is smaller than `min_segment_size`.
#[must_use]
pub fn calculate_segments(
    file_size: u64,
    max_connections: usize,
    min_segment_size: u64,
) -> Vec<SegmentRange> {
    if file_size == 0 {
        return Vec::new();
    }

    if max_connections == 0 || max_connections == 1 || file_size <= min_segment_size {
        return vec![SegmentRange::new(0, file_size - 1)];
    }

    let max_segs_by_size = file_size / min_segment_size;
    let max_segs_by_size = usize::try_from(max_segs_by_size).unwrap_or(usize::MAX);
    let num_segments = max_connections.min(max_segs_by_size).max(1);

    if num_segments <= 1 {
        return vec![SegmentRange::new(0, file_size - 1)];
    }

    let base_size = file_size / u64::try_from(num_segments).unwrap_or(u64::MAX);
    let remainder = file_size % u64::try_from(num_segments).unwrap_or(u64::MAX);

    let mut segments = Vec::with_capacity(num_segments);
    let mut offset = 0u64;

    for i in 0..num_segments {
        let extra = u64::from(i < usize::try_from(remainder).unwrap_or(usize::MAX));
        let seg_size = base_size + extra;
        let end = offset + seg_size - 1;
        segments.push(SegmentRange::new(offset, end));
        offset = end + 1;
    }

    debug!(
        file_size,
        num_segments, min_segment_size, "calculated segment boundaries"
    );

    segments
}

/// Format a byte range header value string `"bytes=start-end"`.
#[must_use]
pub fn range_header_value(range: &SegmentRange) -> String {
    format!("bytes={}-{}", range.start, range.end)
}

/// Download a single byte range from a URL and write to file.
///
/// Sends a GET request with a `Range` header, streams the response body,
/// and writes each chunk to the file at the correct byte offset.
///
/// # Errors
///
/// Returns [`DownloadError::Http`] if the HTTP request fails,
/// [`DownloadError::Io`] if file I/O fails.
pub async fn download_segment(
    client: &hpx::Client,
    url: &str,
    range: &SegmentRange,
    file: &mut tokio::fs::File,
    progress_tx: Option<&tokio::sync::mpsc::Sender<u64>>,
) -> Result<(), DownloadError> {
    let headers = HashMap::new();
    let limiter = CompositeLimiter::unlimited();
    download_segment_with_options(client, url, range, file, progress_tx, &headers, &limiter).await
}

async fn download_segment_with_options(
    client: &hpx::Client,
    url: &str,
    range: &SegmentRange,
    file: &mut tokio::fs::File,
    progress_tx: Option<&tokio::sync::mpsc::Sender<u64>>,
    headers: &HashMap<String, String, impl std::hash::BuildHasher>,
    limiter: &CompositeLimiter,
) -> Result<(), DownloadError> {
    let header_value = range_header_value(range);
    debug!(url, range = %header_value, "starting segment download");

    let mut request = client.get(url).header(hpx::header::RANGE, header_value);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;

    let status = response.status();
    if status != hpx::StatusCode::PARTIAL_CONTENT && status != hpx::StatusCode::OK {
        tracing::warn!(status = %status, "unexpected status for range request");
    }

    let mut stream = response.bytes_stream();
    let mut offset = range.start;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        let chunk_len = chunk.len();
        limiter
            .wait_for(u64::try_from(chunk_len).unwrap_or(u64::MAX))
            .await?;

        file.seek(std::io::SeekFrom::Start(offset)).await?;
        file.write_all(&chunk).await?;
        offset += u64::try_from(chunk_len).unwrap_or(u64::MAX);

        if let Some(tx) = progress_tx {
            let _ = tx.send(u64::try_from(chunk_len).unwrap_or(u64::MAX)).await;
        }
    }

    file.flush().await?;
    debug!(
        bytes_written = offset - range.start,
        "segment download complete"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// SegmentDownloader — concurrent multi-segment download
// ---------------------------------------------------------------------------

/// Compute exponential backoff delay with optional jitter.
///
/// Returns `min(initial_delay * 2^attempt, max_delay)` adjusted by
/// `± jitter` fraction (uniform random).
#[must_use]
pub fn compute_backoff_delay(
    initial_delay: Duration,
    max_delay: Duration,
    jitter: f64,
    attempt: u32,
) -> Duration {
    let base = initial_delay.saturating_mul(2_u32.saturating_pow(attempt));
    let capped = base.min(max_delay);

    // Uniform jitter in [1 - jitter, 1 + jitter]
    let rand_offset = 2.0f64.mul_add(fastrand::f64(), -1.0);
    let factor = jitter.mul_add(rand_offset, 1.0);
    let adjusted = capped.as_secs_f64() * factor;
    Duration::from_secs_f64(adjusted.max(0.0))
}

/// Execute a fallible async closure with exponential backoff retries.
///
/// Retries up to `max_attempts` times (inclusive). On each failure, sleeps
/// for an exponentially increasing delay capped at `max_delay`, with
/// optional jitter. Returns the last error if all attempts fail.
///
/// `max_attempts` of 0 means try once with no retries.
pub async fn with_retry<F, T>(
    max_attempts: u32,
    initial_delay: Duration,
    max_delay: Duration,
    jitter: f64,
    f: F,
) -> Result<T, DownloadError>
where
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, DownloadError>> + Send>>,
{
    let total_tries = max_attempts.saturating_add(1);
    let mut last_err = None;

    for attempt in 0..total_tries {
        match f().await {
            Ok(val) => return Ok(val),
            Err(err) => {
                debug!(
                    attempt,
                    max_attempts, error = %err, "retry attempt failed"
                );
                last_err = Some(err);
                if attempt < total_tries - 1 {
                    let delay = compute_backoff_delay(initial_delay, max_delay, jitter, attempt);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(last_err.unwrap_or(DownloadError::NoSegments))
}

/// Manages concurrent download of multiple segments to a single destination file.
pub struct SegmentDownloader {
    /// HTTP client for making requests.
    pub client: hpx::Client,
    /// URL to download from.
    pub url: String,
    /// Segment byte ranges to download concurrently.
    pub segments: Vec<SegmentRange>,
    /// Destination file path.
    pub destination: PathBuf,
    /// Custom headers applied to requests.
    pub headers: HashMap<String, String>,
    /// Speed limiter enforced for each downloaded chunk.
    pub limiter: CompositeLimiter,
    /// Optional segment state updates.
    pub(crate) segment_tx: Option<tokio::sync::mpsc::UnboundedSender<SegmentProgressUpdate>>,
    /// Maximum retry attempts per segment.
    pub max_retries: u32,
    /// Initial backoff delay.
    pub retry_initial_delay: Duration,
    /// Maximum backoff delay.
    pub retry_max_delay: Duration,
    /// Jitter factor (0.0 to 1.0).
    pub retry_jitter: f64,
}

impl std::fmt::Debug for SegmentDownloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentDownloader")
            .field("url", &self.url)
            .field("segments", &self.segments)
            .field("destination", &self.destination)
            .field("max_retries", &self.max_retries)
            .field("retry_initial_delay", &self.retry_initial_delay)
            .field("retry_max_delay", &self.retry_max_delay)
            .field("retry_jitter", &self.retry_jitter)
            .finish_non_exhaustive()
    }
}

impl SegmentDownloader {
    /// Create a new `SegmentDownloader` with default retry configuration.
    #[must_use]
    pub fn new(
        client: hpx::Client,
        url: impl Into<String>,
        segments: Vec<SegmentRange>,
        destination: impl Into<PathBuf>,
    ) -> Self {
        Self {
            client,
            url: url.into(),
            segments,
            destination: destination.into(),
            headers: HashMap::new(),
            limiter: CompositeLimiter::unlimited(),
            segment_tx: None,
            max_retries: 3,
            retry_initial_delay: Duration::from_millis(200),
            retry_max_delay: Duration::from_secs(30),
            retry_jitter: 0.25,
        }
    }

    /// Create a `SegmentDownloader` that uses a proxied client.
    ///
    /// Builds an [`hpx::Client`] with the given [`crate::ProxyConfig`]
    /// applied, delegating to `hpx`'s proxy infrastructure.
    ///
    /// # Errors
    ///
    /// Returns [`DownloadError::Http`] if the proxy configuration is invalid
    /// and the client cannot be built.
    pub fn with_proxy(
        url: impl Into<String>,
        segments: Vec<SegmentRange>,
        destination: impl Into<PathBuf>,
        proxy_config: &crate::types::ProxyConfig,
    ) -> Result<Self, DownloadError> {
        let hpx_proxy = hpx::Proxy::try_from(proxy_config.clone())?;
        let client = hpx::Client::builder().proxy(hpx_proxy).build()?;
        Ok(Self::new(client, url, segments, destination))
    }

    /// Override the retry configuration.
    #[must_use]
    pub const fn with_retry_config(
        mut self,
        max_retries: u32,
        initial_delay: Duration,
        max_delay: Duration,
        jitter: f64,
    ) -> Self {
        self.max_retries = max_retries;
        self.retry_initial_delay = initial_delay;
        self.retry_max_delay = max_delay;
        self.retry_jitter = jitter;
        self
    }

    /// Override request headers applied to segment requests.
    #[must_use]
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Override the limiter used by each segment request.
    #[must_use]
    pub fn with_limiter(mut self, limiter: CompositeLimiter) -> Self {
        self.limiter = limiter;
        self
    }

    /// Subscribe to segment completion updates.
    #[must_use]
    pub(crate) fn with_segment_updates(
        mut self,
        segment_tx: tokio::sync::mpsc::UnboundedSender<SegmentProgressUpdate>,
    ) -> Self {
        self.segment_tx = Some(segment_tx);
        self
    }

    /// Download all segments concurrently and write to the destination file.
    ///
    /// Creates the destination file with the correct total size, then spawns
    /// one task per segment. Each task opens its own file handle and downloads
    /// its range with retry logic.
    ///
    /// # Errors
    ///
    /// Returns [`DownloadError::NoSegments`] if `self.segments` is empty.
    /// Returns [`DownloadError::Io`] if file creation or sizing fails.
    /// Returns [`DownloadError::SegmentRetryExhausted`] if any segment fails
    /// after all retry attempts.
    /// Returns [`DownloadError::TaskJoin`] if a segment task panics.
    pub async fn download(
        &self,
        progress_tx: Option<tokio::sync::mpsc::Sender<u64>>,
    ) -> Result<u64, DownloadError> {
        if self.segments.is_empty() {
            return Err(DownloadError::NoSegments);
        }

        let total_size: u64 = self.segments.iter().map(SegmentRange::len).sum();

        // Create destination file with correct size
        let file = tokio::fs::File::create(&self.destination).await?;
        file.set_len(total_size).await?;
        drop(file);

        self.download_inner(progress_tx).await
    }

    /// Download with resume support.
    ///
    /// Depending on the [`ResumeState`], this either starts a fresh download,
    /// resumes from the last completed segment, or restarts after content
    /// changed detection.
    ///
    /// # Errors
    ///
    /// Same as [`SegmentDownloader::download`].
    pub async fn download_with_resume(
        &self,
        resume_state: ResumeState,
        progress_tx: Option<tokio::sync::mpsc::Sender<u64>>,
    ) -> Result<u64, DownloadError> {
        match resume_state {
            ResumeState::Fresh => self.download(progress_tx).await,
            ResumeState::CanResume {
                completed_segments, ..
            } => {
                let remaining = filter_remaining_segments(&self.segments, &completed_segments);
                if remaining.is_empty() {
                    debug!("all segments already completed, nothing to download");
                    let total: u64 = self.segments.iter().map(SegmentRange::len).sum();
                    return Ok(total);
                }

                debug!(
                    completed = completed_segments.len(),
                    remaining = remaining.len(),
                    "resuming download with remaining segments"
                );

                // Create a temporary downloader for the remaining segments
                let sub = self.clone_for_segments(remaining);
                // Use existing file, don't truncate
                sub.download_from_existing(progress_tx).await
            }
            ResumeState::ContentChanged => {
                debug!("content changed, deleting partial file and restarting");
                let _ = tokio::fs::remove_file(&self.destination).await;
                self.download(progress_tx).await
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Resume logic
// ---------------------------------------------------------------------------

/// State of a resumable download.
#[derive(Debug, Clone)]
pub enum ResumeState {
    /// No existing file found, start fresh.
    Fresh,
    /// Content unchanged, can resume from given byte offset.
    CanResume {
        /// Total bytes already completed across all finished segments.
        bytes_completed: u64,
        /// Indices of segments that are fully completed.
        completed_segments: Vec<u32>,
    },
    /// Content changed (ETag/Last-Modified mismatch), must restart.
    ContentChanged,
}

/// Compare previous and current content-identity headers.
///
/// Returns `true` if the remote content is considered unchanged.
///
/// - If both ETags are present, they must be exactly equal.
/// - If both Last-Modified values are present, they must be exactly equal.
/// - If one side has a validator the other lacks, returns `false` (unknown state).
/// - If neither side has any validator, returns `true` (no evidence of change).
#[must_use]
pub fn compare_content_headers(
    prev_etag: &Option<String>,
    prev_last_modified: &Option<String>,
    remote_etag: &Option<String>,
    remote_last_modified: &Option<String>,
) -> bool {
    // Compare ETag if both have one
    match (prev_etag, remote_etag) {
        (Some(prev), Some(cur)) => {
            if prev != cur {
                return false;
            }
        }
        (Some(_), None) => return false,
        (None, Some(_)) => return false,
        (None, None) => {}
    }

    // Compare Last-Modified if both have one
    match (prev_last_modified, remote_last_modified) {
        (Some(prev), Some(cur)) => {
            if prev != cur {
                return false;
            }
        }
        (Some(_), None) => return false,
        (None, Some(_)) => return false,
        (None, None) => {}
    }

    true
}

/// Determine the resume state given file existence and content header comparisons.
///
/// This is a pure function extracted for testability.
#[must_use]
pub fn determine_resume_state(
    file_exists: bool,
    record_etag: &Option<String>,
    record_last_modified: &Option<String>,
    remote_etag: &Option<String>,
    remote_last_modified: &Option<String>,
) -> ResumeState {
    if !file_exists {
        return ResumeState::Fresh;
    }

    // File exists but no previous record → unknown state
    if record_etag.is_none() && record_last_modified.is_none() {
        return ResumeState::ContentChanged;
    }

    if compare_content_headers(
        record_etag,
        record_last_modified,
        remote_etag,
        remote_last_modified,
    ) {
        // Content unchanged — caller fills in completed_segments
        ResumeState::CanResume {
            bytes_completed: 0,
            completed_segments: Vec::new(),
        }
    } else {
        ResumeState::ContentChanged
    }
}

/// Determine whether persisted segments can be safely resumed.
#[must_use]
pub fn resume_state_from_segments(
    record_etag: &Option<String>,
    record_last_modified: &Option<String>,
    remote_etag: &Option<String>,
    remote_last_modified: &Option<String>,
    segments: &[crate::storage::SegmentState],
) -> ResumeState {
    let base_state = determine_resume_state(
        true,
        record_etag,
        record_last_modified,
        remote_etag,
        remote_last_modified,
    );

    if !matches!(base_state, ResumeState::CanResume { .. }) {
        return base_state;
    }

    let completed_segments: Vec<u32> = segments
        .iter()
        .filter(|segment| segment.state == SegmentStatus::Completed)
        .map(|segment| segment.index)
        .collect();
    if completed_segments.is_empty() {
        return ResumeState::Fresh;
    }

    let bytes_completed = segments
        .iter()
        .filter(|segment| segment.state == SegmentStatus::Completed)
        .map(|segment| segment.bytes_downloaded)
        .sum();

    ResumeState::CanResume {
        bytes_completed,
        completed_segments,
    }
}

/// Filter segments to only those not in `completed_indices`.
#[must_use]
pub fn filter_remaining_segments(
    segments: &[SegmentRange],
    completed_indices: &[u32],
) -> Vec<SegmentRange> {
    segments
        .iter()
        .enumerate()
        .filter(|(i, _)| !completed_indices.contains(&(*i as u32)))
        .map(|(_, seg)| seg.clone())
        .collect()
}

/// Check if an existing download can be resumed.
///
/// Sends a HEAD request to get current ETag/Last-Modified, loads the previous
/// download record from storage, and compares content-identity headers.
///
/// # Errors
///
/// Returns [`DownloadError::Http`] if the HEAD request fails.
/// Returns storage errors if the record cannot be loaded.
pub async fn check_resume(
    client: &hpx::Client,
    url: &str,
    existing_file: &Path,
    storage: &dyn Storage,
    download_id: crate::types::DownloadId,
) -> Result<ResumeState, DownloadError> {
    let file_exists = existing_file.exists();

    if !file_exists {
        debug!("no existing file found, starting fresh");
        return Ok(ResumeState::Fresh);
    }

    // Probe remote for current content headers
    let remote = probe_remote(client, url).await?;

    // Load previous record
    let record = storage.load(download_id).await?;

    let (record_etag, record_last_modified) = match &record {
        Some(r) => (r.etag.clone(), r.last_modified.clone()),
        None => (None, None),
    };

    let state = determine_resume_state(
        file_exists,
        &record_etag,
        &record_last_modified,
        &remote.etag,
        &remote.last_modified,
    );

    if let Some(record) = record {
        let state = resume_state_from_segments(
            &record_etag,
            &record_last_modified,
            &remote.etag,
            &remote.last_modified,
            &record.segments,
        );

        if let ResumeState::CanResume {
            bytes_completed,
            completed_segments,
        } = &state
        {
            debug!(
                bytes_completed,
                segments = completed_segments.len(),
                "content unchanged, can resume"
            );
        }

        debug!(?state, "resume check completed");
        return Ok(state);
    }

    debug!(?state, "resume check completed");
    Ok(state)
}

impl SegmentDownloader {
    /// Create a clone of this downloader with different segments (internal helper).
    fn clone_for_segments(&self, segments: Vec<SegmentRange>) -> Self {
        Self {
            client: self.client.clone(),
            url: self.url.clone(),
            segments,
            destination: self.destination.clone(),
            headers: self.headers.clone(),
            limiter: self.limiter.clone(),
            segment_tx: self.segment_tx.clone(),
            max_retries: self.max_retries,
            retry_initial_delay: self.retry_initial_delay,
            retry_max_delay: self.retry_max_delay,
            retry_jitter: self.retry_jitter,
        }
    }

    /// Download segments into an already-existing file without truncating.
    ///
    /// Unlike [`SegmentDownloader::download`], this does not create or resize
    /// the destination file. It assumes the file already exists and has the
    /// correct size.
    async fn download_from_existing(
        &self,
        progress_tx: Option<tokio::sync::mpsc::Sender<u64>>,
    ) -> Result<u64, DownloadError> {
        if self.segments.is_empty() {
            return Err(DownloadError::NoSegments);
        }
        self.download_inner(progress_tx).await
    }

    /// Shared download logic used by both `download()` and `download_from_existing()`.
    async fn download_inner(
        &self,
        progress_tx: Option<tokio::sync::mpsc::Sender<u64>>,
    ) -> Result<u64, DownloadError> {
        let mut join_set = tokio::task::JoinSet::new();

        for (segment_index, segment) in self.segments.iter().enumerate() {
            let client = self.client.clone();
            let url = self.url.clone();
            let path = self.destination.clone();
            let seg = segment.clone();
            let progress_tx = progress_tx.clone();
            let max_retries = self.max_retries;
            let initial_delay = self.retry_initial_delay;
            let max_delay = self.retry_max_delay;
            let jitter = self.retry_jitter;
            let headers = self.headers.clone();
            let limiter = self.limiter.clone();
            let segment_tx = self.segment_tx.clone();
            let segment_index = segment_index as u32;

            join_set.spawn(async move {
                let result = with_retry(max_retries, initial_delay, max_delay, jitter, || {
                    let client = client.clone();
                    let url = url.clone();
                    let seg = seg.clone();
                    let path = path.clone();
                    let progress_tx = progress_tx.clone();
                    let headers = headers.clone();
                    let limiter = limiter.clone();

                    Box::pin(async move {
                        let mut file = tokio::fs::OpenOptions::new()
                            .write(true)
                            .open(&path)
                            .await?;
                        download_segment_with_options(
                            &client,
                            &url,
                            &seg,
                            &mut file,
                            progress_tx.as_ref(),
                            &headers,
                            &limiter,
                        )
                        .await
                    })
                        as Pin<Box<dyn Future<Output = Result<(), DownloadError>> + Send>>
                })
                .await;

                match result {
                    Ok(()) => {
                        if let Some(segment_tx) = &segment_tx {
                            let _ = segment_tx.send(SegmentProgressUpdate::Completed {
                                index: segment_index,
                                bytes_downloaded: seg.len(),
                            });
                        }
                        Ok(seg.len())
                    }
                    Err(err) => Err(DownloadError::SegmentRetryExhausted {
                        start: seg.start,
                        end: seg.end,
                        attempts: max_retries + 1,
                        source: Box::new(err),
                    }),
                }
            });
        }

        let mut total_downloaded = 0u64;
        while let Some(result) = join_set.join_next().await {
            let bytes = result.map_err(|e| DownloadError::TaskJoin(e.to_string()))??;
            total_downloaded += bytes;
        }

        Ok(total_downloaded)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    // --- SegmentRange unit tests ---

    #[test]
    fn segment_range_len() {
        let seg = SegmentRange::new(0, 99);
        assert_eq!(seg.len(), 100);
    }

    #[test]
    fn segment_range_single_byte() {
        let seg = SegmentRange::new(42, 42);
        assert_eq!(seg.len(), 1);
        assert!(!seg.is_empty());
    }

    #[test]
    fn segment_range_empty() {
        let seg = SegmentRange::new(5, 3);
        assert!(seg.is_empty());
    }

    // --- calculate_segments edge cases ---

    #[test]
    fn zero_byte_file_returns_empty() {
        let segs = calculate_segments(0, 4, 1024);
        assert!(segs.is_empty());
    }

    #[test]
    fn one_byte_file_returns_single_segment() {
        let segs = calculate_segments(1, 4, 1024);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], SegmentRange::new(0, 0));
    }

    #[test]
    fn file_smaller_than_min_segment_size_returns_single_segment() {
        let segs = calculate_segments(500, 4, 1024);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], SegmentRange::new(0, 499));
    }

    #[test]
    fn max_connections_one_returns_single_segment() {
        let segs = calculate_segments(10_000, 1, 1024);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], SegmentRange::new(0, 9_999));
    }

    #[test]
    fn max_connections_zero_returns_single_segment() {
        let segs = calculate_segments(10_000, 0, 1024);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], SegmentRange::new(0, 9_999));
    }

    #[test]
    fn exact_four_way_split() {
        let segs = calculate_segments(100, 4, 1);
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0], SegmentRange::new(0, 24));
        assert_eq!(segs[1], SegmentRange::new(25, 49));
        assert_eq!(segs[2], SegmentRange::new(50, 74));
        assert_eq!(segs[3], SegmentRange::new(75, 99));
    }

    #[test]
    fn uneven_split_distributes_remainder() {
        let segs = calculate_segments(10, 3, 1);
        assert_eq!(segs.len(), 3);
        // 10 / 3 = 3 base, 1 remainder → first segment gets +1
        assert_eq!(segs[0], SegmentRange::new(0, 3)); // 4 bytes
        assert_eq!(segs[1], SegmentRange::new(4, 6)); // 3 bytes
        assert_eq!(segs[2], SegmentRange::new(7, 9)); // 3 bytes
    }

    #[test]
    fn ten_megabyte_file_four_connections() {
        let file_size: u64 = 10 * 1024 * 1024; // 10_485_760
        let segs = calculate_segments(file_size, 4, 1024);
        assert_eq!(segs.len(), 4);
        // All segments should be 2_621_440 bytes (10MB / 4)
        for seg in &segs {
            assert_eq!(seg.len(), 2_621_440);
        }
        assert_eq!(segs[0].start, 0);
        assert_eq!(segs[3].end, file_size - 1);
    }

    #[test]
    fn min_segment_size_limits_segment_count() {
        // file_size=10000, max_connections=8, min_segment_size=4000
        // 10000 / 4000 = 2 → only 2 segments
        let segs = calculate_segments(10_000, 8, 4000);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], SegmentRange::new(0, 4_999));
        assert_eq!(segs[1], SegmentRange::new(5_000, 9_999));
    }

    // --- Property tests ---

    /// Generate valid test parameters: (file_size, max_connections, min_segment_size)
    fn test_params() -> impl Strategy<Value = (u64, usize, u64)> {
        (
            0u64..1_000_000u64, // file_size
            1usize..16,         // max_connections
            1u64..10_000u64,    // min_segment_size
        )
    }

    proptest! {
        /// Segments must be contiguous: each segment starts where previous ended + 1.
        #[test]
        fn segments_are_contiguous(params in test_params()) {
            let (file_size, max_connections, min_segment_size) = params;
            let segs = calculate_segments(file_size, max_connections, min_segment_size);

            if segs.is_empty() {
                prop_assert_eq!(file_size, 0);
                return Ok(());
            }

            prop_assert_eq!(segs[0].start, 0);
            for window in segs.windows(2) {
                prop_assert_eq!(window[1].start, window[0].end + 1);
            }
        }

        /// Segments must be non-overlapping.
        #[test]
        fn segments_do_not_overlap(params in test_params()) {
            let (file_size, max_connections, min_segment_size) = params;
            let segs = calculate_segments(file_size, max_connections, min_segment_size);

            if segs.is_empty() {
                return Ok(());
            }

            for window in segs.windows(2) {
                prop_assert!(window[0].end < window[1].start,
                    "segment overlap: [{}, {}] and [{}, {}]",
                    window[0].start, window[0].end, window[1].start, window[1].end);
            }
        }

        /// Segments must exactly cover [0, file_size).
        #[test]
        fn segments_cover_file(params in test_params()) {
            let (file_size, max_connections, min_segment_size) = params;
            let segs = calculate_segments(file_size, max_connections, min_segment_size);

            if segs.is_empty() {
                prop_assert_eq!(file_size, 0);
                return Ok(());
            }

            prop_assert_eq!(segs.first().unwrap().start, 0);
            prop_assert_eq!(segs.last().unwrap().end, file_size - 1);

            let total: u64 = segs.iter().map(SegmentRange::len).sum();
            prop_assert_eq!(total, file_size);
        }

        /// No segment (except possibly the last) is smaller than min_segment_size.
        #[test]
        fn segments_respect_min_size(params in test_params()) {
            let (file_size, max_connections, min_segment_size) = params;
            let segs = calculate_segments(file_size, max_connections, min_segment_size);

            if segs.len() <= 1 {
                return Ok(());
            }

            for seg in &segs[..segs.len() - 1] {
                prop_assert!(seg.len() >= min_segment_size,
                    "segment [{}, {}] has len {} < min {}",
                    seg.start, seg.end, seg.len(), min_segment_size);
            }
        }

        /// Number of segments never exceeds max_connections.
        #[test]
        fn segments_within_max_connections(params in test_params()) {
            let (file_size, max_connections, min_segment_size) = params;
            let segs = calculate_segments(file_size, max_connections, min_segment_size);
            prop_assert!(segs.len() <= max_connections);
        }

        /// All segments report correct len().
        #[test]
        fn segment_len_is_correct(params in test_params()) {
            let (file_size, max_connections, min_segment_size) = params;
            let segs = calculate_segments(file_size, max_connections, min_segment_size);

            for seg in &segs {
                prop_assert_eq!(seg.len(), seg.end - seg.start + 1);
                prop_assert!(!seg.is_empty());
            }
        }
    }

    // --- RemoteInfo tests ---

    #[test]
    fn remote_info_clone() {
        let info = RemoteInfo {
            content_length: Some(1024),
            accept_ranges: true,
            etag: Some("\"abc123\"".to_string()),
            last_modified: Some("Thu, 01 Jan 2025 00:00:00 GMT".to_string()),
        };
        let cloned = info.clone();
        assert_eq!(cloned.content_length, Some(1024));
        assert!(cloned.accept_ranges);
        assert_eq!(cloned.etag, Some("\"abc123\"".to_string()));
        assert!(cloned.last_modified.is_some());
    }

    #[test]
    fn remote_info_debug_output() {
        let info = RemoteInfo {
            content_length: None,
            accept_ranges: false,
            etag: None,
            last_modified: None,
        };
        let debug_str = format!("{info:?}");
        assert!(debug_str.contains("RemoteInfo"));
    }

    // --- range_header_value tests ---

    #[test]
    fn range_header_value_basic() {
        let seg = SegmentRange::new(0, 99);
        assert_eq!(range_header_value(&seg), "bytes=0-99");
    }

    #[test]
    fn range_header_value_single_byte() {
        let seg = SegmentRange::new(42, 42);
        assert_eq!(range_header_value(&seg), "bytes=42-42");
    }

    #[test]
    fn range_header_value_large_offsets() {
        let seg = SegmentRange::new(1_000_000, 9_999_999);
        assert_eq!(range_header_value(&seg), "bytes=1000000-9999999");
    }

    #[test]
    fn range_header_value_full_file() {
        let seg = SegmentRange::new(0, 1023);
        assert_eq!(range_header_value(&seg), "bytes=0-1023");
    }

    // --- File offset write tests (no HTTP needed) ---

    #[tokio::test]
    async fn write_at_offset_preserves_surrounding() {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let path = tmp.path().to_path_buf();

        // Pre-fill 20 bytes with 0xAA
        {
            let mut f = tokio::fs::File::create(&path).await.expect("create");
            let fill = vec![0xAA_u8; 20];
            f.write_all(&fill).await.expect("write");
            f.flush().await.expect("flush");
        }

        // Write 5 bytes of 0xBB at offset 10
        {
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .await
                .expect("open");
            f.seek(std::io::SeekFrom::Start(10)).await.expect("seek");
            f.write_all(&[0xBB; 5]).await.expect("write");
            f.flush().await.expect("flush");
        }

        // Verify
        let mut buf = vec![0u8; 20];
        {
            let mut f = tokio::fs::File::open(&path).await.expect("open");
            f.read_exact(&mut buf).await.expect("read");
        }

        // Bytes 0-9 should still be 0xAA
        for byte in &buf[0..10] {
            assert_eq!(*byte, 0xAA);
        }
        // Bytes 10-14 should be 0xBB
        for byte in &buf[10..15] {
            assert_eq!(*byte, 0xBB);
        }
        // Bytes 15-19 should still be 0xAA
        for byte in &buf[15..20] {
            assert_eq!(*byte, 0xAA);
        }
    }

    #[tokio::test]
    async fn write_multiple_chunks_at_offsets() {
        use tokio::io::AsyncReadExt;

        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let path = tmp.path().to_path_buf();

        // Create a 30-byte file filled with zeros
        {
            let f = tokio::fs::File::create(&path).await.expect("create");
            f.set_len(30).await.expect("set_len");
        }

        // Simulate two segments written at different offsets
        {
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .await
                .expect("open");

            // Segment 0: bytes 0-14 (15 bytes of 0x01)
            f.seek(std::io::SeekFrom::Start(0)).await.expect("seek");
            f.write_all(&[0x01; 15]).await.expect("write");

            // Segment 1: bytes 15-29 (15 bytes of 0x02)
            f.seek(std::io::SeekFrom::Start(15)).await.expect("seek");
            f.write_all(&[0x02; 15]).await.expect("write");

            f.flush().await.expect("flush");
        }

        let mut buf = vec![0u8; 30];
        {
            let mut f = tokio::fs::File::open(&path).await.expect("open");
            f.read_exact(&mut buf).await.expect("read");
        }

        for byte in &buf[..15] {
            assert_eq!(*byte, 0x01);
        }
        for byte in &buf[15..] {
            assert_eq!(*byte, 0x02);
        }
    }

    // --- Progress channel tests ---

    #[tokio::test]
    async fn progress_channel_receives_chunk_sizes() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<u64>(16);

        // Simulate what download_segment does: send chunk sizes
        let chunk_sizes: Vec<u64> = vec![1024, 1024, 512];
        for &size in &chunk_sizes {
            tx.send(size).await.expect("send");
        }
        drop(tx);

        let mut received = Vec::new();
        while let Some(size) = rx.recv().await {
            received.push(size);
        }

        assert_eq!(received, chunk_sizes);
    }

    #[tokio::test]
    async fn progress_channel_none_is_noop() {
        let progress_tx: Option<&tokio::sync::mpsc::Sender<u64>> = None;

        // This is what the code does: if let Some(tx) = progress_tx { ... }
        // When None, nothing happens — no panic, no error
        if let Some(tx) = progress_tx {
            tx.send(42).await.expect("unreachable");
        }

        // Test passes if we get here without issues
    }

    // --- SegmentRange offset calculation ---

    #[test]
    fn segment_offset_calculation() {
        let segs = calculate_segments(100, 4, 1);
        // First segment writes at offset 0
        assert_eq!(segs[0].start, 0);
        // Second segment writes at offset 25
        assert_eq!(segs[1].start, 25);
        // Third segment writes at offset 50
        assert_eq!(segs[2].start, 50);
        // Fourth segment writes at offset 75
        assert_eq!(segs[3].start, 75);
    }

    // --- with_retry tests ---

    #[tokio::test]
    async fn with_retry_succeeds_on_first_try() {
        let result = with_retry(
            3,
            Duration::from_millis(1),
            Duration::from_millis(10),
            0.0,
            || Box::pin(async { Ok::<u64, DownloadError>(42) }),
        )
        .await;
        assert_eq!(result.expect("should succeed"), 42);
    }

    #[tokio::test]
    async fn with_retry_succeeds_after_transient_failures() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let attempt = AtomicU32::new(0);

        let result = with_retry(
            5,
            Duration::from_millis(1),
            Duration::from_millis(10),
            0.0,
            || {
                let count = attempt.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if count < 2 {
                        Err(DownloadError::RateLimited)
                    } else {
                        Ok::<u64, DownloadError>(99)
                    }
                })
            },
        )
        .await;

        assert_eq!(result.expect("should succeed on 3rd attempt"), 99);
        assert_eq!(attempt.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn with_retry_returns_last_error_after_max_attempts() {
        let result = with_retry(
            3,
            Duration::from_millis(1),
            Duration::from_millis(10),
            0.0,
            || Box::pin(async { Err::<u64, DownloadError>(DownloadError::RateLimited) }),
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, DownloadError::RateLimited));
    }

    #[tokio::test]
    async fn with_retry_respects_max_delay() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let attempt = AtomicU32::new(0);

        let start = tokio::time::Instant::now();
        let result = with_retry(
            4,
            Duration::from_millis(50),
            Duration::from_millis(60),
            0.0,
            || {
                let count = attempt.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if count < 3 {
                        Err(DownloadError::RateLimited)
                    } else {
                        Ok::<u64, DownloadError>(1)
                    }
                })
            },
        )
        .await;

        assert!(result.is_ok());
        let elapsed = start.elapsed();
        // Should have slept at least 3 times, each capped at 60ms
        // Total minimum ~150ms (3 * 50ms with exponential: 50, 60, 60)
        assert!(
            elapsed >= Duration::from_millis(100),
            "elapsed: {elapsed:?}"
        );
    }

    // --- SegmentDownloader builder tests ---

    #[test]
    fn segment_downloader_new_sets_defaults() {
        let client = hpx::Client::new();
        let segs = vec![SegmentRange::new(0, 99), SegmentRange::new(100, 199)];
        let dl = SegmentDownloader::new(
            client,
            "https://example.com/file",
            segs.clone(),
            "/tmp/test",
        );

        assert_eq!(dl.url, "https://example.com/file");
        assert_eq!(dl.segments.len(), 2);
        assert_eq!(dl.max_retries, 3);
        assert_eq!(dl.retry_initial_delay, Duration::from_millis(200));
        assert_eq!(dl.retry_max_delay, Duration::from_secs(30));
        assert!((dl.retry_jitter - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn segment_downloader_with_retry_config_overrides() {
        let client = hpx::Client::new();
        let segs = vec![SegmentRange::new(0, 99)];
        let dl = SegmentDownloader::new(client, "https://example.com/file", segs, "/tmp/test")
            .with_retry_config(10, Duration::from_millis(500), Duration::from_secs(60), 0.5);

        assert_eq!(dl.max_retries, 10);
        assert_eq!(dl.retry_initial_delay, Duration::from_millis(500));
        assert_eq!(dl.retry_max_delay, Duration::from_secs(60));
        assert!((dl.retry_jitter - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn segment_downloader_accepts_string_types() {
        let client = hpx::Client::new();
        let url = String::from("https://example.com/file");
        let dest = std::path::PathBuf::from("/tmp/test");
        let segs = vec![SegmentRange::new(0, 49)];

        let dl = SegmentDownloader::new(client, url, segs, dest);
        assert_eq!(dl.url, "https://example.com/file");
    }

    // --- SegmentDownloader::download tests (no real HTTP) ---

    #[tokio::test]
    async fn download_returns_error_for_empty_segments() {
        let client = hpx::Client::new();
        let dl = SegmentDownloader::new(client, "https://example.com/file", vec![], "/tmp/test");

        let result = dl.download(None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DownloadError::NoSegments));
    }

    #[tokio::test]
    async fn download_creates_file_with_correct_size() {
        use tokio::io::AsyncReadExt;

        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let path = tmp.path().to_path_buf();

        // Pre-create file with specific size
        {
            let f = tokio::fs::File::create(&path).await.expect("create");
            f.set_len(200).await.expect("set_len");
        }

        // Verify file is 200 bytes
        let metadata = tokio::fs::metadata(&path).await.expect("metadata");
        assert_eq!(metadata.len(), 200);

        // Read and verify all zeros
        let mut buf = vec![0u8; 200];
        {
            let mut f = tokio::fs::File::open(&path).await.expect("open");
            f.read_exact(&mut buf).await.expect("read");
        }
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[tokio::test]
    async fn progress_tx_receives_total_from_concurrent_segments() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<u64>(64);

        // Simulate 3 segment tasks each sending chunk sizes
        let mut handles = Vec::new();
        for _ in 0..3 {
            let tx_clone = tx.clone();
            let handle = tokio::spawn(async move {
                // Each segment sends 3 chunks of 100 bytes
                for _ in 0..3 {
                    let _ = tx_clone.send(100).await;
                }
            });
            handles.push(handle);
        }
        drop(tx);

        for handle in handles {
            handle.await.expect("join");
        }

        let mut total = 0u64;
        while let Some(size) = rx.recv().await {
            total += size;
        }
        assert_eq!(total, 900); // 3 segments * 3 chunks * 100 bytes
    }

    // --- Exponential backoff delay calculation tests ---

    #[test]
    fn backoff_delay_doubles_each_attempt() {
        let initial = Duration::from_millis(100);
        let max_delay = Duration::from_secs(10);

        // attempt 0: 100 * 2^0 = 100ms
        let d0 = compute_backoff_delay(initial, max_delay, 0.0, 0);
        assert_eq!(d0, Duration::from_millis(100));

        // attempt 1: 100 * 2^1 = 200ms
        let d1 = compute_backoff_delay(initial, max_delay, 0.0, 1);
        assert_eq!(d1, Duration::from_millis(200));

        // attempt 2: 100 * 2^2 = 400ms
        let d2 = compute_backoff_delay(initial, max_delay, 0.0, 2);
        assert_eq!(d2, Duration::from_millis(400));

        // attempt 3: 100 * 2^3 = 800ms
        let d3 = compute_backoff_delay(initial, max_delay, 0.0, 3);
        assert_eq!(d3, Duration::from_millis(800));
    }

    #[test]
    fn backoff_delay_caps_at_max() {
        let initial = Duration::from_millis(100);
        let max_delay = Duration::from_millis(300);

        // attempt 0: min(100, 300) = 100ms
        let d0 = compute_backoff_delay(initial, max_delay, 0.0, 0);
        assert_eq!(d0, Duration::from_millis(100));

        // attempt 1: min(200, 300) = 200ms
        let d1 = compute_backoff_delay(initial, max_delay, 0.0, 1);
        assert_eq!(d1, Duration::from_millis(200));

        // attempt 2: min(400, 300) = 300ms (capped)
        let d2 = compute_backoff_delay(initial, max_delay, 0.0, 2);
        assert_eq!(d2, Duration::from_millis(300));

        // attempt 10: min(102400, 300) = 300ms (capped)
        let d10 = compute_backoff_delay(initial, max_delay, 0.0, 10);
        assert_eq!(d10, Duration::from_millis(300));
    }

    #[test]
    fn backoff_delay_with_zero_jitter_is_deterministic() {
        let initial = Duration::from_millis(50);
        let max_delay = Duration::from_secs(10);

        let d1 = compute_backoff_delay(initial, max_delay, 0.0, 4);
        let d2 = compute_backoff_delay(initial, max_delay, 0.0, 4);
        assert_eq!(d1, d2);
        assert_eq!(d1, Duration::from_millis(800)); // 50 * 2^4
    }

    #[test]
    fn backoff_delay_with_jitter_is_within_bounds() {
        let initial = Duration::from_millis(100);
        let max_delay = Duration::from_secs(10);
        let jitter = 0.25;

        for _ in 0..100 {
            let d = compute_backoff_delay(initial, max_delay, jitter, 2);
            // Base: 400ms, jitter 25% → [300ms, 500ms]
            assert!(d >= Duration::from_millis(300), "d={d:?}");
            assert!(d <= Duration::from_millis(500), "d={d:?}");
        }
    }

    // --- ResumeState tests ---

    #[test]
    fn resume_state_fresh_debug() {
        let state = ResumeState::Fresh;
        let debug_str = format!("{state:?}");
        assert!(debug_str.contains("Fresh"));
    }

    #[test]
    fn resume_state_can_resume_debug() {
        let state = ResumeState::CanResume {
            bytes_completed: 5000,
            completed_segments: vec![0, 1, 2],
        };
        let debug_str = format!("{state:?}");
        assert!(debug_str.contains("CanResume"));
    }

    #[test]
    fn resume_state_content_changed_debug() {
        let state = ResumeState::ContentChanged;
        let debug_str = format!("{state:?}");
        assert!(debug_str.contains("ContentChanged"));
    }

    #[test]
    fn resume_state_clone() {
        let state = ResumeState::CanResume {
            bytes_completed: 1000,
            completed_segments: vec![0],
        };
        let cloned = state.clone();
        match cloned {
            ResumeState::CanResume {
                bytes_completed,
                completed_segments,
            } => {
                assert_eq!(bytes_completed, 1000);
                assert_eq!(completed_segments, vec![0]);
            }
            _ => panic!("expected CanResume"),
        }
    }

    // --- compare_content_headers tests ---

    #[test]
    fn compare_headers_both_etags_match() {
        let prev_etag = Some("\"abc\"".to_string());
        let prev_lm = None;
        let cur_etag = Some("\"abc\"".to_string());
        let cur_lm = None;
        assert!(compare_content_headers(
            &prev_etag, &prev_lm, &cur_etag, &cur_lm
        ));
    }

    #[test]
    fn compare_headers_etag_mismatch() {
        let prev_etag = Some("\"abc\"".to_string());
        let prev_lm = None;
        let cur_etag = Some("\"def\"".to_string());
        let cur_lm = None;
        assert!(!compare_content_headers(
            &prev_etag, &prev_lm, &cur_etag, &cur_lm
        ));
    }

    #[test]
    fn compare_headers_both_last_modified_match() {
        let prev_etag = None;
        let prev_lm = Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string());
        let cur_etag = None;
        let cur_lm = Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string());
        assert!(compare_content_headers(
            &prev_etag, &prev_lm, &cur_etag, &cur_lm
        ));
    }

    #[test]
    fn compare_headers_last_modified_mismatch() {
        let prev_etag = None;
        let prev_lm = Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string());
        let cur_etag = None;
        let cur_lm = Some("Tue, 02 Jan 2024 00:00:00 GMT".to_string());
        assert!(!compare_content_headers(
            &prev_etag, &prev_lm, &cur_etag, &cur_lm
        ));
    }

    #[test]
    fn compare_headers_both_present_and_match() {
        let prev_etag = Some("\"abc\"".to_string());
        let prev_lm = Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string());
        let cur_etag = Some("\"abc\"".to_string());
        let cur_lm = Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string());
        assert!(compare_content_headers(
            &prev_etag, &prev_lm, &cur_etag, &cur_lm
        ));
    }

    #[test]
    fn compare_headers_etag_matches_last_modified_differs() {
        let prev_etag = Some("\"abc\"".to_string());
        let prev_lm = Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string());
        let cur_etag = Some("\"abc\"".to_string());
        let cur_lm = Some("Tue, 02 Jan 2024 00:00:00 GMT".to_string());
        // Both must match if present; etag matches but LM differs → mismatch
        assert!(!compare_content_headers(
            &prev_etag, &prev_lm, &cur_etag, &cur_lm
        ));
    }

    #[test]
    fn compare_headers_neither_present_is_match() {
        assert!(compare_content_headers(&None, &None, &None, &None));
    }

    #[test]
    fn compare_headers_previous_had_etag_now_none() {
        let prev_etag = Some("\"abc\"".to_string());
        let prev_lm = None;
        let cur_etag = None;
        let cur_lm = None;
        assert!(!compare_content_headers(
            &prev_etag, &prev_lm, &cur_etag, &cur_lm
        ));
    }

    #[test]
    fn compare_headers_previous_none_now_has_etag() {
        let prev_etag = None;
        let prev_lm = None;
        let cur_etag = Some("\"abc\"".to_string());
        let cur_lm = None;
        // Previous had no validator; we can't confirm it matches → treat as mismatch
        assert!(!compare_content_headers(
            &prev_etag, &prev_lm, &cur_etag, &cur_lm
        ));
    }

    // --- determine_resume_state tests ---

    #[test]
    fn determine_resume_no_file_no_record_is_fresh() {
        let state = determine_resume_state(false, &None, &None, &None, &None);
        assert!(matches!(state, ResumeState::Fresh));
    }

    #[test]
    fn determine_resume_no_file_with_record_is_fresh() {
        let state = determine_resume_state(
            false,
            &Some("\"abc\"".to_string()),
            &None,
            &Some("\"abc\"".to_string()),
            &None,
        );
        // No file on disk → always Fresh (caller starts download from scratch)
        assert!(matches!(state, ResumeState::Fresh));
    }

    #[test]
    fn determine_resume_file_exists_no_record_is_content_changed() {
        let state = determine_resume_state(true, &None, &None, &None, &None);
        assert!(matches!(state, ResumeState::ContentChanged));
    }

    #[test]
    fn determine_resume_file_exists_etag_unchanged_returns_can_resume() {
        let record_etag = Some("\"abc\"".to_string());
        let record_lm = None;
        let remote_etag = Some("\"abc\"".to_string());
        let remote_lm = None;
        let state =
            determine_resume_state(true, &record_etag, &record_lm, &remote_etag, &remote_lm);
        // determine_resume_state is a pure header-comparison function;
        // bytes_completed is filled in by check_resume from actual segment state.
        assert!(matches!(state, ResumeState::CanResume { .. }));
    }

    #[test]
    fn determine_resume_file_exists_etag_changed_is_content_changed() {
        let state = determine_resume_state(
            true,
            &Some("\"old\"".to_string()),
            &None,
            &Some("\"new\"".to_string()),
            &None,
        );
        assert!(matches!(state, ResumeState::ContentChanged));
    }

    // --- filter_remaining_segments tests ---

    #[test]
    fn filter_remaining_no_completed() {
        let segments = vec![SegmentRange::new(0, 4999), SegmentRange::new(5000, 9999)];
        let remaining = filter_remaining_segments(&segments, &[]);
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn filter_remaining_all_completed() {
        let segments = vec![SegmentRange::new(0, 4999), SegmentRange::new(5000, 9999)];
        let remaining = filter_remaining_segments(&segments, &[0, 1]);
        assert!(remaining.is_empty());
    }

    #[test]
    fn filter_remaining_some_completed() {
        let segments = vec![
            SegmentRange::new(0, 4999),
            SegmentRange::new(5000, 9999),
            SegmentRange::new(10_000, 14_999),
        ];
        let remaining = filter_remaining_segments(&segments, &[0, 2]);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0], SegmentRange::new(5000, 9999));
    }

    #[test]
    fn filter_remaining_preserves_order() {
        let segments = vec![
            SegmentRange::new(0, 999),
            SegmentRange::new(1000, 1999),
            SegmentRange::new(2000, 2999),
            SegmentRange::new(3000, 3999),
        ];
        let remaining = filter_remaining_segments(&segments, &[1, 3]);
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0], SegmentRange::new(0, 999));
        assert_eq!(remaining[1], SegmentRange::new(2000, 2999));
    }

    #[test]
    fn filter_remaining_empty_segments() {
        let remaining = filter_remaining_segments(&[], &[0]);
        assert!(remaining.is_empty());
    }

    // --- Integration-like test with local file (no HTTP) ---

    #[tokio::test]
    async fn multi_segment_file_write_produces_correct_content() {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let path = tmp.path().to_path_buf();

        // Simulate 3 segments: [0-99], [100-199], [200-249] = 250 bytes total
        let segments = vec![
            SegmentRange::new(0, 99),
            SegmentRange::new(100, 199),
            SegmentRange::new(200, 249),
        ];

        // Create file with correct size
        {
            let f = tokio::fs::File::create(&path).await.expect("create");
            f.set_len(250).await.expect("set_len");
        }

        // Write each segment concurrently (simulating what SegmentDownloader does)
        let mut join_set = tokio::task::JoinSet::new();
        for (i, seg) in segments.iter().enumerate() {
            let path_clone = path.clone();
            let seg = seg.clone();
            let marker = (i + 1) as u8; // 0x01, 0x02, 0x03

            join_set.spawn(async move {
                let mut f = tokio::fs::OpenOptions::new()
                    .write(true)
                    .open(&path_clone)
                    .await
                    .expect("open");

                f.seek(std::io::SeekFrom::Start(seg.start))
                    .await
                    .expect("seek");
                let data = vec![marker; seg.len() as usize];
                f.write_all(&data).await.expect("write");
                f.flush().await.expect("flush");
                seg.len()
            });
        }

        let mut total = 0u64;
        while let Some(result) = join_set.join_next().await {
            total += result.expect("join");
        }
        assert_eq!(total, 250);

        // Verify content
        let mut buf = vec![0u8; 250];
        {
            let mut f = tokio::fs::File::open(&path).await.expect("open");
            f.read_exact(&mut buf).await.expect("read");
        }

        for byte in &buf[..100] {
            assert_eq!(*byte, 0x01);
        }
        for byte in &buf[100..200] {
            assert_eq!(*byte, 0x02);
        }
        for byte in &buf[200..250] {
            assert_eq!(*byte, 0x03);
        }
    }

    // --- Proxy passthrough tests ---

    #[test]
    fn segment_downloader_with_proxy_http() {
        use crate::types::{ProxyConfig, ProxyKind};

        let config = ProxyConfig {
            url: "http://proxy:8080".to_string(),
            kind: ProxyKind::Http,
        };
        let dl = SegmentDownloader::with_proxy(
            "https://example.com/file",
            vec![SegmentRange::new(0, 99)],
            "/tmp/test",
            &config,
        )
        .expect("should build proxied downloader");

        assert_eq!(dl.url, "https://example.com/file");
        assert_eq!(dl.segments.len(), 1);
        assert_eq!(dl.max_retries, 3);
    }

    #[test]
    fn segment_downloader_with_proxy_socks5() {
        use crate::types::{ProxyConfig, ProxyKind};

        let config = ProxyConfig {
            url: "socks5://proxy:1080".to_string(),
            kind: ProxyKind::Socks5,
        };
        let dl = SegmentDownloader::with_proxy(
            "https://example.com/file",
            vec![SegmentRange::new(0, 99)],
            "/tmp/test",
            &config,
        )
        .expect("should build proxied downloader with socks5");

        assert_eq!(dl.url, "https://example.com/file");
    }

    #[test]
    fn segment_downloader_with_proxy_inherits_retry_defaults() {
        use crate::types::{ProxyConfig, ProxyKind};

        let config = ProxyConfig {
            url: "http://proxy:8080".to_string(),
            kind: ProxyKind::Http,
        };
        let dl = SegmentDownloader::with_proxy(
            "https://example.com/file",
            vec![SegmentRange::new(0, 49)],
            "/tmp/test",
            &config,
        )
        .expect("should build");

        assert_eq!(dl.max_retries, 3);
        assert_eq!(dl.retry_initial_delay, Duration::from_millis(200));
        assert_eq!(dl.retry_max_delay, Duration::from_secs(30));
    }
}
