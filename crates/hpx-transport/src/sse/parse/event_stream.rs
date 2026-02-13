//! [`Stream`] that converts a stream of
//! [`Bytes`](bytes::Bytes) chunks into [`Event`]s.

use core::{
    pin::Pin,
    task::{Context, Poll, ready},
    time::Duration,
};

use bytes::{Buf, BufMut, BytesMut};
use bytes_utils::{Str, StrMut};
use futures_core::Stream;

use super::{
    constants::{BOM, CR, EMPTY_STR, LF, MESSAGE_STR},
    errors::EventStreamError,
    event::Event,
    parser::{FieldName, RawEventLineOwned, ValidatedEventLine, parse_line_from_buffer},
};

// ---------------------------------------------------------------------------
// EventBuilder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct EventBuilder {
    event: Str,
    id: Str,
    data_buffer: EventBuilderDataBuffer,
    retry: Option<Duration>,
    is_complete: bool,
}

/// Optimised data buffer.
///
/// The common case is a single `data:` line per event, so we keep the first
/// value as an immutable [`Str`] and only upgrade to a mutable buffer when a
/// second `data:` line arrives.
#[derive(Debug, Default, Clone)]
enum EventBuilderDataBuffer {
    #[default]
    Uninit,
    Immutable(Str),
    Mutable(StrMut),
}

impl EventBuilderDataBuffer {
    fn freeze(self) -> Str {
        match self {
            Self::Uninit => EMPTY_STR,
            Self::Immutable(s) => s,
            Self::Mutable(s) => s.freeze(),
        }
    }

    fn push_str(&mut self, s: Str) {
        match self {
            Self::Uninit => *self = Self::Immutable(s),
            Self::Immutable(immutable_buf) => {
                let len = immutable_buf.len() + s.len();
                let inner = BytesMut::with_capacity(len);
                // Safety: The buffer is empty — there are no bytes to be
                // invalid.
                let mut buf = unsafe { StrMut::from_inner_unchecked(inner) };
                buf.push_str(immutable_buf);
                buf.push('\n');
                buf.push_str(&s);
                *self = Self::Mutable(buf);
            }
            Self::Mutable(mutable_buf) => {
                mutable_buf.push('\n');
                mutable_buf.push_str(&s);
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::Uninit)
    }
}

impl Default for EventBuilder {
    fn default() -> Self {
        Self {
            event: EMPTY_STR,
            id: EMPTY_STR,
            data_buffer: EventBuilderDataBuffer::default(),
            retry: None,
            is_complete: false,
        }
    }
}

impl EventBuilder {
    fn add(&mut self, line: ValidatedEventLine) {
        match line {
            ValidatedEventLine::Empty => self.is_complete = true,
            ValidatedEventLine::Field {
                field_name: FieldName::Event,
                field_value: Some(field_value),
            } => {
                self.event = field_value;
            }
            ValidatedEventLine::Field {
                field_name: FieldName::Data,
                field_value,
            } => {
                let field_value = field_value.unwrap_or(EMPTY_STR);
                self.data_buffer.push_str(field_value);
            }
            ValidatedEventLine::Field {
                field_name: FieldName::Id,
                field_value,
            } => {
                let no_null_byte = field_value
                    .as_ref()
                    .map(|field_value| memchr::memchr(0, field_value.as_bytes()).is_none())
                    .unwrap_or(true);

                if no_null_byte {
                    self.id = field_value.unwrap_or(EMPTY_STR);
                }
            }
            ValidatedEventLine::Field {
                field_name: FieldName::Retry,
                field_value,
            } => {
                if let Some(Ok(val)) = field_value.map(|val| val.parse()) {
                    self.retry = Some(Duration::from_millis(val));
                }
            }
            // Comments and unknown fields are silently ignored.
            ValidatedEventLine::Comment
            | ValidatedEventLine::Field {
                field_name: FieldName::Ignored,
                ..
            }
            | ValidatedEventLine::Field {
                field_name: FieldName::Event,
                field_value: None,
            } => (),
        }
    }

    /// Dispatch a complete event (HTML spec §9.2.6 steps 1–8).
    #[must_use]
    fn dispatch(&mut self) -> Option<Event> {
        let EventBuilder {
            mut event,
            id,
            data_buffer,
            retry,
            ..
        } = core::mem::take(self);
        // Preserve `id` for the next event.
        self.id = id.clone();

        if data_buffer.is_empty() {
            return None;
        }

        if event.is_empty() {
            event = MESSAGE_STR;
        }

        Some(Event {
            event,
            data: data_buffer.freeze(),
            id,
            retry,
        })
    }
}

// ---------------------------------------------------------------------------
// EventStreamState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum EventStreamState {
    NotStarted,
    Started,
    Terminated,
}

impl EventStreamState {
    fn is_terminated(self) -> bool {
        matches!(self, Self::Terminated)
    }

    fn is_not_started(self) -> bool {
        matches!(self, Self::NotStarted)
    }
}

// ---------------------------------------------------------------------------
// EventStream
// ---------------------------------------------------------------------------

pin_project_lite::pin_project! {
    /// A [`Stream`] that converts a stream of byte chunks into parsed SSE
    /// [`Event`]s.
    ///
    /// Handles BOM detection, line-ending normalisation (LF / CR / CRLF),
    /// field parsing, and event dispatch according to the HTML spec.
    #[project = EventStreamProjection]
    #[derive(Debug)]
    pub struct EventStream<S> {
        #[pin]
        stream: S,
        buffer: BytesMut,
        builder: EventBuilder,
        state: EventStreamState,
        last_event_id: Str,
    }
}

impl<S> EventStream<S> {
    /// Create a new [`EventStream`] from an underlying byte stream.
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buffer: BytesMut::new(),
            builder: EventBuilder::default(),
            state: EventStreamState::NotStarted,
            last_event_id: EMPTY_STR,
        }
    }

    /// Set the last event ID (useful for resumability).
    pub fn set_last_event_id(&mut self, id: impl Into<Str>) {
        self.last_event_id = id.into();
    }

    /// Reference to the last event ID given out by this stream.
    pub fn last_event_id(&self) -> &Str {
        &self.last_event_id
    }

    /// Take the current buffer (useful for checking leftovers).
    pub fn take_buffer(self) -> BytesMut {
        self.buffer
    }
}

// ---------------------------------------------------------------------------
// BOM helper
// ---------------------------------------------------------------------------

const fn starts_with_bom(buf: &[u8]) -> Option<bool> {
    match buf.len() {
        0 => None,
        1 => {
            if buf[0] == BOM[0] {
                None
            } else {
                Some(false)
            }
        }
        2 => {
            if buf[0] == BOM[0] && buf[1] == BOM[1] {
                None
            } else {
                Some(false)
            }
        }
        _gte_3 => {
            if buf[0] == BOM[0] && buf[1] == BOM[1] && buf[2] == BOM[2] {
                Some(true)
            } else {
                Some(false)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing helper
// ---------------------------------------------------------------------------

fn parse_event<E>(
    buffer: &mut BytesMut,
    builder: &mut EventBuilder,
) -> Result<Option<Event>, EventStreamError<E>> {
    if buffer.is_empty() {
        return Ok(None);
    }
    loop {
        let event_line = match parse_line_from_buffer(buffer).map(RawEventLineOwned::validate) {
            Some(Ok(event_line)) => event_line,
            Some(Err(e)) => return Err(EventStreamError::Utf8Error(e)),
            None => return Ok(None),
        };

        builder.add(event_line);

        #[allow(clippy::collapsible_if)]
        if builder.is_complete {
            if let Some(event) = builder.dispatch() {
                return Ok(Some(event));
            }
        }
    }
}

macro_rules! try_parse_event_buffer {
    ($this:ident) => {
        match parse_event($this.buffer, $this.builder) {
            Ok(Some(event)) => {
                *$this.last_event_id = event.id.clone();
                return Poll::Ready(Some(Ok(event)));
            }
            Err(e) => return Poll::Ready(Some(Err(e))),
            _ => {}
        }
    };
}

// ---------------------------------------------------------------------------
// Stream implementation
// ---------------------------------------------------------------------------

impl<S, E, B> Stream for EventStream<S>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    type Item = Result<Event, EventStreamError<E>>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<<Self as Stream>::Item>> {
        let mut this = self.project();

        try_parse_event_buffer!(this);

        if this.state.is_terminated() {
            return Poll::Ready(None);
        };

        loop {
            let new_bytes = match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(Ok(o)) => o,
                Some(Err(e)) => return Poll::Ready(Some(Err(EventStreamError::Transport(e)))),
                None => {
                    *this.state = EventStreamState::Terminated;
                    // The parser waits to see if a line is CR LF or just CR —
                    // once the stream ends we know a trailing CR is standalone.
                    if this
                        .buffer
                        .last()
                        .map(|&last| last == CR)
                        .unwrap_or_default()
                    {
                        this.buffer.put_u8(LF);
                    }

                    try_parse_event_buffer!(this);
                    return Poll::Ready(None);
                }
            };

            let new_bytes = new_bytes.as_ref();

            if new_bytes.is_empty() {
                continue;
            }

            this.buffer.extend_from_slice(new_bytes);

            // BOM detection on the very first chunk(s).
            if this.state.is_not_started() {
                match starts_with_bom(this.buffer) {
                    Some(true) => {
                        *this.state = EventStreamState::Started;
                        this.buffer.advance(BOM.len());
                    }
                    Some(false) => *this.state = EventStreamState::Started,
                    None => continue,
                }
            };

            try_parse_event_buffer!(this);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures_util::StreamExt;

    use super::*;

    #[tokio::test]
    async fn valid_data_fields() {
        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello, world!\n\n")
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("Hello, world!"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b"data: Hello,")),
                Ok::<_, ()>(Bytes::from_static(b" world!\n\n"))
            ]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("Hello, world!"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b"data: Hello,")),
                Ok::<_, ()>(Bytes::from_static(b"")),
                Ok::<_, ()>(Bytes::from_static(b" world!\n\n"))
            ]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("Hello, world!"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello, world!\n")
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello,\ndata: world!\n\n")
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("Hello,\nworld!"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: Hello,\n\ndata: world!\n\n")
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("Hello,"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("world!"),
                    id: EMPTY_STR,
                    retry: None,
                }
            ]
        );
    }

    #[tokio::test]
    async fn spec_examples() {
        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"data: This is the first message.

data: This is the second message, it
data: has two lines.

data: This is the third message.

"
                )
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("This is the first message."),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("This is the second message, it\nhas two lines."),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("This is the third message."),
                    id: EMPTY_STR,
                    retry: None,
                }
            ]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"event: add
data: 73857293

event: remove
data: 2153

event: add
data: 113411

"
                )
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![
                Event {
                    event: Str::from_static("add"),
                    data: Str::from_static("73857293"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("remove"),
                    data: Str::from_static("2153"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("add"),
                    data: Str::from_static("113411"),
                    id: EMPTY_STR,
                    retry: None,
                }
            ]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"data: YHOO
data: +2
data: 10

"
                )
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("YHOO\n+2\n10"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b": test stream

data: first event
id: 1

data:second event
id

data:  third event

"
                )
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    id: Str::from_static("1"),
                    data: Str::from_static("first event"),
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("second event"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static(" third event"),
                    id: EMPTY_STR,
                    retry: None,
                }
            ]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"data

data
data

data:
"
                )
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: EMPTY_STR,
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("\n"),
                    id: EMPTY_STR,
                    retry: None,
                },
            ]
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(
                    b"data:test

data: test

"
                )
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("test"),
                    id: EMPTY_STR,
                    retry: None,
                },
                Event {
                    event: Str::from_static("message"),
                    data: Str::from_static("test"),
                    id: EMPTY_STR,
                    retry: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn bom_handling() {
        // BOM at start should be stripped.
        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"\xEF\xBB\xBFdata: test\n\n")
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        // BOM split across chunks.
        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b"\xEF\xBB")),
                Ok::<_, ()>(Bytes::from_static(b"\xBFdata: test\n\n"))
            ]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        // Short first line without BOM.
        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![
                Ok::<_, ()>(Bytes::from_static(b":\n")),
                Ok::<_, ()>(Bytes::from_static(b"data: test\n\n"))
            ]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );

        // No BOM.
        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: test\n\n")
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );
    }

    #[tokio::test]
    async fn trailing_cr_handling() {
        // Stream ending with CR should be treated as complete line.
        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: test\r")
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![] // No complete event without the empty line.
        );

        assert_eq!(
            EventStream::new(futures_util::stream::iter(vec![Ok::<_, ()>(
                Bytes::from_static(b"data: test\r\r")
            )]))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>(),
            vec![Event {
                event: Str::from_static("message"),
                data: Str::from_static("test"),
                id: EMPTY_STR,
                retry: None,
            }]
        );
    }
}
