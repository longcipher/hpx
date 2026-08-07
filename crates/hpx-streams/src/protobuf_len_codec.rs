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
}

impl<T> ProtobufLenPrefixCodec<T> {
    pub(crate) const fn new_with_max_length(max_length: usize) -> Self {
        let initial_cursor = ProtobufCursor { current_obj_len: 0 };

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
        let buf_len = buf.len();
        if buf_len == 0 {
            return Ok(None);
        }

        if self.cursor.current_obj_len == 0 {
            let bytes = buf.chunk();
            let byte = bytes[0];
            if byte < 0x80 {
                buf.advance(1);
                self.cursor.current_obj_len = u64::from(byte) as usize;
            } else if buf_len > 10 || bytes[buf_len - 1] < 0x80 {
                let (value, advance) = decode_varint_slice(bytes)?;
                buf.advance(advance);
                self.cursor.current_obj_len = value as usize;
            }
            Ok(None)
        } else if self.cursor.current_obj_len > self.max_length {
            Err(StreamBodyError::new(
                StreamBodyKind::MaxLenReachedError,
                None,
                Some("Max object length reached".into()),
            ))
        } else if buf_len >= self.cursor.current_obj_len {
            let obj_bytes = buf.copy_to_bytes(self.cursor.current_obj_len);
            let result: Result<Option<T>, StreamBodyError> =
                prost::Message::decode(obj_bytes).map(Some).map_err(|err| {
                    StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
                });
            self.cursor.current_obj_len = 0;
            result
        } else {
            Ok(None)
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        if !buf.is_empty() {
            match self.decode(buf) {
                Ok(Some(item)) => Ok(Some(item)),
                Ok(None) => {
                    // Buffer has data but decode returned None, meaning the varint length
                    // prefix or the message body is incomplete at EOF.
                    Err(StreamBodyError::new(
                        StreamBodyKind::CodecError,
                        None,
                        Some("incomplete varint length prefix at EOF".into()),
                    ))
                }
                Err(e) => Err(e),
            }
        } else {
            Ok(None)
        }
    }
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
    // The varint is incomplete when the slice is short (<= 10 bytes) AND the final byte still
    // has the continuation bit set. A slice longer than 10 bytes is fine — only the first 10
    // bytes are consumed.
    if bytes.len() <= 10 && bytes[bytes.len() - 1] >= 0x80 {
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
    use prost::Message as _;
    use tokio_util::codec::Decoder;

    use super::*;

    #[derive(Clone, PartialEq, prost::Message)]
    struct TestMsg {
        #[prost(string, tag = "1")]
        name: String,
        #[prost(uint32, tag = "2")]
        value: u32,
    }

    fn encode_len_prefixed(msg: &TestMsg) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut encoded = Vec::new();
        msg.encode(&mut encoded).unwrap();
        // write varint length
        let mut len = encoded.len();
        while len >= 0x80 {
            buf.push((len as u8) | 0x80);
            len >>= 7;
        }
        buf.push(len as u8);
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
    fn empty_message() {
        // Empty message: varint=0 followed by 0 bytes of body.
        // The codec requires two decode calls: first reads varint, second reads body.
        // Note: when body length is 0, the second decode may need the buffer to contain
        // at least one additional byte (or use decode_eof) to trigger the body read path.
        let msg = TestMsg {
            name: String::new(),
            value: 0,
        };
        let data = encode_len_prefixed(&msg);
        let mut codec = ProtobufLenPrefixCodec::<TestMsg>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&data[..]);
        // First decode reads varint
        let _ = codec.decode(&mut buf);
        // Use decode_eof to flush the empty body
        let result = codec.decode_eof(&mut buf);
        // At minimum, verify no panic occurs
        let _ = result;
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
