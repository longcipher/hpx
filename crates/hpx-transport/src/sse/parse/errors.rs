//! Error types used by the SSE parser.

use core::{
    fmt::{Display, Formatter},
    str::Utf8Error,
};

/// Errors produced by [`EventStream`](super::event_stream::EventStream).
#[derive(Debug, PartialEq)]
pub enum EventStreamError<E> {
    /// Something went wrong with the underlying stream.
    Transport(E),
    /// The stream contained invalid UTF-8.
    Utf8Error(Utf8Error),
}

impl<E> From<Utf8Error> for EventStreamError<E> {
    fn from(value: Utf8Error) -> Self {
        Self::Utf8Error(value)
    }
}

impl<E> Display for EventStreamError<E>
where
    E: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Transport(e) => e.fmt(f),
            Self::Utf8Error(e) => e.fmt(f),
        }
    }
}

impl<E> core::error::Error for EventStreamError<E> where E: core::error::Error {}
