//! HTTP/3 client and server.
//!
//! This crate is a fork of [`hyperium/h3`](https://github.com/hyperium/h3) vendored
//! into the hpx workspace with RFC 9220 WebSocket support. It tracks upstream
//! closely; lint suppressions below exist to keep the diff minimal.

// Vendored third-party crate: the following suppressions keep the upstream
// source tractable while still inheriting the workspace lint baseline.
#![allow(
    unreachable_pub,
    missing_debug_implementations,
    missing_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::trivially_copy_pass_by_ref,
    clippy::redundant_closure_for_method_calls,
    clippy::too_many_lines,
    clippy::module_name_repetitions,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools,
    clippy::cognitive_complexity,
    clippy::future_not_send,
    clippy::needless_pass_by_ref_mut,
    clippy::similar_names,
    clippy::manual_string_new,
    clippy::len_without_is_empty,
    clippy::should_implement_trait,
    clippy::return_self_not_must_use,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_const_for_fn,
    clippy::manual_let_else,
    clippy::if_not_else,
    clippy::derivable_impls,
    clippy::inline_always,
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::option_option,
    clippy::single_match_else,
    clippy::question_mark,
    clippy::manual_is_variant_and,
    clippy::match_same_arms,
    clippy::unnecessary_wraps,
    clippy::unnecessary_struct_initialization,
    clippy::needless_borrow,
    clippy::map_unwrap_or,
    clippy::unnested_or_patterns,
    clippy::redundant_else,
    clippy::unnecessary_self_imports,
    clippy::items_after_statements,
    clippy::used_underscore_binding,
    clippy::no_effect_underscore_binding,
    clippy::default_trait_access,
    clippy::uninlined_format_args,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::borrow_as_ptr,
    clippy::branches_sharing_code,
    clippy::empty_line_after_doc_comments,
    clippy::empty_line_after_outer_attr,
    clippy::enum_glob_use,
    clippy::equatable_if_let,
    clippy::explicit_into_iter_loop,
    clippy::explicit_iter_loop,
    clippy::flat_map_option,
    clippy::if_then_some_else_none,
    clippy::implicit_clone,
    clippy::imprecise_flops,
    clippy::iter_on_empty_collections,
    clippy::iter_on_single_items,
    clippy::iter_with_drain,
    clippy::iter_without_into_iter,
    clippy::large_stack_frames,
    clippy::manual_assert,
    clippy::manual_clamp,
    clippy::manual_string_new,
    clippy::mutex_integer,
    clippy::naive_bytecount,
    clippy::needless_bitwise_bool,
    clippy::needless_continue,
    clippy::needless_for_each,
    clippy::nonstandard_macro_braces,
    clippy::option_as_ref_cloned,
    clippy::or_fun_call,
    clippy::path_buf_push_overwrite,
    clippy::read_zero_byte_vec,
    clippy::redundant_clone,
    clippy::single_char_pattern,
    clippy::string_lit_as_bytes,
    clippy::string_lit_chars_any,
    clippy::suboptimal_flops,
    clippy::suspicious_operation_groupings,
    clippy::trailing_empty_array,
    clippy::trait_duplication_in_bounds,
    clippy::transmute_undefined_repr,
    clippy::trivial_regex,
    clippy::tuple_array_conversions,
    clippy::type_repetition_in_bounds,
    clippy::uninhabited_references,
    clippy::unused_peekable,
    clippy::unused_rounding,
    clippy::use_self,
    clippy::useless_let_if_seq,
    clippy::while_float,
    clippy::zero_sized_map_values,
    clippy::as_ptr_cast_mut,
    clippy::fallible_impl_from,
    clippy::needless_collect,
    clippy::non_send_fields_in_send_ty,
    clippy::redundant_pub_crate,
    clippy::significant_drop_in_scrutinee,
    clippy::significant_drop_tightening,
    clippy::too_long_first_doc_paragraph,
    clippy::dbg_macro,
    clippy::todo,
    clippy::unimplemented,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::ref_option_ref,
    clippy::ref_option,
    clippy::format_push_string,
    clippy::unnecessary_wraps,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::manual_assert,
    clippy::manual_clamp,
    clippy::manual_let_else,
    clippy::match_same_arms,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::struct_excessive_bools,
    clippy::too_many_lines,
    clippy::unnested_or_patterns,
    clippy::unused_async,
    clippy::use_self
)]

pub mod client;

mod config;
pub mod ext;
pub mod quic;

pub mod server;

mod buf;

mod shared_state;

#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
pub use shared_state::{ConnectionState, SharedState};

pub mod error;

#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod connection;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod frame;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod proto;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(dead_code, missing_docs)]
pub mod qpack;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod stream;
#[cfg(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes")]
#[allow(missing_docs)]
pub mod webtransport;

#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod connection;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod frame;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod proto;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
#[allow(dead_code)]
mod qpack;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod stream;
#[cfg(not(feature = "i-implement-a-third-party-backend-and-opt-into-breaking-changes"))]
mod webtransport;

/// Quinn QUIC transport backend (merged from hpx-h3-quinn).
#[cfg(feature = "quinn")]
pub mod quinn;
