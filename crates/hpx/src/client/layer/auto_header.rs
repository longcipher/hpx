//! Automatic header value management inspired by ureq's `AutoHeaderValue`.
//!
//! This type provides a way to distinguish between:
//! - No automatic header (`None`)
//! - Default behavior (e.g., `hpx/2.3.1`)
//! - User-provided value
//!
//! This enables clean management of headers like `User-Agent`, `Accept`,
//! and `Accept-Encoding` without ambiguity.

use std::sync::Arc;

/// Controls automatic header value behavior.
///
/// - `None` – do not send this header automatically.
/// - `Default` – use the library default (e.g., `hpx/<version>` for User-Agent).
/// - `Provided` – use the user-specified value.
#[derive(Debug, Clone, Default)]
pub enum AutoHeaderValue {
    /// No automatic header.
    None,

    /// Use library default behavior.
    #[default]
    Default,

    /// User provided header value.
    Provided(Arc<String>),
}

impl AutoHeaderValue {
    /// Returns the string value, falling back to `default` for `Default` variant.
    ///
    /// Returns `None` if the header should not be sent.
    #[must_use]
    pub fn as_str(&self, default: &'static str) -> Option<&str> {
        match self {
            Self::None => Some(""),
            Self::Default => Some(default),
            Self::Provided(v) => Some(v.as_str()),
        }
        .filter(|s| !s.is_empty())
    }

    /// Returns `true` if this is the `None` variant.
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns `true` if this is the `Default` variant.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }
}

impl<S: AsRef<str>> From<S> for AutoHeaderValue {
    fn from(value: S) -> Self {
        if value.as_ref().is_empty() {
            Self::None
        } else {
            Self::Provided(Arc::new(value.as_ref().to_owned()))
        }
    }
}
