//! Zero-copy, on-demand access to JSON market-data frames.
//!
//! Exchange depth and trade frames are typically a few kilobytes of JSON in
//! which only a handful of fields matter (`bids[0]`, `asks[0]`, the event
//! timestamp and the symbol). Deserializing the whole frame into an owned
//! struct allocates a `String` for every key and value and parses numbers that
//! are never read.
//!
//! [`BorrowedJsonFrame`] instead parses the frame **in place** into a
//! `simd-json` tape. The tape stores indices and borrowed `&str` slices that
//! point directly into the caller's buffer; no DOM is built, no string is
//! copied and no per-field allocation happens. A path lookup walks only the
//! tape nodes needed to reach the requested value.
//!
//! The buffer must be mutable because `simd-json` unescapes strings in situ.
//! Strings containing escape sequences are rewritten inside the buffer, so
//! the frame must be discarded (or the buffer refilled) after use.
//!
//! # Allocation behaviour
//!
//! [`BorrowedJsonFrame::parse`] allocates the tape itself once per frame.
//! [`BorrowedJsonFrame::parse_with_buffers`] additionally reuses the internal
//! string/indices buffers across frames, which keeps steady-state allocations
//! bounded to a single reusable tape. This is the intended path for a hot
//! market-data loop.
//!
//! # Example
//!
//! ```
//! use hpx_streams::{BorrowedJsonFrame, PathSegment as P};
//!
//! let mut buf = br#"{"s":"BTCUSDT","bids":[["67213.50","1.25"]]}"#.to_vec();
//! let frame = BorrowedJsonFrame::parse(&mut buf)?;
//! assert_eq!(frame.get_str(&[P::Key("s")]), Some("BTCUSDT"));
//! assert_eq!(
//!     frame.get_str(&[P::Key("bids"), P::Index(0), P::Index(0)]),
//!     Some("67213.50")
//! );
//! # Ok::<(), simd_json::Error>(())
//! ```

use simd_json::{
    Buffers, Tape,
    prelude::{ValueAsScalar, ValueIntoString},
    value::tape::Value,
};

/// One step of a path into a JSON value.
///
/// A path is an ordered slice of segments. Object members are addressed by key
/// and array elements by index; mixing them (for example
/// `["bids", 0, 0]`) is supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment<'a> {
    /// Select an object member by its name.
    Key(&'a str),
    /// Select an array element by its zero-based index.
    Index(usize),
}

impl<'a> PathSegment<'a> {
    /// Construct an object-key segment.
    #[inline]
    #[must_use]
    pub const fn key(name: &'a str) -> Self {
        Self::Key(name)
    }

    /// Construct an array-index segment.
    #[inline]
    #[must_use]
    pub const fn index(index: usize) -> Self {
        Self::Index(index)
    }
}

impl<'a> From<&'a str> for PathSegment<'a> {
    #[inline]
    fn from(name: &'a str) -> Self {
        Self::Key(name)
    }
}

impl From<usize> for PathSegment<'static> {
    #[inline]
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

/// A parsed JSON document that borrows all of its strings from the input.
///
/// Created by [`BorrowedJsonFrame::parse`] or
/// [`BorrowedJsonFrame::parse_with_buffers`]. The returned frame keeps an
/// immutable borrow of the mutable input buffer, so the buffer cannot be
/// mutated again until the frame is dropped.
#[derive(Debug)]
pub struct BorrowedJsonFrame<'a> {
    tape: Tape<'a>,
}

impl<'a> BorrowedJsonFrame<'a> {
    /// Parse `buf` in place with SIMD dispatch selected at runtime.
    ///
    /// # Errors
    ///
    /// Returns [`simd_json::Error`] when `buf` is not a single well-formed JSON
    /// value, including when trailing non-whitespace content follows it.
    #[inline]
    pub fn parse(buf: &'a mut [u8]) -> Result<Self, simd_json::Error> {
        Ok(Self {
            tape: simd_json::to_tape(buf)?,
        })
    }

    /// Parse `buf` in place, reusing the caller-owned parse buffers.
    ///
    /// Reusing one [`Buffers`] value across frames avoids reallocating the
    /// internal string and structural-index storage on every message.
    ///
    /// # Errors
    ///
    /// Returns [`simd_json::Error`] when `buf` is not a single well-formed JSON
    /// value.
    #[inline]
    pub fn parse_with_buffers(
        buf: &'a mut [u8],
        buffers: &mut Buffers,
    ) -> Result<Self, simd_json::Error> {
        Ok(Self {
            tape: simd_json::to_tape_with_buffers(buf, buffers)?,
        })
    }

    /// Walk `path` from the document root.
    ///
    /// An empty path selects the root value. Returns `None` as soon as a key is
    /// absent, an index is out of bounds, or a segment is applied to a value of
    /// the wrong shape.
    fn walk<'t>(&'t self, path: &[PathSegment<'_>]) -> Option<Value<'t, 'a>>
    where
        'a: 't,
    {
        let mut current = self.tape.as_value();
        for segment in path {
            current = match *segment {
                PathSegment::Key(name) => current.get(name)?,
                PathSegment::Index(index) => current.get_idx(index)?,
            };
        }
        Some(current)
    }

    /// Read a string at `path`, borrowed directly from the input buffer.
    ///
    /// Returns `None` when the path does not resolve or the value is not a
    /// JSON string.
    #[inline]
    #[must_use]
    pub fn get_str(&self, path: &[PathSegment<'_>]) -> Option<&'a str> {
        // `ValueAsScalar::as_str` ties the returned reference to the temporary
        // `Value`; `ValueIntoString` consumes it and yields the `'input`
        // lifetime, which is the caller's buffer.
        ValueIntoString::into_string(self.walk(path)?)
    }

    /// Read an unsigned 64-bit integer at `path`.
    #[inline]
    #[must_use]
    pub fn get_u64(&self, path: &[PathSegment<'_>]) -> Option<u64> {
        self.walk(path)?.as_u64()
    }

    /// Read a signed 64-bit integer at `path`.
    #[inline]
    #[must_use]
    pub fn get_i64(&self, path: &[PathSegment<'_>]) -> Option<i64> {
        self.walk(path)?.as_i64()
    }

    /// Read a 64-bit floating-point number at `path`.
    #[inline]
    #[must_use]
    pub fn get_f64(&self, path: &[PathSegment<'_>]) -> Option<f64> {
        self.walk(path)?.as_f64()
    }

    /// Read a boolean at `path`.
    #[inline]
    #[must_use]
    pub fn get_bool(&self, path: &[PathSegment<'_>]) -> Option<bool> {
        self.walk(path)?.as_bool()
    }

    /// Return `true` when the value at `path` is JSON `null`.
    ///
    /// A missing path is not the same as a present `null`; this returns
    /// `false` for missing paths.
    #[inline]
    #[must_use]
    pub fn is_null(&self, path: &[PathSegment<'_>]) -> bool {
        self.walk(path).and_then(|value| value.as_null()).is_some()
    }

    /// Return `true` when the object selected by `path` contains `name`.
    #[inline]
    #[must_use]
    pub fn contains_key(&self, path: &[PathSegment<'_>], name: &str) -> bool {
        self.walk(path)
            .is_some_and(|value| value.contains_key(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segment_constructors_match_conversions() {
        assert_eq!(PathSegment::key("bids"), PathSegment::from("bids"));
        assert_eq!(PathSegment::index(2), PathSegment::from(2usize));
        assert_eq!(PathSegment::index(0), PathSegment::Index(0));
    }

    #[test]
    fn reusing_buffers_across_frames_yields_the_same_view() {
        let mut buffers = Buffers::default();
        let mut first = br#"{"s":"BTCUSDT","n":1}"#.to_vec();
        {
            let frame = BorrowedJsonFrame::parse_with_buffers(&mut first, &mut buffers)
                .expect("first frame parses");
            assert_eq!(frame.get_str(&[PathSegment::key("s")]), Some("BTCUSDT"));
            assert_eq!(frame.get_u64(&[PathSegment::key("n")]), Some(1));
        }

        let mut second = br#"{"s":"ETHUSDT","n":2}"#.to_vec();
        let frame = BorrowedJsonFrame::parse_with_buffers(&mut second, &mut buffers)
            .expect("second frame parses");
        assert_eq!(frame.get_str(&[PathSegment::key("s")]), Some("ETHUSDT"));
        assert_eq!(frame.get_u64(&[PathSegment::key("n")]), Some(2));
    }
}
