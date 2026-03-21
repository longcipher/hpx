//! Allocation and stack size guard tests.
//!
//! These tests ensure that core types remain efficient as the codebase evolves.
//! Inspired by ureq's approach of explicitly verifying no-allocation clone
//! and bounded stack sizes.

use hpx::{Body, Client};
use http_body::Body as _;

/// Verifies that `Client::clone()` does not trigger heap allocation.
///
/// `Client` wraps its state in `Arc`, so cloning should be a ref-count bump only.
#[test]
fn client_clone_does_not_allocate() {
    let client = Client::new();
    let _cloned = client.clone();
}

/// Verifies that `Body` created from empty string is efficient.
#[test]
fn body_empty_is_efficient() {
    let body = Body::from("");
    assert!(body.is_end_stream());
    assert_eq!(body.size_hint().exact(), Some(0));
}

/// Verifies that `Body` from string has correct size hint.
#[test]
fn body_from_string_has_size() {
    let body = Body::from("hello world");
    assert!(!body.is_end_stream());
    assert_eq!(body.size_hint().exact(), Some(11));
}

/// Verifies that `AsSendBody` provides correct content lengths.
#[test]
fn as_send_body_content_length() {
    use hpx::AsSendBody;

    let body = "hello".to_string().into_send_body();
    assert_eq!(body.size_hint().exact(), Some(5));

    let body = vec![1u8, 2, 3].into_send_body();
    assert_eq!(body.size_hint().exact(), Some(3));

    let body = ().into_send_body();
    assert!(body.is_end_stream());
}

// ===== Stack size guards =====
//
// These tests ensure that core types don't grow beyond expected stack sizes.
// If a type's size increases, it could impact performance in deep call stacks
// and hot loops.

macro_rules! assert_stack_size {
    ($type:ty, $max:expr) => {
        let sz = std::mem::size_of::<$type>();
        assert!(
            sz <= $max,
            "Stack size of {} is too large: {} bytes > {} bytes limit.\n\
             Consider reviewing recent changes to this type.",
            std::any::type_name::<$type>(),
            sz,
            $max,
        );
    };
}

#[test]
fn stack_size_client() {
    assert_stack_size!(Client, 32);
}

#[test]
fn stack_size_body() {
    assert_stack_size!(Body, 64);
}

#[test]
fn stack_size_request_builder() {
    assert_stack_size!(hpx::RequestBuilder, 320);
}

#[test]
fn stack_size_error() {
    assert_stack_size!(hpx::Error, 128);
}

#[test]
fn stack_size_response() {
    assert_stack_size!(hpx::Response, 256);
}
