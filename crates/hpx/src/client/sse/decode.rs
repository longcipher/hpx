//! Zero-I/O SSE state-machine parser.
//!
//! Vendored from [sse-core](https://github.com/PizzasBear/sse-rs) and adapted for hpx.

use std::{
    borrow::Cow,
    fmt, iter,
    num::{NonZeroU8, NonZeroUsize},
    str,
    sync::Arc,
};

use bytes::Buf;
use memchr::{memchr, memchr2};

/// Represents a single Server-Sent Event message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageEvent {
    /// The event name (defaults to `"message"`).
    pub event: Cow<'static, str>,
    /// The payload data.
    pub data: String,
    /// The `Last-Event-ID` sent by the server, if any.
    pub last_event_id: Option<Arc<str>>,
}

/// Commands and payloads yielded by the SSE stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    /// A standard data message.
    Message(MessageEvent),
    /// A server request to change the client's reconnect time (in milliseconds).
    Retry(u32),
}

/// Error indicating that a parsed field exceeded the maximum allowed buffer size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct PayloadTooLargeError;

impl fmt::Display for PayloadTooLargeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("payload exceeded the allotted buffer size limit")
    }
}

impl std::error::Error for PayloadTooLargeError {}

const MAX_DEBUG_SIZE: usize = 200;

struct ShowBigStr<'a>(&'a str);

impl fmt::Debug for ShowBigStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut end = self.0.len().min(MAX_DEBUG_SIZE);
        while !self.0.is_char_boundary(end) {
            end -= 1;
        }
        let s = &self.0[..end];

        fmt::Debug::fmt(s, f)?;
        if end < self.0.len() {
            write!(f, "... ({} bytes total)", self.0.len())?;
        }

        Ok(())
    }
}

struct ShowBigBuf<'a>(&'a [u8]);

impl fmt::Debug for ShowBigBuf<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (buf, truncated) = match self.0.len() {
            ..=MAX_DEBUG_SIZE => (self.0, false),
            _ => (&self.0[..MAX_DEBUG_SIZE], true),
        };

        let mut chunks = buf.utf8_chunks().peekable();

        f.write_str("\"")?;
        while let Some(chunk) = chunks.next() {
            fmt::Display::fmt(&chunk.valid().escape_debug(), f)?;

            let invalid = chunk.invalid();
            if invalid.is_empty() {
                continue;
            }

            // If we truncated and this is the very last chunk, the invalid bytes
            // are almost certainly just a sliced multi-byte UTF-8 character.
            if truncated && chunks.peek().is_none() {
                break;
            }

            for &byte in invalid {
                write!(f, "\\x{byte:02X}")?;
            }
        }

        if truncated {
            write!(f, "\"... ({} bytes total)", self.0.len())?;
        } else {
            f.write_str("\"")?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum ValueMode {
    Data,
    Event,
    Retry,
    Id,
}

impl ValueMode {
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Event => "event",
            Self::Retry => "retry",
            Self::Id => "id",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Bom { bytes_read: u8 },
    Field(Option<(ValueMode, NonZeroU8)>),
    Value(ValueMode),
    Ignore,
    PostCr,
    PostColon(ValueMode),
}

/// The core state-machine parser for SSE.
///
/// This decoder does not perform any I/O. It consumes bytes from a given buffer
/// and yields parsed [`SseEvent`]s.
#[derive(Clone)]
pub struct SseDecoder {
    mode: Mode,
    last_event_id: Option<Arc<str>>,
    staged_last_event_id: Option<Arc<str>>,
    last_event_id_buf: Vec<u8>,
    event_buf: Vec<u8>,
    data_buf: Vec<u8>,
    retry_buf: Option<u32>,
    max_payload_size: NonZeroUsize,
    corrupted: bool,
}

impl SseDecoder {
    /// Creates a new decoder with the default payload size limit of 512KiB.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::with_limit(NonZeroUsize::new(512 * 1024).unwrap())
    }

    /// Creates a new decoder with a custom maximum payload size limit.
    #[inline]
    #[must_use]
    pub fn with_limit(max_payload_size: NonZeroUsize) -> Self {
        Self {
            mode: Mode::Bom { bytes_read: 0 },
            last_event_id: None,
            staged_last_event_id: None,
            last_event_id_buf: vec![],
            event_buf: vec![],
            data_buf: vec![],
            retry_buf: None,
            max_payload_size,
            corrupted: false,
        }
    }

    /// Returns the current `Last-Event-ID` known to the decoder, if any.
    #[inline]
    #[must_use]
    pub fn last_event_id(&self) -> Option<&Arc<str>> {
        self.last_event_id.as_ref()
    }

    /// Resets the decoder state for a new connection, explicitly overriding
    /// the currently tracked `Last-Event-ID`.
    #[inline]
    pub fn reconnect_with_id(&mut self, id: Option<Arc<str>>) {
        self.last_event_id = id;
        self.reconnect();
    }

    /// Resets the decoder state completely, dropping the current `Last-Event-ID`.
    #[inline]
    pub fn clear(&mut self) {
        self.reconnect_with_id(None);
    }

    /// Resets the buffer state for a new connection while retaining the `Last-Event-ID`.
    #[inline]
    pub fn reconnect(&mut self) {
        self.mode = Mode::Bom { bytes_read: 0 };
        self.clear_bufs();
        self.corrupted = false;
    }

    fn mark_corrupted(&mut self) {
        self.clear_bufs();
        self.corrupted = true;
    }

    #[inline]
    fn clear_bufs(&mut self) {
        self.data_buf.clear();
        self.event_buf.clear();
        self.last_event_id_buf.clear();
        self.staged_last_event_id = self.last_event_id.clone();
    }

    fn dispatch(&mut self, cr: bool) -> Option<SseEvent> {
        self.mode = match cr {
            true => Mode::PostCr,
            false => Mode::Field(None),
        };

        if self.corrupted {
            self.corrupted = false;
            return None;
        }

        self.last_event_id = self.staged_last_event_id.clone();

        match self.data_buf.last() {
            Some(b'\n') => {
                self.data_buf.pop();
            }
            Some(_) => {}
            None => {
                self.event_buf.clear();
                return None;
            }
        }

        let data = String::from_utf8_lossy(&self.data_buf).into_owned();
        self.data_buf.clear();

        let event = match &*self.event_buf {
            b"" => Cow::Borrowed("message"),
            event_buf => Cow::Owned(String::from_utf8_lossy(event_buf).into_owned()),
        };
        self.event_buf.clear();

        Some(SseEvent::Message(MessageEvent {
            data,
            event,
            last_event_id: self.last_event_id.clone(),
        }))
    }

    /// Consumes bytes from the provided buffer and attempts to yield an event.
    ///
    /// If `Ok(None)` is returned, the buffer is exhausted and more bytes are needed.
    pub fn next(&mut self, buf: &mut impl Buf) -> Option<Result<SseEvent, PayloadTooLargeError>> {
        loop {
            let chunk = buf.chunk();
            if chunk.is_empty() {
                return None;
            }

            match &mut self.mode {
                Mode::Bom { bytes_read } => {
                    let b0 = chunk[0];

                    const BOM: &[u8; 3] = b"\xef\xbb\xbf";

                    if b0 != BOM[*bytes_read as usize] {
                        self.mode = match *bytes_read {
                            0 => Mode::Field(None),
                            _ => Mode::Ignore,
                        };
                        continue;
                    }

                    buf.advance(1);
                    *bytes_read += 1;

                    if BOM.len() <= *bytes_read as usize {
                        self.mode = Mode::Field(None);
                    }
                }
                Mode::Field(None) => {
                    let b0 = chunk[0];
                    buf.advance(1);
                    let mode = match b0 {
                        b'd' if !self.corrupted => ValueMode::Data,
                        b'e' if !self.corrupted => ValueMode::Event,
                        b'i' if !self.corrupted => ValueMode::Id,
                        b'r' => ValueMode::Retry,

                        b'\n' | b'\r' => match self.dispatch(b0 == b'\r') {
                            Some(ev) => return Some(Ok(ev)),
                            None => continue,
                        },

                        _ => {
                            self.mode = Mode::Ignore;
                            continue;
                        }
                    };
                    self.mode = Mode::Field(Some((mode, NonZeroU8::new(1).unwrap())));
                }
                &mut Mode::Field(Some((mode, ref mut len))) => {
                    let cmp = &mode.field_name().as_bytes()[len.get() as usize..];
                    if iter::zip(chunk, cmp).any(|(ch0, ch1)| ch0 != ch1) {
                        self.mode = Mode::Ignore;
                        continue;
                    }
                    let Some(&b_post) = chunk.get(cmp.len()) else {
                        *len = NonZeroU8::new(len.get() + chunk.len() as u8).unwrap();
                        buf.advance(chunk.len());
                        continue;
                    };
                    buf.advance(cmp.len() + 1);

                    match b_post {
                        b'\n' => self.mode = Mode::Field(None),
                        b'\r' => self.mode = Mode::PostCr,
                        b':' => {
                            match mode {
                                ValueMode::Data => {}
                                ValueMode::Event => self.event_buf.clear(),
                                ValueMode::Id => self.last_event_id_buf.clear(),
                                ValueMode::Retry => self.retry_buf = None,
                            }

                            self.mode = Mode::PostColon(mode);
                            continue;
                        }
                        _ => {
                            self.mode = Mode::Ignore;
                            continue;
                        }
                    }

                    match mode {
                        ValueMode::Data => self.data_buf.push(b'\n'),
                        ValueMode::Id => self.last_event_id_buf.clear(),
                        ValueMode::Event | ValueMode::Retry => {}
                    }
                }
                Mode::Value(ValueMode::Retry) => {
                    let mut advanced = 0;
                    let mut return_event = false;

                    for &b in chunk {
                        advanced += 1;
                        match b {
                            b'0'..=b'9' => {
                                let digit = (b & 0xf) as _;

                                let retry_buf = self.retry_buf.unwrap_or(0);
                                let Some(retry_buf) = retry_buf.checked_mul(10) else {
                                    self.mode = Mode::Ignore;
                                    break;
                                };
                                let Some(retry_buf) = retry_buf.checked_add(digit) else {
                                    self.mode = Mode::Ignore;
                                    break;
                                };
                                self.retry_buf = Some(retry_buf);
                            }
                            b'\r' => {
                                self.mode = Mode::PostCr;
                                return_event = true;
                                break;
                            }
                            b'\n' => {
                                self.mode = Mode::Field(None);
                                return_event = true;
                                break;
                            }
                            _ => {
                                self.mode = Mode::Ignore;
                                break;
                            }
                        }
                    }

                    buf.advance(advanced);

                    if let (true, Some(retry_buf)) = (return_event, self.retry_buf) {
                        return Some(Ok(SseEvent::Retry(retry_buf)));
                    }
                }
                Mode::Value(ValueMode::Data) => {
                    match consume_until_newline(
                        &mut self.mode,
                        Some(&mut self.data_buf),
                        self.max_payload_size,
                        buf,
                    ) {
                        Ok(true) => self.data_buf.push(b'\n'),
                        Ok(false) => {}
                        Err(err) => {
                            self.mark_corrupted();
                            return Some(Err(err));
                        }
                    }
                }
                Mode::Value(ValueMode::Event) => {
                    if let Err(err) = consume_until_newline(
                        &mut self.mode,
                        Some(&mut self.event_buf),
                        self.max_payload_size,
                        buf,
                    ) {
                        self.mark_corrupted();
                        return Some(Err(err));
                    }
                }
                Mode::Value(ValueMode::Id) => {
                    match consume_until_newline(
                        &mut self.mode,
                        Some(&mut self.last_event_id_buf),
                        self.max_payload_size,
                        buf,
                    ) {
                        Ok(true) => {
                            if memchr(0, &self.last_event_id_buf).is_none() {
                                self.staged_last_event_id = match &*self.last_event_id_buf {
                                    [] => None,
                                    buf => Some(String::from_utf8_lossy(buf).into()),
                                };
                            }
                            self.last_event_id_buf.clear();
                        }
                        Ok(false) => {}
                        Err(err) => {
                            self.mark_corrupted();
                            return Some(Err(err));
                        }
                    }
                }
                Mode::Ignore => {
                    consume_until_newline(&mut self.mode, None, self.max_payload_size, buf)
                        .expect("there should be no payload to grow too large");
                }
                Mode::PostCr => {
                    if chunk[0] == b'\n' {
                        buf.advance(1);
                    }
                    self.mode = Mode::Field(None);
                }
                Mode::PostColon(value) => {
                    if chunk[0] == b' ' {
                        buf.advance(1);
                    }
                    self.mode = Mode::Value(*value);
                }
            }
        }
    }
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SseDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SseDecoder")
            .field("mode", &self.mode)
            .field(
                "last_event_id",
                &self.last_event_id.as_deref().map(ShowBigStr),
            )
            .field(
                "staged_last_event_id",
                &self.staged_last_event_id.as_deref().map(ShowBigStr),
            )
            .field("last_event_id_buf", &ShowBigBuf(&self.last_event_id_buf))
            .field("event_buf", &ShowBigBuf(&self.event_buf))
            .field("data_buf", &ShowBigBuf(&self.data_buf))
            .field("retry_buf", &self.retry_buf)
            .field("max_payload_size", &self.max_payload_size)
            .finish()
    }
}

fn consume_until_newline(
    mode: &mut Mode,
    mut out: Option<&mut Vec<u8>>,
    max_size: NonZeroUsize,
    buf: &mut impl Buf,
) -> Result<bool, PayloadTooLargeError> {
    loop {
        let chunk = buf.chunk();
        if chunk.is_empty() {
            return Ok(false);
        };

        let Some(i) = memchr2(b'\r', b'\n', chunk) else {
            if let Some(out) = out.as_deref_mut() {
                if max_size.get() < out.len() + chunk.len() {
                    out.clear();
                    *mode = Mode::Ignore;
                    return Err(PayloadTooLargeError);
                }
                out.extend_from_slice(chunk);
            }
            buf.advance(chunk.len());
            continue;
        };

        if let Some(out) = out.as_deref_mut() {
            if max_size.get() < out.len() + i {
                out.clear();
                *mode = Mode::Ignore;
                return Err(PayloadTooLargeError);
            }
            out.extend_from_slice(&chunk[..i]);
        }

        *mode = match chunk[i] {
            b'\r' => Mode::PostCr,
            b'\n' => Mode::Field(None),
            _ => unreachable!(),
        };

        buf.advance(i + 1);

        return Ok(true);
    }
}

#[cfg(test)]
mod tests {
    use core::slice;

    use super::*;

    #[test]
    fn hard_parse() -> Result<(), PayloadTooLargeError> {
        let bytes = "\u{FEFF}data: x

:

event: my-event\r
data:line1
data: line2
data
:
id: my-id
:should be ignored too\rretry:42
retry:

data:second

data:ignored
";

        let mut decoder = SseDecoder::new();

        let events = bytes
            .bytes()
            .filter_map(|b| decoder.next(&mut slice::from_ref(&b)))
            .collect::<Result<Vec<_>, PayloadTooLargeError>>()?;

        let id = Some("my-id".into());

        assert_eq!(
            events,
            &[
                SseEvent::Message(MessageEvent {
                    event: "message".into(),
                    data: "x".into(),
                    last_event_id: None
                }),
                SseEvent::Retry(42),
                SseEvent::Message(MessageEvent {
                    event: "my-event".into(),
                    data: "line1\nline2\n".into(),
                    last_event_id: id.clone()
                }),
                SseEvent::Message(MessageEvent {
                    event: "message".into(),
                    data: "second".into(),
                    last_event_id: id.clone()
                })
            ]
        );
        Ok(())
    }

    #[test]
    fn test_reconnect() {
        let mut stream1: &[u8] = b"
id: my-id

event: my-event
data:line1
:
data: line2
id: ignored1
";

        let mut stream2: &[u8] = b"

data: data

id: final

id: ignored2
";

        let my_id = Some("my-id".into());

        let mut decoder = SseDecoder::new();

        assert_eq!(decoder.next(&mut stream1), None);
        assert_eq!(decoder.last_event_id(), my_id.as_ref());
        assert!(stream1.is_empty());

        decoder.reconnect();

        assert_eq!(
            decoder.next(&mut stream2),
            Some(Ok(SseEvent::Message(MessageEvent {
                event: "message".into(),
                data: "data".into(),
                last_event_id: my_id,
            }))),
        );
        assert_eq!(decoder.next(&mut stream2), None);
        assert_eq!(decoder.last_event_id().map(|id| &**id), Some("final"));
        assert!(stream2.is_empty());
    }

    #[test]
    fn test_limits() {
        let mut stream: &[u8] = b"
data: 0123456789
id: my-id

id: 01234567890
data: thing
event: ev

data: mid

event: jojo
id: ignored
data: 01234
data: 56789
retry: 10

event: final
data

";

        let my_id = Some("my-id".into());

        let mut decoder = SseDecoder::with_limit(NonZeroUsize::new(10).unwrap());

        assert_eq!(
            decoder.next(&mut stream),
            Some(Ok(SseEvent::Message(MessageEvent {
                event: "message".into(),
                data: "0123456789".into(),
                last_event_id: my_id.clone(),
            })))
        );
        assert_eq!(decoder.next(&mut stream), Some(Err(PayloadTooLargeError)));
        assert_eq!(
            decoder.next(&mut stream),
            Some(Ok(SseEvent::Message(MessageEvent {
                event: "message".into(),
                data: "mid".into(),
                last_event_id: my_id.clone()
            })))
        );
        assert_eq!(decoder.next(&mut stream), Some(Err(PayloadTooLargeError)));
        assert_eq!(decoder.next(&mut stream), Some(Ok(SseEvent::Retry(10))));
        assert_eq!(
            decoder.next(&mut stream),
            Some(Ok(SseEvent::Message(MessageEvent {
                event: "final".into(),
                data: "".into(),
                last_event_id: my_id.clone()
            })))
        );
        assert!(stream.is_empty());
    }
}
