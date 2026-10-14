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
// Unit tests are allowed to panic, unwrap, and print.
// Unit tests are allowed to panic, unwrap, and print.
// Unit tests are allowed to panic, unwrap, and print.
#![cfg_attr(
    test,
    allow(
        clippy::allow_attributes,
        clippy::assert_is_empty,
        clippy::borrow_as_ptr,
        clippy::cast_lossless,
        clippy::collection_is_never_read,
        clippy::collapsible_if,
        clippy::duration_suboptimal_units,
        clippy::explicit_iter_loop,
        clippy::expect_fun_call,
        clippy::expect_used,
        clippy::ignore_without_reason,
        clippy::ignored_unit_patterns,
        clippy::items_after_test_module,
        clippy::iter_kv_map,
        clippy::iter_with_drain,
        clippy::let_underscore_future,
        clippy::manual_div_ceil,
        clippy::manual_string_new,
        clippy::map_unwrap_or,
        clippy::needless_borrows_for_generic_args,
        clippy::needless_continue,
        clippy::needless_pass_by_ref_mut,
        clippy::needless_raw_string_hashes,
        clippy::nonminimal_bool,
        clippy::panic,
        clippy::print_stderr,
        clippy::print_stdout,
        clippy::redundant_async_block,
        clippy::redundant_clone,
        clippy::redundant_pattern_matching,
        clippy::struct_field_names,
        clippy::type_complexity,
        clippy::unchecked_time_subtraction,
        clippy::unreadable_literal,
        clippy::unnecessary_cast,
        clippy::unnecessary_trailing_comma,
        clippy::unnested_or_patterns,
        clippy::useless_borrows_in_formatting,
        clippy::useless_vec,
        clippy::use_self,
        clippy::unwrap_used,
        missing_docs,
        unfulfilled_lint_expectations,
    )
)]
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
