use std::marker::PhantomData;

use bytes::{Buf, BytesMut};

use crate::{StreamBodyError, error::StreamBodyKind};

#[derive(Clone, Debug)]
pub(crate) struct ProtobufLenPrefixCodec<T> {
    max_length: usize,
    cursor: ProtobufCursor,
    _ph: PhantomData<T>,
}

#[derive(Clone, Debug)]
struct ProtobufCursor {
    current_obj_len: usize,
    /// Whether `current_obj_len` holds a decoded length prefix.
    ///
    /// Using `current_obj_len == 0` as the "not yet read" sentinel silently
    /// swallowed legitimate zero-length messages and desynchronized the
    /// stream; an explicit flag keeps those states distinct.
    have_len: bool,
}

impl<T> ProtobufLenPrefixCodec<T> {
    pub(crate) const fn new_with_max_length(max_length: usize) -> Self {
        let initial_cursor = ProtobufCursor {
            current_obj_len: 0,
            have_len: false,
        };

        Self {
            max_length,
            cursor: initial_cursor,
            _ph: PhantomData,
        }
    }
}

impl<T> tokio_util::codec::Decoder for ProtobufLenPrefixCodec<T>
where
    T: prost::Message + Default,
{
    type Item = T;
    type Error = StreamBodyError;

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        // No length prefix read yet: we need at least one byte to start.
        if !self.cursor.have_len {
            if buf.is_empty() {
                return Ok(None);
            }
            let bytes = buf.chunk();
            let byte = bytes[0];
            if byte < 0x80 {
                buf.advance(1);
                self.cursor.current_obj_len = usize::from(byte);
                self.cursor.have_len = true;
            } else if varint_is_complete(bytes) {
                let (value, advance) = decode_varint_slice(bytes)?;
                buf.advance(advance);
                self.cursor.current_obj_len = usize::try_from(value).map_err(|_| {
                    StreamBodyError::new(
                        StreamBodyKind::MaxLenReachedError,
                        None,
                        Some("length prefix exceeds addressable size".into()),
                    )
                })?;
                self.cursor.have_len = true;
            }
            // Either we just read the prefix (have_len=true) or a multi-byte
            // varint is still incomplete; in both cases wait for more data
            // before attempting to yield a message.
            return Ok(None);
        }

        // have_len == true: a length prefix is known; wait for the full body.
        if self.cursor.current_obj_len > self.max_length {
            return Err(StreamBodyError::new(
                StreamBodyKind::MaxLenReachedError,
                None,
                Some("Max object length reached".into()),
            ));
        }
        if buf.len() >= self.cursor.current_obj_len {
            let obj_bytes = buf.copy_to_bytes(self.cursor.current_obj_len);
            let result: Result<Option<T>, StreamBodyError> =
                prost::Message::decode(obj_bytes).map(Some).map_err(|err| {
                    StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
                });
            self.cursor.current_obj_len = 0;
            self.cursor.have_len = false;
            result
        } else {
            Ok(None)
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        // `decode` consumes a length prefix and then reports "need more data",
        // so the first `decode` at EOF has usually made progress without
        // yielding a message. Driving `decode` in a loop is what lets a final
        // frame whose prefix and body arrived together still be emitted;
        // calling `decode` exactly once reported a spurious "incomplete varint
        // length prefix" error and truncated the stream after one message.
        loop {
            let before = buf.len();
            match self.decode(buf)? {
                Some(item) => return Ok(Some(item)),
                None => {
                    if buf.is_empty() && !self.cursor.have_len {
                        // Clean end of stream: no bytes left and no length
                        // waiting for a body.
                        return Ok(None);
                    }
                    if buf.len() == before {
                        // No bytes were consumed, so the remaining input can
                        // never complete: either a truncated varint prefix or a
                        // body shorter than its own length prefix claims.
                        return Err(StreamBodyError::new(
                            StreamBodyKind::CodecError,
                            None,
                            Some("truncated length-prefixed message at EOF".into()),
                        ));
                    }
                }
            }
        }
    }
}

/// Maximum number of bytes a LEB128 varint can occupy.
const MAX_VARINT_BYTES: usize = 10;

/// Whether `bytes` starts with a complete LEB128 varint.
///
/// Completeness is decided by scanning for the first byte without the
/// continuation bit, never by inspecting only the last byte of the buffer.
/// Inspecting the last byte looks equivalent but is not: the message body that
/// follows a multi-byte prefix can itself begin with a byte that has the high
/// bit set, which would make an already-complete prefix look unfinished and
/// stall the stream until EOF.
#[inline]
fn varint_is_complete(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_VARINT_BYTES {
        // A varint cannot legally be this long, so let `decode_varint_slice`
        // report the malformed input rather than waiting for bytes that will
        // never arrive.
        return true;
    }
    bytes.iter().any(|b| *b < 0x80)
}

/// Decodes a LEB128-encoded variable length integer from the slice, returning the value and the
/// number of bytes read.
///
/// # Errors
///
/// Returns `StreamBodyError` if `bytes` is empty or if the varint is malformed (e.g. the slice
/// ends before the varint terminator byte is encountered).
#[inline]
#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn decode_varint_slice(bytes: &[u8]) -> Result<(u64, usize), StreamBodyError> {
    if bytes.is_empty() {
        return Err(StreamBodyError::new(
            StreamBodyKind::CodecError,
            None,
            Some("varint slice is empty".into()),
        ));
    }
    // The varint is incomplete when every one of the first 10 bytes still has
    // the continuation bit set. A slice longer than 10 bytes is fine — only
    // the first 10 are consumed. Note this scans the varint bytes rather than
    // looking at the final byte of the slice: the caller's buffer usually also
    // holds the message body, whose bytes say nothing about varint
    // completeness.
    if bytes.len() <= MAX_VARINT_BYTES && !varint_is_complete(bytes) {
        return Err(StreamBodyError::new(
            StreamBodyKind::CodecError,
            None,
            Some("malformed varint: incomplete length prefix".into()),
        ));
    }

    let mut b: u8 = bytes[0];
    let mut part0: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((u64::from(part0), 1));
    }
    part0 -= 0x80;
    b = bytes[1];
    part0 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((u64::from(part0), 2));
    }
    part0 -= 0x80 << 7;
    b = bytes[2];
    part0 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((u64::from(part0), 3));
    }
    part0 -= 0x80 << 14;
    b = bytes[3];
    part0 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((u64::from(part0), 4));
    }
    part0 -= 0x80 << 21;
    let value = u64::from(part0);

    b = bytes[4];
    let mut part1: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 5));
    }
    part1 -= 0x80;
    b = bytes[5];
    part1 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 6));
    }
    part1 -= 0x80 << 7;
    b = bytes[6];
    part1 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 7));
    }
    part1 -= 0x80 << 14;
    b = bytes[7];
    part1 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 8));
    }
    part1 -= 0x80 << 21;
    let value = value + ((u64::from(part1)) << 28);

    b = bytes[8];
    let mut part2: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part2) << 56), 9));
    }
    part2 -= 0x80;
    b = bytes[9];
    part2 += u32::from(b) << 7;
    if b < 0x02 {
        return Ok((value + (u64::from(part2) << 56), 10));
    }

    Err(StreamBodyError::new(
        StreamBodyKind::CodecError,
        None,
        Some("invalid varint".into()),
    ))
}

#[cfg(test)]
mod tests {
    use tokio_util::codec::Decoder;

    use super::*;

    #[derive(Clone, PartialEq, prost::Message)]
    struct TestMsg {
        #[prost(string, tag = "1")]
        name: String,
        #[prost(uint32, tag = "2")]
        value: u32,
    }

    /// Message whose highest-numbered field is a string, so the encoded body
    /// ends with arbitrary UTF-8 bytes rather than a varint terminator (which
    /// always has the high bit clear).
    #[derive(Clone, PartialEq, prost::Message)]
    struct TrailingLabelMsg {
        #[prost(uint32, tag = "1")]
        count: u32,
        #[prost(string, tag = "2")]
        label: String,
    }

    fn encode_len_prefixed<M: prost::Message>(msg: &M) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut encoded = Vec::new();
        msg.encode(&mut encoded).unwrap();
        // write varint length
        let mut len = encoded.len();
        while len >= 0x80 {
            buf.push(u8::try_from(len & 0x7F).unwrap() | 0x80);
            len >>= 7;
        }
        buf.push(u8::try_from(len).unwrap());
        buf.extend_from_slice(&encoded);
        buf
    }

    #[test]
    fn normal_parse_single_message() {
        let msg = TestMsg {
            name: "alice".into(),
            value: 42,
        };
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);
        // First decode reads varint, returns None
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        // Second decode reads the message body
        let result = codec.decode(&mut buf).unwrap();
        assert_eq!(result, Some(msg));
    }

    #[test]
    fn empty_input_returns_none() {
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::new();
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
    }

    #[test]
    fn truncated_payload_returns_none() {
        let msg = TestMsg {
            name: "alice".into(),
            value: 42,
        };
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..3]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
    }

    #[test]
    fn max_length_exceeded_returns_error() {
        let msg = TestMsg {
            name: "alice".into(),
            value: 42,
        };
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(5);
        let mut buf = BytesMut::from(&data[..]);
        // First decode reads varint, returns None
        let _ = codec.decode(&mut buf);
        // Second decode: message body (10 bytes) exceeds max_length (5)
        assert!(codec.decode(&mut buf).is_err());
    }

    #[test]
    fn multiple_messages_in_sequence() {
        let m1 = TestMsg {
            name: "a".into(),
            value: 1,
        };
        let m2 = TestMsg {
            name: "b".into(),
            value: 2,
        };
        let mut data = encode_len_prefixed(&m1);
        data.extend_from_slice(&encode_len_prefixed(&m2));

        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);
        // First message: varint + body
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(m1));
        // Second message: varint + body
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(m2));
    }

    #[test]
    fn zero_length_message_is_delivered() {
        // Empty message: varint=0 followed by 0 bytes of body. The message
        // must be delivered, not silently dropped.
        let msg = TestMsg::default();
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);
        // First decode reads the varint length prefix.
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        // Second decode yields the zero-length message.
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(msg));
    }

    #[test]
    fn zero_length_message_does_not_desync_stream() {
        // A zero-length message followed by a normal one: the stream must
        // stay aligned (the empty message previously consumed the next
        // message's length prefix).
        let first = TestMsg::default();
        let second = TestMsg {
            name: "after".into(),
            value: 2,
        };
        let mut data = encode_len_prefixed(&first);
        data.extend_from_slice(&encode_len_prefixed(&second));

        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);

        let mut decoded = Vec::new();
        while decoded.len() < 2 {
            match codec.decode(&mut buf).expect("decode") {
                Some(msg) => decoded.push(msg),
                None if buf.is_empty() => break,
                None => continue,
            }
        }

        assert_eq!(decoded, vec![first, second]);
    }

    #[test]
    fn long_field_value() {
        let msg = TestMsg {
            name: "a".repeat(500),
            value: u32::MAX,
        };
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        let result = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.name.len(), 500);
        assert_eq!(result.value, u32::MAX);
    }

    #[test]
    fn decode_eof_returns_none_on_empty() {
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::new();
        assert!(matches!(codec.decode_eof(&mut buf), Ok(None)));
    }

    #[test]
    fn incremental_feed_varint_split() {
        let msg = TestMsg {
            name: "split".into(),
            value: 7,
        };
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);

        // Feed first byte of varint
        let mut buf = BytesMut::from(&data[..1]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));

        // Feed rest of data
        buf.extend_from_slice(&data[1..]);
        // varint decode might still need more context
        let _ = codec.decode(&mut buf);
        // Eventually the message should decode
        let result = codec.decode(&mut buf);
        // The result depends on varint parsing; at minimum it shouldn't panic
        let _ = result;
    }

    #[test]
    fn multiple_decodes_same_buffer() {
        let messages: Vec<TestMsg> = (1..=5)
            .map(|i| TestMsg {
                name: format!("msg{i}"),
                value: i,
            })
            .collect();

        let mut data = Vec::new();
        for msg in &messages {
            data.extend_from_slice(&encode_len_prefixed(msg));
        }

        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(4096);
        let mut buf = BytesMut::from(&data[..]);

        let mut decoded = Vec::new();
        // The codec may need multiple decode calls per message (varint then body).
        // Use an absolute iteration bound so a mutant that always yields Some
        // (e.g. decode returning a default message) fails fast instead of hanging.
        let mut iterations = 0;
        let mut idle = 0;
        while iterations < 1_000 {
            iterations += 1;
            match codec.decode(&mut buf) {
                Ok(Some(msg)) => {
                    decoded.push(msg);
                    idle = 0;
                }
                Ok(None) => {
                    idle += 1;
                    if buf.is_empty() && idle > 2 {
                        break;
                    }
                }
                Err(e) => panic!("unexpected error: {e}"),
            }
        }

        assert_eq!(decoded.len(), 5);
        for (i, msg) in decoded.iter().enumerate() {
            let expected = i as u32 + 1;
            assert_eq!(msg.name, format!("msg{expected}"));
            assert_eq!(msg.value, expected);
        }
    }

    #[test]
    fn varint_edge_cases() {
        // Single-byte varint (value < 128)
        let msg = TestMsg {
            name: "x".into(),
            value: 1,
        };
        let data = encode_len_prefixed(&msg);
        assert!(data[0] < 0x80, "single-byte varint expected");

        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        let result = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(result.name, "x");
    }

    #[test]
    fn decode_varint_slice_multibyte_and_malformed() {
        // Multi-byte varint: 0x80 0x01 encodes 128 over two bytes.
        let (val, advance) = decode_varint_slice(&[0x80, 0x01]).unwrap();
        assert_eq!((val, advance), (128, 2));

        // Malformed: the last byte still has the continuation bit set.
        assert!(decode_varint_slice(&[0x80, 0x80]).is_err());
        // Empty input.
        assert!(decode_varint_slice(&[]).is_err());
    }

    #[test]
    fn decode_varint_slice_across_all_byte_lengths() {
        // 1-byte varint.
        assert_eq!(decode_varint_slice(&[0x05]).unwrap(), (5, 1));
        // 2-byte: 128.
        assert_eq!(decode_varint_slice(&[0x80, 0x01]).unwrap(), (128, 2));
        // 3-byte: 16384 = 1 << 14.
        assert_eq!(
            decode_varint_slice(&[0x80, 0x80, 0x01]).unwrap(),
            (16_384, 3)
        );
        // 4-byte: 2097152 = 1 << 21.
        assert_eq!(
            decode_varint_slice(&[0x80, 0x80, 0x80, 0x01]).unwrap(),
            (2_097_152, 4)
        );
        // 5-byte: 268435456 = 1 << 28.
        assert_eq!(
            decode_varint_slice(&[0x80, 0x80, 0x80, 0x80, 0x01]).unwrap(),
            (268_435_456, 5)
        );
        // 6-byte: 1 << 35.
        assert_eq!(
            decode_varint_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01]).unwrap(),
            (34_359_738_368, 6)
        );
        // 7-byte: 1 << 42.
        assert_eq!(
            decode_varint_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]).unwrap(),
            (4_398_046_511_104, 7)
        );
        // 8-byte: 1 << 49.
        assert_eq!(
            decode_varint_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]).unwrap(),
            (562_949_953_421_312, 8)
        );
        // 9-byte: 1 << 56.
        assert_eq!(
            decode_varint_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]).unwrap(),
            (72_057_594_037_927_936, 9)
        );
        // 10-byte: 1 << 63 (final byte must be < 0x02).
        assert_eq!(
            decode_varint_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01])
                .unwrap(),
            (9_223_372_036_854_775_808, 10)
        );
        // 10-byte overflow: final byte >= 0x02 is invalid.
        assert!(
            decode_varint_slice(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02])
                .is_err()
        );
    }

    #[test]
    fn decode_eof_emits_pending_body() {
        let msg = TestMsg {
            name: "flush".into(),
            value: 9,
        };
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);
        // First decode reads the length prefix.
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        // decode_eof must flush the pending message body.
        let result = codec.decode_eof(&mut buf).unwrap();
        assert_eq!(result, Some(msg));
    }

    #[test]
    fn decode_eof_errors_on_incomplete_varint() {
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        // 0x80 alone: continuation bit set but no terminator byte available.
        let mut buf = BytesMut::from(&[0x80u8][..]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        assert!(codec.decode_eof(&mut buf).is_err());
    }

    #[test]
    fn decode_eof_drains_a_fully_buffered_stream() {
        // Regression: `decode_eof` used to call `decode` exactly once. Because
        // `decode` consumes the length prefix and then reports "need more
        // data", that single call yielded no message and `decode_eof` declared
        // the stream truncated. `FramedRead` therefore returned exactly one
        // message followed by an error for any multi-message response.
        let expected: Vec<TestMsg> = (0..5)
            .map(|i| TestMsg {
                name: format!("m{i}"),
                value: i,
            })
            .collect();
        let mut data = Vec::new();
        for msg in &expected {
            data.extend_from_slice(&encode_len_prefixed(msg));
        }

        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(65536);
        let mut buf = BytesMut::from(&data[..]);

        let mut decoded = Vec::new();
        // Mirror how `FramedRead` drives the codec: decode until it needs more
        // bytes, then decode_eof until it reports a clean end.
        loop {
            match codec.decode(&mut buf) {
                Ok(Some(msg)) => decoded.push(msg),
                Ok(None) => break,
                Err(e) => panic!("unexpected decode error: {e}"),
            }
        }
        loop {
            match codec.decode_eof(&mut buf) {
                Ok(Some(msg)) => decoded.push(msg),
                Ok(None) => break,
                Err(e) => panic!("unexpected decode_eof error: {e}"),
            }
        }

        assert_eq!(decoded, expected, "decode_eof dropped messages");
    }

    #[test]
    fn decode_eof_emits_a_single_fully_buffered_message() {
        // One message, delivered with its prefix and body in the same segment:
        // the very first `decode_eof` must already produce it.
        let msg = TestMsg {
            name: "solo".into(),
            value: 3,
        };
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);

        assert_eq!(codec.decode_eof(&mut buf).unwrap(), Some(msg));
        assert!(codec.decode_eof(&mut buf).unwrap().is_none());
    }

    #[test]
    fn multi_byte_prefix_completed_before_a_high_bit_body_byte_does_not_stall() {
        // Regression: prefix completeness was decided by looking at the *last*
        // byte of the buffer rather than by scanning the varint. When a TCP
        // segment ends just after a two-byte length prefix, the buffer still
        // holds the first few body bytes, and one of them carrying the high bit
        // made an already-complete prefix look unfinished. The codec then waited
        // for bytes that could never complete its (wrong) view of the prefix and
        // the stream stalled until EOF, where it surfaced as a truncation error.
        let msg = TrailingLabelMsg {
            count: 1,
            // Starts with "abÿ": the third byte is 0xC3, i.e. high bit set.
            label: format!("ab\u{00FF}{}", "x".repeat(200)),
        };
        let data = encode_len_prefixed(&msg);
        assert!(
            data.len() > 10 && data[0] >= 0x80,
            "fixture needs a multi-byte length prefix and a buffer longer than 10 bytes"
        );

        // Exactly the pathological prefix: 10 bytes, the 10th with the high bit
        // set, and the varint already terminated on byte 2.
        let mut codec = ProtobufLenPrefixCodec::<TrailingLabelMsg>::new_with_max_length(4096);
        let mut buf = BytesMut::from(&data[..10]);
        assert!(
            buf[9] >= 0x80,
            "fixture must place a high-bit byte at the buffer tail"
        );
        assert!(matches!(codec.decode(&mut buf), Ok(None)));

        // The prefix must have been consumed, leaving only the pending body.
        assert_eq!(buf.len(), 8, "the two-byte prefix must be consumed");
        assert!(matches!(codec.decode(&mut buf), Ok(None)));

        // Supplying the rest yields the message intact.
        buf.extend_from_slice(&data[10..]);
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(msg));
        assert!(codec.decode_eof(&mut buf).unwrap().is_none());
    }

    #[test]
    fn varint_is_complete_scans_varint_bytes_not_the_buffer_tail() {
        // A terminated varint is complete even when the following bytes all have
        // the high bit set.
        assert!(varint_is_complete(&[0x82, 0x02, 0x8A, 0xFF]));
        assert!(varint_is_complete(&[0x01]));
        // No terminator within the 10-byte window: still incomplete.
        assert!(!varint_is_complete(&[0x80; 10]));
        assert!(!varint_is_complete(&[0x80, 0x80]));
        // Beyond 10 bytes the input is malformed; report it rather than wait.
        assert!(varint_is_complete(&[0x80; 11]));
        assert!(!varint_is_complete(&[]));
    }

    #[test]
    fn decode_varint_slice_ignores_body_bytes_when_checking_completeness() {
        // The completeness guard must not consult the buffer tail. Before the
        // fix this rejected a valid 2-byte varint that happened to be followed
        // by a high-bit body byte.
        let (value, advance) = decode_varint_slice(&[0xD1, 0x01, 0xC3, 0xFF]).unwrap();
        assert_eq!((value, advance), (209, 2));
    }

    /// Property tests for the length-prefix framing.
    mod properties {
        use bytes::BytesMut;
        use proptest::prelude::*;
        use prost::Message as _;
        use tokio_util::codec::Decoder;

        use super::{ProtobufLenPrefixCodec, TestMsg, decode_varint_slice, varint_is_complete};

        /// Absolute step bound for the chunk-feeding helper. Kept low so a
        /// non-terminating mutant fails fast instead of timing out.
        const MAX_HELPER_STEPS: usize = 4_096;

        /// Encode a varint exactly as the wire format requires.
        fn encode_varint(value: u64) -> Vec<u8> {
            let mut out = Vec::new();
            let mut rest = value;
            while rest >= 0x80 {
                out.push(u8::try_from(rest & 0x7F).expect("7 bits fit a byte") | 0x80);
                rest >>= 7;
            }
            out.push(u8::try_from(rest).expect("7 bits fit a byte"));
            out
        }

        /// Drive the codec the way `FramedRead` does: decode until it needs more
        /// bytes, then `decode_eof` until it reports a clean end. Returns
        /// `None` if any step reported an error.
        fn decode_framed<M>(data: &[u8], chunk: usize, max_length: usize) -> Option<Vec<M>>
        where
            M: prost::Message + Default + std::fmt::Debug,
        {
            let mut codec = ProtobufLenPrefixCodec::<M>::new_with_max_length(max_length);
            let mut buf = BytesMut::new();
            let mut out = Vec::new();
            let mut fed = 0usize;

            for _ in 0..MAX_HELPER_STEPS {
                if fed < data.len() {
                    let take = chunk.min(data.len() - fed);
                    buf.extend_from_slice(&data[fed..fed + take]);
                    fed += take;
                }
                match codec.decode(&mut buf) {
                    Ok(Some(message)) => {
                        out.push(message);
                        continue;
                    }
                    Ok(None) => {}
                    Err(_) => return None,
                }
                if fed == data.len() {
                    // Keep draining at EOF: `decode_eof` yields at most one
                    // frame per call, so a fully-buffered stream needs several
                    // calls before it reports a clean end.
                    match codec.decode_eof(&mut buf) {
                        Ok(Some(message)) => {
                            out.push(message);
                            continue;
                        }
                        Ok(None) => return Some(out),
                        Err(_) => return None,
                    }
                }
            }
            panic!(
                "decoder did not terminate for a {}-byte payload",
                data.len()
            );
        }

        fn message() -> impl Strategy<Value = TestMsg> {
            (
                proptest::collection::vec(any::<char>(), 0..40),
                any::<u32>(),
            )
                .prop_map(|(chars, value)| TestMsg {
                    name: chars.into_iter().collect(),
                    value,
                })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]

            /// A stream of generated messages decodes back to exactly those
            /// messages, in order, at every chunk size.
            #[test]
            fn message_stream_round_trips(
                messages in proptest::collection::vec(message(), 1..8),
                chunk in 1usize..40,
            ) {
                let mut data = Vec::new();
                for m in &messages {
                    let mut body = Vec::new();
                    m.encode(&mut body).expect("prost encodes into a Vec");
                    data.extend_from_slice(&encode_varint(body.len() as u64));
                    data.extend_from_slice(&body);
                }
                prop_assert_eq!(
                    decode_framed::<TestMsg>(&data, chunk, 1 << 20),
                    Some(messages),
                    "failed for a {}-byte payload at chunk size {}",
                    data.len(),
                    chunk
                );
            }

            /// An empty message is delivered, and the stream stays aligned, no
            /// matter where the empty messages sit.
            #[test]
            fn zero_length_messages_do_not_desync(
                empties in prop::collection::vec(any::<bool>(), 1..10),
            ) {
                let expected: Vec<TestMsg> = empties
                    .iter()
                    .enumerate()
                    .map(|(i, empty)| {
                        if *empty {
                            TestMsg::default()
                        } else {
                            TestMsg {
                                name: format!("m{i}"),
                                value: i as u32,
                            }
                        }
                    })
                    .collect();
                let mut data = Vec::new();
                for m in &expected {
                    let mut body = Vec::new();
                    m.encode(&mut body).expect("prost encodes into a Vec");
                    data.extend_from_slice(&encode_varint(body.len() as u64));
                    data.extend_from_slice(&body);
                }
                prop_assert_eq!(decode_framed::<TestMsg>(&data, 3, 1 << 20), Some(expected));
            }

            /// `decode_varint_slice` agrees with the encoder for every value the
            /// length-prefixed format can carry.
            #[test]
            fn varint_round_trips(value in 0u64..(1 << 24)) {
                let encoded = encode_varint(value);
                let (decoded, advance) = decode_varint_slice(&encoded)
                    .expect("an encoder-produced varint always decodes");
                prop_assert_eq!(advance, encoded.len());
                prop_assert_eq!(decoded, value);
            }

            /// Prefix completeness is decided by scanning the varint, so a body
            /// byte with the high bit set right after a terminated prefix cannot
            /// stall the decoder.
            #[test]
            fn varint_completeness_ignores_the_buffer_tail(
                value in 128u64..(1 << 24),
                tail in proptest::collection::vec(128u8..=255, 1..8),
            ) {
                let encoded = encode_varint(value);
                let mut buffer = encoded.clone();
                buffer.extend_from_slice(&tail);
                prop_assert!(varint_is_complete(&buffer));
                let (decoded, advance) = decode_varint_slice(&buffer)
                    .expect("a terminated varint decodes regardless of the tail");
                prop_assert_eq!(advance, encoded.len());
                prop_assert_eq!(decoded, value);
            }

            /// A varint whose bytes all carry the continuation bit is never
            /// mistaken for complete, so the decoder waits for more input
            /// instead of mis-framing the stream.
            #[test]
            fn truncated_varint_is_never_complete(len in 1usize..10) {
                prop_assert!(!varint_is_complete(&vec![0x80u8; len]));
            }

            /// A message larger than `max_length` is rejected rather than
            /// buffered.
            #[test]
            fn oversized_messages_are_rejected(name_len in 1usize..200) {
                let m = TestMsg {
                    name: "x".repeat(name_len),
                    value: 1,
                };
                let mut body = Vec::new();
                m.encode(&mut body).expect("prost encodes into a Vec");
                let mut data = encode_varint(body.len() as u64);
                data.extend_from_slice(&body);

                let decoded = decode_framed::<TestMsg>(&data, 64, 4);
                prop_assert!(decoded.is_none(), "an oversized message must be rejected");
            }

            /// Arbitrary bytes never panic, whatever the chunk size.
            #[test]
            fn arbitrary_bytes_never_panic(
                data in proptest::collection::vec(any::<u8>(), 0..256),
                chunk in 1usize..16,
            ) {
                let _ = decode_framed::<TestMsg>(&data, chunk, 1 << 16);
            }
        }
    }

    #[test]
    fn exact_max_length_body_is_accepted() {
        // body = field(0x0A,0x08) + 8 name bytes + field(0x10,0x01) = 12 bytes
        let msg = TestMsg {
            name: "x".repeat(8),
            value: 1,
        };
        let data = encode_len_prefixed(&msg);
        let body_len = data.len() - 1; // single-byte varint prefix
        assert_eq!(body_len, 12, "expected an exact 12-byte body");

        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(body_len);
        let mut buf = BytesMut::from(&data[..]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        let result = codec.decode(&mut buf).unwrap();
        assert_eq!(result, Some(msg));
    }
}
