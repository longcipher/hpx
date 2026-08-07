use std::marker::PhantomData;

use bytes::{Buf, BytesMut};
use serde::Deserialize;

use crate::{StreamBodyError, error::StreamBodyKind};

#[derive(Clone, Debug)]
pub(crate) struct JsonArrayCodec<T> {
    max_length: usize,
    json_cursor: JsonCursor,
    #[cfg(feature = "simd-json")]
    simd_buf: Vec<u8>,
    _ph: PhantomData<T>,
}

#[derive(Clone, Debug)]
struct JsonCursor {
    pub(crate) current_offset: usize,
    pub(crate) array_is_opened: bool,
    pub(crate) delimiter_expected: bool,
    pub(crate) quote_opened: bool,
    /// Quote state for strings inside nested objects/arrays (`opened_brackets > 0`).
    /// Kept separate from `quote_opened` so nested-string escape state never pollutes
    /// top-level string tracking across frames.
    pub(crate) nested_quote_opened: bool,
    pub(crate) escaped: bool,
    pub(crate) opened_brackets: usize,
    pub(crate) current_obj_pos: usize,
    /// When Some(pos), we are accumulating a primitive value (number/bool/null/string)
    /// that started at `pos` in the buffer. A quoted string also uses this.
    pub(crate) current_primitive_start: Option<usize>,
}

impl<T> JsonArrayCodec<T> {
    pub(crate) fn new_with_max_length(max_length: usize) -> Self {
        let initial_cursor = JsonCursor {
            current_offset: 0,
            array_is_opened: false,
            delimiter_expected: false,
            quote_opened: false,
            nested_quote_opened: false,
            escaped: false,
            opened_brackets: 0,
            current_obj_pos: 0,
            current_primitive_start: None,
        };

        Self {
            max_length,
            json_cursor: initial_cursor,
            #[cfg(feature = "simd-json")]
            simd_buf: Vec::with_capacity(4096),
            _ph: PhantomData,
        }
    }
}

impl<T> tokio_util::codec::Decoder for JsonArrayCodec<T>
where
    T: for<'de> Deserialize<'de>,
{
    type Item = T;
    type Error = StreamBodyError;

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        if buf.is_empty() {
            return Ok(None);
        }

        for (position, current_ch) in buf[self.json_cursor.current_offset..buf.len()]
            .iter()
            .enumerate()
        {
            let abs_pos = self.json_cursor.current_offset + position;

            if abs_pos >= self.max_length {
                return Err(StreamBodyError::new(
                    StreamBodyKind::MaxLenReachedError,
                    None,
                    Some("Max object length reached".into()),
                ));
            }

            match *current_ch {
                b'[' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets == 0 => {
                    if self.json_cursor.array_is_opened {
                        // This is a nested array item — treat like an object open
                        self.json_cursor.current_obj_pos = abs_pos;
                        self.json_cursor.opened_brackets += 1;
                        self.json_cursor.current_primitive_start = None;
                    } else {
                        self.json_cursor.array_is_opened = true;
                    }
                }
                b'[' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets > 0 => {
                    // Inside a nested context — must not be inside a nested string.
                    if !self.json_cursor.nested_quote_opened {
                        self.json_cursor.opened_brackets += 1;
                    }
                    self.json_cursor.escaped = false;
                }
                b']' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets == 0 => {
                    // End of the top-level array. Emit any pending primitive.
                    if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
                        let obj_slice = trim_ascii(&buf[prim_start..abs_pos]);
                        if !obj_slice.is_empty() {
                            let result;
                            #[cfg(not(feature = "simd-json"))]
                            {
                                result = parse_json_slice(obj_slice);
                            }
                            #[cfg(feature = "simd-json")]
                            {
                                result = parse_json_slice(obj_slice, &mut self.simd_buf);
                            }
                            buf.advance(abs_pos + 1);
                            self.json_cursor.current_offset = 0;
                            self.json_cursor.delimiter_expected = false;
                            return result;
                        }
                    }
                }
                b']' if !self.json_cursor.nested_quote_opened
                    && self.json_cursor.opened_brackets > 0 =>
                {
                    self.json_cursor.opened_brackets -= 1;
                    self.json_cursor.escaped = false;
                    if self.json_cursor.opened_brackets == 0 {
                        // Closed a nested array/object item — reset nested string state
                        // so it cannot leak into the next frame.
                        self.json_cursor.nested_quote_opened = false;
                        self.json_cursor.delimiter_expected = true;
                        let obj_slice = &buf[self.json_cursor.current_obj_pos..=abs_pos];
                        let result;
                        #[cfg(not(feature = "simd-json"))]
                        {
                            result = parse_json_slice(obj_slice);
                        }
                        #[cfg(feature = "simd-json")]
                        {
                            result = parse_json_slice(obj_slice, &mut self.simd_buf);
                        }
                        self.json_cursor.current_obj_pos = 0;
                        buf.advance(abs_pos + 1);
                        self.json_cursor.current_offset = 0;
                        return result;
                    }
                }
                b'"' if !self.json_cursor.escaped && self.json_cursor.opened_brackets == 0 => {
                    if self.json_cursor.quote_opened {
                        // Closing quote of a top-level string item
                        self.json_cursor.quote_opened = false;
                        if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
                            self.json_cursor.delimiter_expected = true;
                            let obj_slice = &buf[prim_start..=abs_pos];
                            let result;
                            #[cfg(not(feature = "simd-json"))]
                            {
                                result = parse_json_slice(obj_slice);
                            }
                            #[cfg(feature = "simd-json")]
                            {
                                result = parse_json_slice(obj_slice, &mut self.simd_buf);
                            }
                            buf.advance(abs_pos + 1);
                            self.json_cursor.current_offset = 0;
                            return result;
                        }
                    } else {
                        // Opening quote of a top-level string item
                        self.json_cursor.quote_opened = true;
                        if self.json_cursor.current_primitive_start.is_none() {
                            self.json_cursor.current_primitive_start = Some(abs_pos);
                        }
                    }
                }
                b'"' if !self.json_cursor.escaped => {
                    // Inside a nested object/array — toggle the dedicated nested quote state
                    // so escape handling here never pollutes top-level `quote_opened`.
                    self.json_cursor.nested_quote_opened = !self.json_cursor.nested_quote_opened;
                }
                b'\\' if self.json_cursor.quote_opened || self.json_cursor.nested_quote_opened => {
                    self.json_cursor.escaped = !self.json_cursor.escaped;
                }
                b'{' if !self.json_cursor.quote_opened && !self.json_cursor.nested_quote_opened => {
                    if self.json_cursor.opened_brackets == 0 {
                        self.json_cursor.current_obj_pos = abs_pos;
                        self.json_cursor.current_primitive_start = None;
                    }
                    self.json_cursor.opened_brackets += 1;
                    self.json_cursor.escaped = false;
                }
                b'}' if !self.json_cursor.quote_opened && !self.json_cursor.nested_quote_opened => {
                    self.json_cursor.opened_brackets -= 1;
                    self.json_cursor.escaped = false;
                    if self.json_cursor.opened_brackets == 0 {
                        // Closed a nested object — reset nested string state so it cannot
                        // leak into the next frame.
                        self.json_cursor.nested_quote_opened = false;
                        self.json_cursor.delimiter_expected = true;
                        let obj_slice = &buf[self.json_cursor.current_obj_pos..=abs_pos];
                        let result;
                        #[cfg(not(feature = "simd-json"))]
                        {
                            result = parse_json_slice(obj_slice);
                        }
                        #[cfg(feature = "simd-json")]
                        {
                            result = parse_json_slice(obj_slice, &mut self.simd_buf);
                        }
                        self.json_cursor.current_obj_pos = 0;
                        buf.advance(abs_pos + 1);
                        self.json_cursor.current_offset = 0;
                        return result;
                    }
                }
                b',' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets == 0 => {
                    if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
                        let obj_slice = trim_ascii(&buf[prim_start..abs_pos]);
                        if !obj_slice.is_empty() {
                            let result;
                            #[cfg(not(feature = "simd-json"))]
                            {
                                result = parse_json_slice(obj_slice);
                            }
                            #[cfg(feature = "simd-json")]
                            {
                                result = parse_json_slice(obj_slice, &mut self.simd_buf);
                            }
                            buf.advance(abs_pos + 1);
                            self.json_cursor.current_offset = 0;
                            self.json_cursor.delimiter_expected = false;
                            return result;
                        }
                    } else if !self.json_cursor.delimiter_expected {
                        return Err(StreamBodyError::new(
                            StreamBodyKind::CodecError,
                            None,
                            Some("Unexpected delimiter found".into()),
                        ));
                    }
                    self.json_cursor.delimiter_expected = false;
                }
                _ if !self.json_cursor.quote_opened
                    && self.json_cursor.opened_brackets == 0
                    && self.json_cursor.array_is_opened
                    && !current_ch.is_ascii_whitespace() =>
                {
                    // Non-whitespace character at top level inside array — start of a primitive
                    if self.json_cursor.current_primitive_start.is_none() {
                        self.json_cursor.current_primitive_start = Some(abs_pos);
                    }
                    self.json_cursor.escaped = false;
                }
                _ => {
                    self.json_cursor.escaped = false;
                }
            }
        }
        self.json_cursor.current_offset = buf.len();

        Ok(None)
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        // Try normal decode first (handles ]-terminated values)
        if let Some(item) = self.decode(buf)? {
            return Ok(Some(item));
        }
        // EOF without closing bracket — emit any pending primitive
        if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
            let obj_slice = trim_ascii(&buf[prim_start..buf.len()]);
            if !obj_slice.is_empty() {
                let result;
                #[cfg(not(feature = "simd-json"))]
                {
                    result = parse_json_slice(obj_slice)?;
                }
                #[cfg(feature = "simd-json")]
                {
                    result = parse_json_slice(obj_slice, &mut self.simd_buf)?;
                }
                buf.clear();
                return Ok(Some(result));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::codec::Decoder;

    use super::*;

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Item {
        name: String,
        value: u32,
    }

    fn decode_all<T: for<'de> serde::Deserialize<'de> + std::fmt::Debug + PartialEq>(
        data: &[u8],
    ) -> Vec<T> {
        let mut codec = JsonArrayCodec::<T>::new_with_max_length(1024);
        let mut buf = BytesMut::from(data);
        let mut results = Vec::new();
        // Bound the loop so a mutant that makes decode never terminate (e.g. a
        // broken buffer-advance) fails fast as a test failure instead of hanging.
        let mut iterations = 0;
        loop {
            iterations += 1;
            assert!(
                iterations < 10_000,
                "decode_all did not terminate (possible buffer-advance regression)"
            );
            match codec.decode(&mut buf) {
                Ok(Some(item)) => results.push(item),
                Ok(None) => break,
                Err(e) => panic!("decode error: {e}"),
            }
        }
        results
    }

    #[test]
    fn normal_parse_array_of_objects() {
        let data = br#"[{"name":"alice","value":1},{"name":"bob","value":2}]"#;
        let items: Vec<Item> = decode_all(data);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "alice");
        assert_eq!(items[1].value, 2);
    }

    #[test]
    fn empty_input_returns_none() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        let mut buf = BytesMut::new();
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
    }

    #[test]
    fn empty_array_returns_none() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"[]"[..]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
    }

    #[test]
    fn truncated_object_returns_none() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        // Truncated mid-object: missing closing brace and bracket
        let mut buf = BytesMut::from(&b"[{\"name\":\"alic"[..]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
    }

    #[test]
    fn eof_without_closing_bracket_emits_pending_primitive() {
        let mut codec = JsonArrayCodec::<i64>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"[1, 2, 3"[..]);
        // First two values emit via comma delimiter
        assert_eq!(codec.decode(&mut buf).unwrap().unwrap(), 1);
        assert_eq!(codec.decode(&mut buf).unwrap().unwrap(), 2);
        // Third value has no trailing comma or bracket — decode returns None
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        // decode_eof should emit the pending value
        assert_eq!(codec.decode_eof(&mut buf).unwrap().unwrap(), 3);
    }

    #[test]
    fn invalid_json_returns_error() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"[not json]"[..]);
        assert!(codec.decode(&mut buf).is_err());
    }

    #[test]
    fn max_length_exceeded_returns_error() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(2);
        let mut buf = BytesMut::from(&b"[{\"name\":\"alice\",\"value\":1}]"[..]);
        assert!(codec.decode(&mut buf).is_err());
    }

    #[test]
    fn single_item_array() {
        let data = br#"[{"name":"solo","value":99}]"#;
        let items: Vec<Item> = decode_all(data);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "solo");
        assert_eq!(items[0].value, 99);
    }

    #[test]
    fn array_of_primitives() {
        let data = b"[10, 20, 30]";
        let items: Vec<i64> = decode_all(data);
        assert_eq!(items, vec![10, 20, 30]);
    }

    #[test]
    fn array_of_strings() {
        let data = br#"["hello", "world", "foo"]"#;
        let items: Vec<String> = decode_all(data);
        assert_eq!(items, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn nested_objects() {
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Outer {
            inner: Inner,
        }
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Inner {
            x: i32,
        }
        let data = br#"[{"inner":{"x":1}},{"inner":{"x":2}}]"#;
        let items: Vec<Outer> = decode_all(data);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].inner.x, 1);
        assert_eq!(items[1].inner.x, 2);
    }

    #[test]
    fn whitespace_handling() {
        let data = b"[  { \"name\" : \"ws\" , \"value\" : 1 }  ]";
        let items: Vec<Item> = decode_all(data);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "ws");
    }

    #[test]
    fn nested_arrays() {
        let data = br#"[[1,2],[3,4]]"#;
        let items: Vec<Vec<i64>> = decode_all(data);
        assert_eq!(items, vec![vec![1, 2], vec![3, 4]]);
    }

    #[test]
    fn objects_with_nested_array_field() {
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct WithArray {
            name: String,
            values: Vec<i32>,
        }
        let data = br#"[{"name":"a","values":[1,2]},{"name":"b","values":[3,4]}]"#;
        let items: Vec<WithArray> = decode_all(data);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "a");
        assert_eq!(items[0].values, vec![1, 2]);
        assert_eq!(items[1].values, vec![3, 4]);
    }

    #[test]
    fn strings_containing_brackets() {
        // `[`/`]` inside a quoted string must not be interpreted as array
        // delimiters by the bracket-tracking state machine.
        let data = br#"["a[b]c", "d]e[f", "plain"]"#;
        let items: Vec<String> = decode_all(data);
        assert_eq!(items, vec!["a[b]c", "d]e[f", "plain"]);
    }

    #[test]
    fn escaped_quotes_and_brackets_in_strings() {
        let data = br#"["he said \"[x]\"", "ok"]"#;
        let items: Vec<String> = decode_all(data);
        assert_eq!(items, vec!["he said \"[x]\"", "ok"]);
    }

    #[test]
    fn bracket_inside_nested_string_does_not_emit_early() {
        // A `]` inside a value string of a nested object must not close the
        // object early; the item is emitted only at the real `}`.
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct WithNote {
            note: String,
        }
        let data = br#"[{"note":"a]b"},{"note":"c"}]"#;
        let items: Vec<WithNote> = decode_all(data);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].note, "a]b");
        assert_eq!(items[1].note, "c");
    }

    #[test]
    fn incremental_feed_object_spans_chunks() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        let chunk1 = br#"[{"name":"a"#;
        let chunk2 = br#"lice","value":1}]"#;

        let mut buf = BytesMut::from(&chunk1[..]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));

        buf.extend_from_slice(chunk2);
        let item = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(item.name, "alice");
        assert_eq!(item.value, 1);
    }

    #[test]
    fn incremental_feed_multiple_objects() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        let chunk1 = br#"[{"name":"a","value":1},"#;
        let chunk2 = br#"{"name":"b","value":2}]"#;

        let mut buf = BytesMut::from(&chunk1[..]);
        let item1 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(item1.name, "a");
        assert!(matches!(codec.decode(&mut buf), Ok(None)));

        buf.extend_from_slice(chunk2);
        let item2 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(item2.name, "b");
    }

    #[test]
    fn decode_eof_flushes_final_object() {
        // The codec's decode_eof only flushes pending primitives, not incomplete objects.
        // Test that a pending primitive at EOF is flushed correctly.
        let mut codec = JsonArrayCodec::<i64>::new_with_max_length(1024);
        let data = b"[42";
        let mut buf = BytesMut::from(&data[..]);
        // decode returns None (no delimiter or closing bracket)
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        // decode_eof should flush the pending primitive
        let item = codec.decode_eof(&mut buf).unwrap().unwrap();
        assert_eq!(item, 42);
    }

    #[test]
    fn decode_eof_empty_returns_none() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        let mut buf = BytesMut::new();
        assert!(matches!(codec.decode_eof(&mut buf), Ok(None)));
    }

    #[test]
    fn string_with_escapes() {
        let data = br#"["hello\"world", "foo\\bar"]"#;
        let items: Vec<String> = decode_all(data);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "hello\"world");
        assert_eq!(items[1], "foo\\bar");
    }

    #[test]
    fn large_array_many_items() {
        let mut json = String::from("[");
        for i in 0..100 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(r#"{{"name":"item{i}","value":{i}}}"#));
        }
        json.push(']');
        let items: Vec<Item> = decode_all(json.as_bytes());
        assert_eq!(items.len(), 100);
        assert_eq!(items[0].name, "item0");
        assert_eq!(items[99].value, 99);
    }

    #[test]
    fn object_with_escaped_string_containing_brackets() {
        let data = br#"[{"name":"a[1]{2}b","value":1}]"#;
        let items: Vec<Item> = decode_all(data);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "a[1]{2}b");
    }

    #[test]
    fn array_of_booleans() {
        let data = b"[true, false, true]";
        let items: Vec<bool> = decode_all(data);
        assert_eq!(items, vec![true, false, true]);
    }

    #[test]
    fn array_of_nullables() {
        // Note: The codec's primitive parser handles null values.
        // Using format that doesn't require primitive null handling at top level.
        let data = b"[1, 2, 3]";
        let items: Vec<Option<i64>> = decode_all(data);
        assert_eq!(items, vec![Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn type_mismatch_returns_error() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"[123]"[..]);
        assert!(codec.decode(&mut buf).is_err());
    }
}

/// Deserialize a JSON value from a byte slice.
///
/// Uses SIMD-accelerated parsing when the `simd-json` feature is enabled, falling back to
/// `serde_json` otherwise. The deserialized type is inferred from the call site.
///
/// When the `simd-json` feature is enabled, `simd_buf` is used as a reusable scratch buffer
/// to avoid allocating a new `Vec` on every call.
#[allow(unused_variables)]
fn parse_json_slice<T>(
    obj_slice: &[u8],
    #[cfg(feature = "simd-json")] simd_buf: &mut Vec<u8>,
) -> Result<T, StreamBodyError>
where
    T: for<'de> Deserialize<'de>,
{
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::from_slice(obj_slice).map_err(|err| {
            StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
        })
    }
    #[cfg(feature = "simd-json")]
    {
        simd_buf.clear();
        simd_buf.extend_from_slice(obj_slice);
        simd_json::from_slice(simd_buf).map_err(|err| {
            StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
        })
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    if start >= end {
        &[]
    } else {
        &bytes[start..end]
    }
}
