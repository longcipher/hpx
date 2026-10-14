#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Streaming response support for [hpx](https://docs.rs/hpx/latest/hpx/) for different formats:
//! - JSON array stream format
//! - JSON Lines (NL/NewLines) format
//! - CSV stream format
//! - [Protobuf] len-prefixed stream format
//! - [Apache Arrow IPC] stream format
//!
//! This type of responses are useful when you are reading huge stream of objects from some source
//! (such as database, file, etc) and want to avoid huge memory allocations.
//!
//! # Features
//!
//! **Note:** The `default` features do not include any formats.
//!
//! - `json`: JSON array and JSON Lines (JSONL) stream formats
//! - `csv`: CSV stream format
//! - `protobuf`: [Protobuf] len-prefixed stream format
//! - `arrow`: [Apache Arrow IPC] stream format
//!
//! # Example
//!
//! ```rust,no_run
//! use hpx_streams::JsonStreamResponse as _;
//! use serde::Deserialize;
//!
//! #[derive(Debug, Clone, Deserialize)]
//! struct MyTestStructure {
//!     some_test_field: String,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = hpx::Client::new();
//!     let _stream = client
//!         .get("http://localhost:8080/json-array")
//!         .send()
//!         .await?
//!         .json_array_stream::<MyTestStructure>(1024);
//!
//!     Ok(())
//! }
//! ```
//!
//! [Apache Arrow IPC]: https://arrow.apache.org/docs/format/Columnar.html#serialization-and-interprocess-communication-ipc
//! [Protobuf]: https://protobuf.dev/programming-guides/encoding/

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
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub use json_stream::JsonStreamResponse;
#[cfg(feature = "json")]
mod json_array_codec;
#[cfg(feature = "json")]
mod json_stream;

#[cfg(feature = "csv")]
#[cfg_attr(docsrs, doc(cfg(feature = "csv")))]
pub use csv_stream::CsvStreamResponse;
#[cfg(feature = "csv")]
mod csv_stream;

#[cfg(feature = "protobuf")]
#[cfg_attr(docsrs, doc(cfg(feature = "protobuf")))]
pub use protobuf_stream::ProtobufStreamResponse;

use crate::error::StreamBodyError;
#[cfg(feature = "protobuf")]
mod protobuf_len_codec;
#[cfg(feature = "protobuf")]
mod protobuf_stream;

#[cfg(feature = "arrow")]
#[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
pub use arrow_ipc_stream::ArrowIpcStreamResponse;
#[cfg(feature = "arrow")]
mod arrow_ipc_len_codec;
#[cfg(feature = "arrow")]
mod arrow_ipc_stream;

/// Error types for streaming responses.
pub mod error;

/// Alias for the [`Result`] type returned by streaming responses.
pub type StreamBodyResult<T> = std::result::Result<T, StreamBodyError>;

use futures::StreamExt as _;

/// Normalize any format-specific stream into the common boxed shape.
///
/// All four streaming families (`JsonStreamResponse`, `CsvStreamResponse`,
/// `ProtobufStreamResponse`, `ArrowIpcStreamResponse`) produce streams
/// with the identical item type [`StreamBodyResult`]`<T>`; this helper erases
/// the concrete stream type so generic consumers can mix formats behind a
/// single return type without depending on which codec produced it.
///
/// # Example
///
/// ```rust,no_run
/// use futures::StreamExt as _;
/// use hpx_streams::{CsvStreamResponse as _, normalize_stream};
/// # use serde::Deserialize;
/// # #[derive(Deserialize)]
/// # struct Row;
/// #
/// # async fn example(client: hpx::Client) {
/// #     let csv_resp = client.get("http://localhost/csv").send().await.unwrap();
/// #     let json_resp = client.get("http://localhost/json").send().await.unwrap();
/// let streams: Vec<_> = vec![normalize_stream(csv_resp.csv_stream::<Row>(
///     64 * 1024,
///     true,
///     b',',
/// ))];
/// # }
/// ```
pub fn normalize_stream<S, T>(stream: S) -> futures::stream::BoxStream<'static, StreamBodyResult<T>>
where
    S: futures::Stream<Item = StreamBodyResult<T>> + Send + 'static,
{
    stream.boxed()
}
