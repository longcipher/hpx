//! Streaming bodies for Requests and Responses
//!
//! For both [Clients](crate::client), requests and
//! responses use streaming bodies, instead of complete buffering. This
//! allows applications to not use memory they don't need, and allows exerting
//! back-pressure on connections by only reading when asked.
//!
//! There are two pieces to this:
//!
//! - **The `Body` trait** describes all possible bodies. It allows any body type that
//!   implements `Body`, allowing applications to have fine-grained control over their streaming.
//! - **The [`Incoming`] concrete type**, which is an implementation of `Body`, and returned as
//!   a "receive stream" (so, for server requests and client responses).
//!
//! There are additional implementations available in [`http-body-util`][],
//! such as a `Full` or `Empty` body.
//!
//! [`http-body-util`]: https://docs.rs/http-body-util

mod incoming;
mod length;

pub(crate) use self::{
    incoming::{Incoming, Sender},
    length::DecodedLength,
};

const fn _assert_send_sync() {
    const fn assert_send<T: Send>() {}
    const fn assert_sync<T: Sync>() {}

    assert_send::<Incoming>();
    assert_sync::<Incoming>();
}
