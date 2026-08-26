//! High-performance download engine built on hpx.
//!
//! `hpx-dl` provides segmented HTTP downloads with metalink support,
//! pluggable storage, rate limiting, and progress reporting.

// The `Engine::spawn_download` async block nests deeply through
// `execute_download` → `download_from_candidate` → `tokio::select!`, and under
// workspace-wide feature unification the trait solver overflows its default
// recursion limit (128) while proving the spawned future is `Send`. Raise the
// budget so `cargo clippy --all` / `cargo check --workspace` stay green.
#![recursion_limit = "512"]

pub mod checksum;
#[cfg(feature = "http")]
pub mod engine;
pub mod error;
pub mod event;
pub mod metalink;
pub mod parse;
mod persistence;
pub mod queue;
#[cfg(feature = "http")]
pub mod segment;
pub mod speed;
pub mod storage;
pub mod types;

pub use checksum::{compute_checksum, verify_checksum};
#[cfg(feature = "http")]
pub use engine::{DownloadEngine, EngineBuilder, EngineConfig};
pub use error::DownloadError;
pub use event::{EventBroadcaster, EventError};
pub use metalink::{MetalinkFile, MetalinkUrl, parse_metalink};
pub use parse::{parse_checksum, parse_proxy_config, parse_speed_limit};
pub use queue::{PriorityQueue, QueueEntry};
#[cfg(feature = "http")]
pub use segment::{
    RemoteInfo, ResumeState, SegmentDownloader, SegmentRange, calculate_segments,
    compare_content_headers, compute_backoff_delay, determine_resume_state,
    filter_remaining_segments, probe_remote, range_header_value, with_retry,
};
pub use speed::{CompositeLimiter, SpeedLimiter};
#[cfg(feature = "test")]
pub use storage::MemoryStorage;
#[cfg(feature = "sqlite")]
pub use storage::SqliteStorage;
pub use storage::{DownloadRecord, SegmentState, SegmentStatus, Storage};
pub use types::{
    ChecksumSpec, DownloadEvent, DownloadId, DownloadPriority, DownloadRequest, DownloadState,
    DownloadStatus, HashAlgorithm, ProxyConfig, ProxyKind,
};
