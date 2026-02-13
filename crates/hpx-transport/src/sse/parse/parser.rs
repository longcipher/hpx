//! UTF-8 agnostic parser implementation for SSE.

use core::str::Utf8Error;

use bytes::{Buf, Bytes, BytesMut};
use bytes_utils::Str;

use super::constants::{CR, LF};

/// A full line from an SSE stream.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RawEventLine<'a> {
    /// Comment line (starts with `:`)
    Comment,
    /// A field line with optional value.
    Field {
        field_name: &'a [u8],
        field_value: Option<&'a [u8]>,
    },
    /// An empty line (event delimiter).
    Empty,
}

/// Full line from an SSE stream, owned version of [`RawEventLine`].
#[derive(Debug, Clone)]
pub(crate) enum RawEventLineOwned {
    /// Comment line.
    Comment,
    /// An empty line.
    Empty,
    /// A field line with optional value.
    Field {
        field_name: Bytes,
        field_value: Option<Bytes>,
    },
}

/// Valid field names per the
/// [spec](https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation).
#[derive(Debug, Clone, Copy)]
pub(crate) enum FieldName {
    Event,
    Data,
    Id,
    Retry,
    Ignored,
}

/// Completely parsed SSE event line.
#[derive(Debug, Clone)]
pub(crate) enum ValidatedEventLine {
    Comment,
    Empty,
    Field {
        field_name: FieldName,
        field_value: Option<Str>,
    },
}

fn validate_bytes(val: Bytes) -> Result<Str, Utf8Error> {
    match str::from_utf8(val.as_ref()) {
        Ok(_) => {
            // Safety: we just validated the bytes are valid UTF-8.
            Ok(unsafe { Str::from_inner_unchecked(val) })
        }
        Err(e) => Err(e),
    }
}

impl RawEventLineOwned {
    pub(crate) fn validate(self) -> Result<ValidatedEventLine, core::str::Utf8Error> {
        match self {
            Self::Comment => Ok(ValidatedEventLine::Comment),
            Self::Empty => Ok(ValidatedEventLine::Empty),
            Self::Field {
                field_name,
                field_value,
            } => {
                let field_name = match field_name.as_ref() {
                    b"event" => FieldName::Event,
                    b"data" => FieldName::Data,
                    b"id" => FieldName::Id,
                    b"retry" => FieldName::Retry,
                    _ => FieldName::Ignored,
                };

                let field_value = match field_value {
                    Some(b) => Some(validate_bytes(b)?),
                    None => None,
                };

                Ok(ValidatedEventLine::Field {
                    field_name,
                    field_value,
                })
            }
        }
    }
}

/// Finds the next end-of-line in `bytes`.
///
/// Returns `(line_end, remainder_start)` — the non-inclusive end of the
/// line and the inclusive start of the remainder.  Returns `None` if
/// more data is needed (e.g. buffer ends with a lone CR that could be
/// part of a CRLF pair).
fn find_eol(bytes: &[u8]) -> Option<(usize, usize)> {
    let first_match = memchr::memchr2(CR, LF, bytes)?;

    match bytes[first_match] {
        LF => Some((first_match, first_match + 1)),
        CR => {
            if first_match + 1 >= bytes.len() {
                return None; // need more data to see if it's CRLF or just CR
            }

            if bytes[first_match + 1] == LF {
                Some((first_match, first_match + 2))
            } else {
                Some((first_match, first_match + 1))
            }
        }
        _ => unreachable!(),
    }
}

fn read_line(bytes: &[u8]) -> RawEventLine<'_> {
    match memchr::memchr(b':', bytes) {
        Some(colon_pos) => {
            if colon_pos == 0 {
                RawEventLine::Comment
            } else {
                let value = &bytes[colon_pos + 1..];
                // Strip single leading space if present.
                let value = match value {
                    [b' ', rest @ ..] => rest,
                    _ => value,
                };
                RawEventLine::Field {
                    field_name: &bytes[..colon_pos],
                    field_value: Some(value),
                }
            }
        }
        None => {
            if bytes.is_empty() {
                RawEventLine::Empty
            } else {
                RawEventLine::Field {
                    field_name: bytes,
                    field_value: None,
                }
            }
        }
    }
}

/// Reads the next [`RawEventLineOwned`] from the buffer, then advances the
/// buffer past the corresponding EOL.
///
/// Returns `None` if the buffer contains no complete line terminator.
pub(crate) fn parse_line_from_buffer(buffer: &mut BytesMut) -> Option<RawEventLineOwned> {
    let (line_end, rem_start) = find_eol(buffer)?;

    let line = buffer.split_to(line_end).freeze();
    buffer.advance(rem_start - line_end);

    match read_line(&line) {
        RawEventLine::Field {
            field_name,
            field_value,
        } => {
            let field_name = line.slice_ref(field_name);
            let field_value = field_value.map(|field_value| line.slice_ref(field_value));

            Some(RawEventLineOwned::Field {
                field_name,
                field_value,
            })
        }
        RawEventLine::Comment => Some(RawEventLineOwned::Comment),
        RawEventLine::Empty => Some(RawEventLineOwned::Empty),
    }
}
