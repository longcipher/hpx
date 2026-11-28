//! Zero-copy [FIX Simple Binary Encoding] streaming support plus SBE schema
//! negotiation helpers for exchange APIs.
//!
//! A FIX SBE message starts with an 8-byte little-endian message header:
//! `block_length: u16`, `template_id: u16`, `schema_id: u16`, `version: u16`.
//! Exchange streams (for example Binance's `responseFormat=sbe`) wrap that
//! message in a 4-byte big-endian length prefix so a decoder can frame it
//! without knowing the template. [`SbeCodec`] implements exactly that framing;
//! [`SbeMessage`] exposes the header and a borrowed body without copying the
//! payload.
//!
//! The codec deliberately does **not** embed a venue-specific schema: schemas
//! evolve independently of the client. Instead, [`SbeSchema`] carries the
//! schema id/version through `Accept` + `X-MBX-SBE` (REST) or the WebSocket
//! query parameters, [`SbeRequest`] applies that negotiation to an outgoing
//! request, and [`SbeStreamResponse::sbe_refused`] implements the documented
//! `406` / `415` downgrade to JSON.
//!
//! [FIX Simple Binary Encoding]: https://www.fixtrading.org/standards/sbe-online/
//!
//! # Example
//!
//! ```rust,no_run
//! use hpx_streams::SbeStreamResponse as _;
//!
//! # async fn example(client: hpx::Client) -> Result<(), Box<dyn std::error::Error>> {
//! let stream = client
//!     .get("https://api.binance.com/api/v3/exchangeInfo")
//!     .send()
//!     .await?
//!     .sbe_stream(64 * 1024);
//! # let _ = stream;
//! # Ok(())
//! # }
//! ```

use bytes::{Buf, Bytes, BytesMut};
use futures::TryStreamExt;
use tokio_util::{codec::Decoder, io::StreamReader};

use crate::{
    StreamBodyResult,
    error::{StreamBodyError, StreamBodyKind},
};

/// Size of the FIX SBE message header in bytes.
pub const SBE_HEADER_LEN: usize = 8;
/// Size of the big-endian length prefix used by SBE response envelopes.
pub const SBE_LENGTH_PREFIX_LEN: usize = 4;

/// The `X-MBX-SBE` / query parameter representation of an SBE schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SbeSchema {
    id: u16,
    version: u16,
}

impl SbeSchema {
    /// Create a schema selection.
    #[inline]
    #[must_use]
    pub const fn new(id: u16, version: u16) -> Self {
        Self { id, version }
    }

    /// Schema id.
    #[inline]
    #[must_use]
    pub const fn id(&self) -> u16 {
        self.id
    }

    /// Schema version.
    #[inline]
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Value for the `X-MBX-SBE` request header (`"<id>:<version>"`).
    #[must_use]
    pub fn header_value(&self) -> String {
        format!("{}:{}", self.id, self.version)
    }

    /// Value for the `Accept` request header.
    #[inline]
    #[must_use]
    pub const fn accept_value() -> &'static str {
        "application/sbe"
    }

    /// WebSocket URL query suffix selecting this schema.
    #[must_use]
    pub fn ws_query(&self) -> String {
        format!(
            "responseFormat=sbe&sbeSchemaId={}&sbeSchemaVersion={}",
            self.id, self.version
        )
    }
}

impl Default for SbeSchema {
    fn default() -> Self {
        Self::new(1, 0)
    }
}

/// Return `true` when the server refused SBE and the caller must retry JSON.
///
/// `406 Not Acceptable` is the documented SBE rejection; `415` is included
/// for servers that report an unsupported media type with the same meaning.
#[inline]
#[must_use]
pub const fn should_fallback(status: u16) -> bool {
    status == 406 || status == 415
}

/// A parsed SBE message with a zero-copy body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SbeMessage {
    block_length: u16,
    template_id: u16,
    schema_id: u16,
    version: u16,
    body: Bytes,
}

impl SbeMessage {
    /// Parse an unframed SBE message (header followed by its fixed/variable body).
    ///
    /// # Errors
    ///
    /// Returns [`StreamBodyKind::CodecError`] when fewer than
    /// [`SBE_HEADER_LEN`] bytes are present.
    pub fn parse(frame: Bytes) -> Result<Self, StreamBodyError> {
        if frame.len() < SBE_HEADER_LEN {
            return Err(codec_error(format!(
                "SBE message is {} bytes, need at least {SBE_HEADER_LEN}",
                frame.len()
            )));
        }
        let mut header = &frame[..SBE_HEADER_LEN];
        let block_length = header.get_u16_le();
        let template_id = header.get_u16_le();
        let schema_id = header.get_u16_le();
        let version = header.get_u16_le();
        Ok(Self {
            block_length,
            template_id,
            schema_id,
            version,
            body: frame.slice(SBE_HEADER_LEN..),
        })
    }

    /// Declared length of the fixed-size root block.
    #[inline]
    #[must_use]
    pub const fn block_length(&self) -> u16 {
        self.block_length
    }

    /// SBE template id identifying the message type.
    #[inline]
    #[must_use]
    pub const fn template_id(&self) -> u16 {
        self.template_id
    }

    /// SBE schema id.
    #[inline]
    #[must_use]
    pub const fn schema_id(&self) -> u16 {
        self.schema_id
    }

    /// SBE schema version.
    #[inline]
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Borrow the message body without copying.
    #[inline]
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Consume the message, returning the owned body buffer.
    #[inline]
    #[must_use]
    pub fn into_body(self) -> Bytes {
        self.body
    }

    /// Read a little-endian `u64` at `offset` inside the body.
    #[inline]
    #[must_use]
    pub fn read_u64(&self, offset: usize) -> Option<u64> {
        let end = offset.checked_add(8)?;
        self.body
            .get(offset..end)
            .map(|mut slice| slice.get_u64_le())
    }

    /// Read a little-endian `i64` at `offset` inside the body.
    #[inline]
    #[must_use]
    pub fn read_i64(&self, offset: usize) -> Option<i64> {
        let end = offset.checked_add(8)?;
        self.body
            .get(offset..end)
            .map(|mut slice| slice.get_i64_le())
    }
}

fn codec_error(message: String) -> StreamBodyError {
    StreamBodyError::new(StreamBodyKind::CodecError, None, Some(message))
}

fn max_len_error(message: String) -> StreamBodyError {
    StreamBodyError::new(StreamBodyKind::MaxLenReachedError, None, Some(message))
}

/// Framing decoder for length-prefixed SBE response envelopes.
#[derive(Clone, Debug)]
pub struct SbeCodec {
    max_frame_len: usize,
}

impl SbeCodec {
    /// Create a decoder that rejects envelopes larger than `max_frame_len` bytes.
    #[inline]
    #[must_use]
    pub const fn new(max_frame_len: usize) -> Self {
        Self { max_frame_len }
    }

    /// Configured maximum envelope size.
    #[inline]
    #[must_use]
    pub const fn max_frame_len(&self) -> usize {
        self.max_frame_len
    }
}

impl Decoder for SbeCodec {
    type Item = SbeMessage;
    type Error = StreamBodyError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < SBE_LENGTH_PREFIX_LEN {
            return Ok(None);
        }

        let frame_len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if frame_len < SBE_HEADER_LEN {
            src.clear();
            return Err(codec_error(format!(
                "SBE envelope declares {frame_len} bytes, less than the {SBE_HEADER_LEN}-byte header"
            )));
        }
        if frame_len > self.max_frame_len {
            src.clear();
            return Err(max_len_error(format!(
                "SBE envelope of {frame_len} bytes exceeds the {}-byte limit",
                self.max_frame_len
            )));
        }

        // A u32 length prefix can exceed the address space on 32-bit targets;
        // reject it instead of wrapping and later panicking in `split_to`.
        let Some(total) = SBE_LENGTH_PREFIX_LEN.checked_add(frame_len) else {
            src.clear();
            return Err(codec_error(format!(
                "SBE envelope length {frame_len} does not fit in the address space"
            )));
        };
        if src.len() < total {
            src.reserve(total - src.len());
            return Ok(None);
        }

        let _prefix = src.split_to(SBE_LENGTH_PREFIX_LEN);
        let frame = src.split_to(frame_len).freeze();
        SbeMessage::parse(frame).map(Some)
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(src)? {
            Some(frame) => Ok(Some(frame)),
            None if src.is_empty() => Ok(None),
            None => Err(codec_error(format!(
                "truncated SBE envelope: {} bytes remain at EOF",
                src.len()
            ))),
        }
    }
}

/// Extension trait that negotiates an SBE schema on an outgoing request.
///
/// The codec deliberately does **not** embed a venue-specific schema: schemas
/// evolve independently of the client. Instead, [`SbeSchema`] carries the
/// schema id and version through `Accept` + `X-MBX-SBE` (REST), and
/// [`SbeSchema::ws_query`] carries it through the WebSocket query parameters.
pub trait SbeRequest {
    /// Ask the server for `schema`.
    ///
    /// The request carries `Accept: application/sbe` and
    /// `X-MBX-SBE: "<id>:<version>"`.
    fn sbe_schema(self, schema: SbeSchema) -> Self;
}

impl SbeRequest for hpx::RequestBuilder {
    fn sbe_schema(self, schema: SbeSchema) -> Self {
        self.header("accept", SbeSchema::accept_value())
            .header("x-mbx-sbe", schema.header_value())
    }
}

/// Extension trait turning an [`hpx::Response`] into a stream of SBE messages.
pub trait SbeStreamResponse {
    /// Stream length-prefixed SBE envelopes, rejecting frames larger than
    /// `max_frame_len` bytes.
    fn sbe_stream(
        self,
        max_frame_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<SbeMessage>> + Send;

    /// Whether the server refused the SBE negotiation.
    ///
    /// `406 Not Acceptable` is the documented SBE rejection; `415` is
    /// included for servers that report an unsupported media type with the
    /// same meaning. A refused response carries no SBE payload, so the caller
    /// must retry the request as JSON.
    fn sbe_refused(&self) -> bool;
}

impl SbeStreamResponse for hpx::Response {
    fn sbe_stream(
        self,
        max_frame_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<SbeMessage>> + Send {
        let reader = StreamReader::new(self.bytes_stream().map_err(std::io::Error::other));
        let frames = tokio_util::codec::FramedRead::new(reader, SbeCodec::new(max_frame_len));
        frames.into_stream()
    }

    fn sbe_refused(&self) -> bool {
        should_fallback(self.status().as_u16())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(message: &[u8]) -> BytesMut {
        let mut out = BytesMut::new();
        out.extend_from_slice(&(message.len() as u32).to_be_bytes());
        out.extend_from_slice(message);
        out
    }

    fn message() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&32u16.to_le_bytes()); // block length
        out.extend_from_slice(&7u16.to_le_bytes()); // template id
        out.extend_from_slice(&1u16.to_le_bytes()); // schema id
        out.extend_from_slice(&0u16.to_le_bytes()); // version
        out.extend_from_slice(&42u64.to_le_bytes());
        out.extend_from_slice(&(-9i64).to_le_bytes());
        out
    }

    #[test]
    fn decodes_a_complete_envelope() {
        let mut codec = SbeCodec::new(1024);
        let mut src = envelope(&message());
        let frame = codec.decode(&mut src).expect("decode").expect("frame");
        assert_eq!(frame.block_length(), 32);
        assert_eq!(frame.template_id(), 7);
        assert_eq!(frame.schema_id(), 1);
        assert_eq!(frame.version(), 0);
        assert_eq!(frame.read_u64(0), Some(42));
        assert_eq!(frame.read_i64(8), Some(-9));
        assert!(src.is_empty());
    }

    #[test]
    fn waits_for_the_whole_frame() {
        let mut codec = SbeCodec::new(1024);
        let mut src = envelope(&message());
        let tail = src.split_off(6);
        assert!(codec.decode(&mut src).expect("partial").is_none());
        src.extend_from_slice(&tail);
        assert!(codec.decode(&mut src).expect("complete").is_some());
    }

    #[test]
    fn decodes_consecutive_frames() {
        let mut codec = SbeCodec::new(1024);
        let mut src = BytesMut::new();
        src.extend_from_slice(&envelope(&message()));
        src.extend_from_slice(&envelope(&message()));
        assert_eq!(
            codec
                .decode(&mut src)
                .expect("first")
                .expect("first")
                .template_id(),
            7
        );
        assert_eq!(
            codec
                .decode(&mut src)
                .expect("second")
                .expect("second")
                .template_id(),
            7
        );
        assert!(src.is_empty());
    }

    #[test]
    fn rejects_short_and_oversized_frames() {
        let mut codec = SbeCodec::new(16);
        let mut short = BytesMut::from(&[0u8, 0, 0, 3][..]);
        let err = codec.decode(&mut short).expect_err("short frame");
        assert_eq!(err.kind(), StreamBodyKind::CodecError);

        let mut oversized = BytesMut::from(&[0u8, 0, 0, 32][..]);
        let err = codec.decode(&mut oversized).expect_err("oversized frame");
        assert_eq!(err.kind(), StreamBodyKind::MaxLenReachedError);
    }

    #[test]
    fn truncated_frame_at_eof_is_an_error() {
        let mut codec = SbeCodec::new(1024);
        let mut src = BytesMut::from(&[0u8, 0, 0, 16, 1, 2, 3][..]);
        let err = codec.decode_eof(&mut src).expect_err("truncated");
        assert_eq!(err.kind(), StreamBodyKind::CodecError);
        assert!(
            codec
                .decode_eof(&mut BytesMut::new())
                .expect("clean eof")
                .is_none()
        );
    }

    #[test]
    fn reads_past_the_end_return_none_instead_of_overflowing() {
        let frame = SbeMessage::parse(Bytes::from(message())).expect("parse");
        assert_eq!(frame.read_u64(usize::MAX), None);
        assert_eq!(frame.read_i64(usize::MAX), None);
        assert_eq!(frame.read_u64(frame.body().len()), None);
    }

    /// On a 32-bit target the u32 length prefix plus the envelope prefix can
    /// exceed `usize`; the decoder must reject it rather than wrap and panic
    /// in `split_to`.
    #[cfg(target_pointer_width = "32")]
    #[test]
    fn an_unrepresentable_envelope_length_is_rejected() {
        let mut codec = SbeCodec::new(usize::MAX);
        let mut src = BytesMut::from(&[0xFF_u8, 0xFF, 0xFF, 0xFF][..]);
        let err = codec.decode(&mut src).expect_err("overflowing length");
        assert_eq!(err.kind(), StreamBodyKind::CodecError);
    }

    #[test]
    fn parse_rejects_a_missing_header() {
        assert_eq!(
            SbeMessage::parse(Bytes::from_static(&[1, 2, 3]))
                .expect_err("short")
                .kind(),
            StreamBodyKind::CodecError
        );
    }

    #[test]
    fn schema_negotiation_strings() {
        let schema = SbeSchema::new(1, 0);
        assert_eq!(schema.header_value(), "1:0");
        assert_eq!(SbeSchema::accept_value(), "application/sbe");
        assert_eq!(
            schema.ws_query(),
            "responseFormat=sbe&sbeSchemaId=1&sbeSchemaVersion=0"
        );
        assert_eq!(SbeSchema::default().header_value(), "1:0");
    }

    #[test]
    fn fallback_only_on_documented_statuses() {
        assert!(should_fallback(406));
        assert!(should_fallback(415));
        assert!(!should_fallback(200));
        assert!(!should_fallback(429));
    }

    #[test]
    fn body_is_borrowed_not_copied() {
        let frame = SbeMessage::parse(Bytes::from(message())).expect("parse");
        let expected = frame.body().to_vec();
        assert_eq!(expected.len(), 16);
        let owned = frame.into_body();
        assert_eq!(&owned[..], expected.as_slice());
    }

    #[test]
    fn codec_reports_its_maximum_frame_length() {
        assert_eq!(SbeCodec::new(4096).max_frame_len(), 4096);
    }

    /// The schema negotiation must reach the wire.
    #[tokio::test]
    async fn sbe_schema_sets_the_negotiation_headers() {
        let client = hpx::Client::builder().build().expect("client");
        let builder = client
            .get("http://localhost:8080/exchangeInfo")
            .sbe_schema(SbeSchema::new(1, 0));
        let debug = format!("{builder:?}");
        assert!(debug.contains("application/sbe"), "{debug}");
        assert!(debug.contains("x-mbx-sbe"), "{debug}");
        assert!(debug.contains("1:0"), "{debug}");
    }
}
