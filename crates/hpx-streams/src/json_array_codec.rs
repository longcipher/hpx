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

    /// Decode the next array element.
    ///
    /// Every emission site must return `result.map(Some)` rather than
    /// `result` directly. `Ok(None)` is the `Decoder` contract's "buffer is
    /// incomplete, call me again" signal, so returning a bare parse result lets
    /// inference widen the parse target to `Option<T>`. A JSON `null` element
    /// then deserializes to `None` and is silently dropped from the stream
    /// instead of being yielded. Wrapping in `Some` keeps the two meanings
    /// distinct for item types that are themselves optional.
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
                            return result.map(Some);
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
                        return result.map(Some);
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
                            return result.map(Some);
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
                    // A `}` with nothing open is malformed input. Subtracting
                    // from a zero depth panics in debug builds and wraps to
                    // `usize::MAX` in release builds, which would leave the
                    // decoder permanently unable to close a bracket and pin the
                    // whole rest of the response in the buffer. Reject the input
                    // instead: it cannot be parsed as a JSON array element
                    // anyway.
                    if self.json_cursor.opened_brackets == 0 {
                        return Err(StreamBodyError::new(
                            StreamBodyKind::CodecError,
                            None,
                            Some("unbalanced closing brace in JSON array".into()),
                        ));
                    }
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
                        return result.map(Some);
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
                            return result.map(Some);
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
        let data = b"[1, 2, 3]";
        let items: Vec<Option<i64>> = decode_all(data);
        assert_eq!(items, vec![Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn null_elements_are_yielded_not_dropped() {
        // `Ok(None)` means "need more data" in the `Decoder` contract, so a
        // bare `null` element must still be wrapped in `Some`. Returning the
        // parse result unwrapped used to make inference target `Option<T>`,
        // which turned every `null` into a "need more data" signal and dropped
        // the element on the floor.
        let items: Vec<Option<i64>> = decode_all(b"[null,1,null,2]");
        assert_eq!(items, vec![None, Some(1), None, Some(2)]);
    }

    #[test]
    fn leading_and_trailing_nulls_are_yielded() {
        let items: Vec<Option<bool>> = decode_all(b"[null,true,false,null]");
        assert_eq!(items, vec![None, Some(true), Some(false), None]);
    }

    #[test]
    fn null_only_array_yields_every_element() {
        let items: Vec<Option<i64>> = decode_all(b"[null,null,null]");
        assert_eq!(items, vec![None, None, None]);
    }

    #[test]
    fn null_inside_nested_object_is_preserved() {
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Maybe {
            v: Option<i64>,
        }
        let items: Vec<Maybe> = decode_all(br#"[{"v":null},{"v":5}]"#);
        assert_eq!(items, vec![Maybe { v: None }, Maybe { v: Some(5) }]);
    }

    #[test]
    fn null_element_split_across_chunks() {
        // The `null` token straddles two TCP segments, so the primitive start
        // offset has to survive between calls.
        let mut codec = JsonArrayCodec::<Option<i64>>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"[nu"[..]);
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
        buf.extend_from_slice(&b"ll,1]"[..]);
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(None));
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(Some(1)));
    }

    #[test]
    fn unbalanced_closing_brace_is_rejected_not_underflowed() {
        // `opened_brackets` was decremented unconditionally, so a stray `}`
        // underflowed: a panic in debug builds, and `usize::MAX` in release
        // builds, which leaves the decoder permanently unable to close a
        // bracket and pins the rest of the response in its buffer.
        for payload in [&b"}"[..], b"[}]", b"[1}]", b"[,}]"] {
            let mut codec = JsonArrayCodec::<serde_json::Value>::new_with_max_length(1024);
            let mut buf = BytesMut::from(payload);
            assert!(
                codec.decode(&mut buf).is_err(),
                "unbalanced `}}` in {payload:?} must be rejected, not underflowed"
            );
        }
    }

    #[test]
    fn balanced_braces_still_decode() {
        // The unbalanced-brace guard must not reject well-formed nesting.
        let items: Vec<serde_json::Value> = decode_all(br#"[{"a":{"b":1}},{"c":2}]"#);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["a"]["b"], 1);
        assert_eq!(items[1]["c"], 2);
    }

    #[test]
    fn deep_nesting_does_not_overflow() {
        // Each `{` increments the depth; make sure a long run of openers is
        // handled without wrapping.
        let depth = 512;
        let mut payload = String::from("[");
        for _ in 0..depth {
            payload.push('{');
        }
        let mut codec = JsonArrayCodec::<serde_json::Value>::new_with_max_length(1024 * 1024);
        let mut buf = BytesMut::from(payload.as_bytes());
        assert!(matches!(codec.decode(&mut buf), Ok(None)));
    }

    #[test]
    fn unterminated_top_level_string_yields_nothing() {
        // A `]` seen while a top-level string is still open is data, not a
        // delimiter. If the guard let it through, the pending value would be
        // emitted mid-string and the consumer would receive a corrupt item
        // instead of waiting for the string to close.
        let mut codec = JsonArrayCodec::<String>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"[\"abc]"[..]);
        assert!(
            matches!(codec.decode(&mut buf), Ok(None)),
            "an unterminated string must not be emitted"
        );
    }

    #[test]
    fn backslash_outside_a_string_is_rejected_not_treated_as_a_value_prefix() {
        // The backslash arm must only apply while a string is open. A `\` at
        // top level is not JSON, and it must be framed as part of the value so
        // the element is rejected. Treating it as an escape outside a string
        // would silently drop it, and `1` would then decode as a bare value
        // even though the element it belongs to is malformed.
        //
        // Bytes: `[`, `\`, `1`, `]`.
        let mut codec = JsonArrayCodec::<i64>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"[\\1]"[..]);
        assert!(
            codec.decode(&mut buf).is_err(),
            "a backslash outside a string must not be skipped"
        );
    }

    #[test]
    fn escaped_quote_inside_nested_string_is_handled() {
        // `\\"` inside a nested value must not close the nested string, so the
        // `}` that follows it is the real object terminator.
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Note {
            note: String,
        }
        let data = br#"[{"note":"a\"b"},{"note":"c"}]"#;
        let items: Vec<Note> = decode_all(data);
        assert_eq!(
            items,
            vec![
                Note {
                    note: "a\"b".into()
                },
                Note { note: "c".into() },
            ]
        );
    }

    #[test]
    fn type_mismatch_returns_error() {
        let mut codec = JsonArrayCodec::<Item>::new_with_max_length(1024);
        let mut buf = BytesMut::from(&b"[123]"[..]);
        assert!(codec.decode(&mut buf).is_err());
    }

    /// Property tests for the invariants that must hold for every input.
    ///
    /// The codec is a hand-written incremental state machine, so example-based
    /// tests cannot enumerate the input space. These properties let `proptest`
    /// shrink a failure to a minimal reproducer, and they run as part of the
    /// ordinary `cargo test` loop.
    mod properties {
        use bytes::BytesMut;
        use proptest::prelude::*;
        use tokio_util::codec::Decoder;

        use super::JsonArrayCodec;

        /// Absolute step bound for the chunk-feeding helpers.
        ///
        /// Kept low on purpose: mutation testing injects regressions that stop
        /// making progress, and a bound in the hundreds of thousands turns
        /// every such mutant into a 25s timeout instead of a fast failure.
        /// A few thousand steps is still far more than any realistic payload
        /// needs, since each step either delivers a chunk or emits an item.
        const MAX_HELPER_STEPS: usize = 4_096;

        /// Strings built from JSON metacharacters, which is what the
        /// bracket/quote/escape tracking in the codec exists to handle.
        fn nasty_string() -> impl Strategy<Value = String> {
            let pieces = prop::sample::select(vec![
                "a",
                "b",
                "0",
                "1",
                " ",
                "\t",
                "\n",
                "[",
                "]",
                "{",
                "}",
                "\"",
                "\\",
                "/",
                "\u{e9}",
                "\u{1F600}",
            ]);
            prop::collection::vec(pieces, 0..12).prop_map(|parts| parts.concat())
        }

        fn string_array() -> impl Strategy<Value = Vec<String>> {
            prop::collection::vec(nasty_string(), 0..10)
        }

        /// Decode `data` to exhaustion, feeding `chunk` bytes per `decode` call.
        ///
        /// Returns `None` as soon as the decoder reports an error, so callers
        /// can distinguish "decoded cleanly" from "rejected". An unterminated
        /// trailing value is dropped, matching the codec's documented
        /// behaviour.
        fn decode_chunked<T>(data: &[u8], chunk: usize) -> Option<Vec<T>>
        where
            T: for<'de> serde::Deserialize<'de> + std::fmt::Debug,
        {
            let mut codec = JsonArrayCodec::<T>::new_with_max_length(1024 * 1024);
            let mut buf = BytesMut::new();
            let mut out = Vec::new();
            let mut fed = 0usize;

            // Absolute bound so a regression that stops making progress fails
            // loudly instead of hanging the suite.
            for _ in 0..MAX_HELPER_STEPS {
                if fed < data.len() {
                    let take = chunk.min(data.len() - fed);
                    buf.extend_from_slice(&data[fed..fed + take]);
                    fed += take;
                }
                match codec.decode(&mut buf) {
                    Ok(Some(item)) => {
                        out.push(item);
                        continue;
                    }
                    Ok(None) => {}
                    Err(_) => return None,
                }
                if fed == data.len() {
                    // All input delivered: flush what the EOF handler yields.
                    // `decode_eof` returns one item per call, so keep going
                    // until it reports a clean end. A trailing partial value is
                    // intentionally discarded.
                    match codec.decode_eof(&mut buf) {
                        Ok(Some(item)) => {
                            out.push(item);
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

        /// Render a JSON array literal from generated values, with structural
        /// whitespace inserted at the top level only.
        ///
        /// `pad` controls the whitespace emitted between structural tokens;
        /// string *contents* are escaped by `serde_json` and never touched, so
        /// generated values containing `[`, `]`, `,`, quotes, or newlines
        /// cannot corrupt the payload.
        fn encode_string_array(values: &[String], pad: &str) -> Vec<u8> {
            let mut out = Vec::from(&b"["[..]);
            out.extend_from_slice(pad.as_bytes());
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(pad.as_bytes());
                    out.push(b',');
                    out.extend_from_slice(pad.as_bytes());
                }
                out.extend_from_slice(
                    serde_json::to_vec(value)
                        .expect("serializing a String cannot fail")
                        .as_slice(),
                );
            }
            out.extend_from_slice(pad.as_bytes());
            out.push(b']');
            out
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]

            /// A JSON array of generated strings decodes back to exactly those
            /// strings, in order, no matter how the bytes are chunked.
            #[test]
            fn string_array_round_trips(values in string_array(), chunk in 1usize..32) {
                let payload = encode_string_array(&values, "");
                let decoded = decode_chunked::<String>(&payload, chunk);
                prop_assert_eq!(
                    decoded.as_deref(),
                    Some(values.as_slice()),
                    "failed for payload {:?} at chunk size {}",
                    String::from_utf8_lossy(&payload),
                    chunk
                );
            }

            /// Decoding is independent of the chunk size, for arrays of nested
            /// objects carrying escaped strings.
            #[test]
            fn nested_object_array_is_chunk_independent(
                labels in prop::collection::vec(nasty_string(), 1..6),
                chunk in 1usize..40,
            ) {
                // Build the payload with `serde_json` so the nesting and the
                // escaping are guaranteed well-formed.
                let doc: Vec<serde_json::Value> = labels
                    .iter()
                    .enumerate()
                    .map(|(i, label)| {
                        serde_json::json!({ "label": label, "n": i, "nested": [1, 2, 3] })
                    })
                    .collect();
                let payload = serde_json::to_vec(&doc).expect("json value serializes");

                let decoded = decode_chunked::<serde_json::Value>(&payload, chunk)
                    .expect("a well-formed array decodes at every chunk size");
                prop_assert_eq!(decoded.len(), labels.len());
                for (i, value) in decoded.iter().enumerate() {
                    prop_assert_eq!(&value["label"], labels[i].as_str());
                    prop_assert_eq!(value["n"].as_u64(), Some(i as u64));
                    prop_assert_eq!(value["nested"].as_array().map(Vec::len), Some(3));
                }
            }

            /// Concatenating two arrays decodes to the concatenation. Both sides
            /// are non-empty, because an empty side would make the combined
            /// literal a leading/trailing delimiter rather than a value list.
            #[test]
            fn decoding_is_additive(
                left in prop::collection::vec(nasty_string(), 1..6),
                right in prop::collection::vec(nasty_string(), 1..6),
            ) {
                let mut payload = encode_string_array(&left, "");
                payload.pop(); // drop the closing bracket
                payload.push(b',');
                payload.extend_from_slice(&encode_string_array(&right, "")[1..]);

                let mut expected = left;
                expected.extend(right);
                prop_assert_eq!(
                    decode_chunked::<String>(&payload, 7),
                    Some(expected),
                    "failed for payload {:?}",
                    String::from_utf8_lossy(&payload)
                );
            }

            /// Structural whitespace must not change the decoded sequence.
            /// Whitespace is only inserted between structural tokens, so values
            /// that themselves contain whitespace are unaffected.
            #[test]
            fn structural_whitespace_does_not_change_result(values in string_array()) {
                let compact = encode_string_array(&values, "");
                for pad in [" ", "\n", " \t\n  ", "\r\n"] {
                    let spaced = encode_string_array(&values, pad);
                    prop_assert_eq!(
                        decode_chunked::<String>(&compact, 5),
                        decode_chunked::<String>(&spaced, 5),
                        "whitespace {:?} changed the decoded sequence",
                        pad
                    );
                }
            }

            /// Structural whitespace survives an arbitrary chunk split.
            #[test]
            fn structural_whitespace_survives_chunking(
                values in prop::collection::vec(nasty_string(), 1..6),
                chunk in 1usize..24,
            ) {
                let payload = encode_string_array(&values, " \n\t");
                prop_assert_eq!(decode_chunked::<String>(&payload, chunk), Some(values));
            }

            /// Surrounding whitespace is trimmed, never absorbed into the value.
            #[test]
            fn surrounding_whitespace_is_trimmed(value in nasty_string()) {
                let mut payload = Vec::from(&b"[   \n\t"[..]);
                payload.extend_from_slice(
                    serde_json::to_vec(&value)
                        .expect("string serializes")
                        .as_slice(),
                );
                payload.extend_from_slice(b"   ,  \"tail\"  ]");

                let decoded = decode_chunked::<String>(&payload, 3)
                    .expect("a valid array decodes");
                prop_assert_eq!(decoded.as_slice(), [value.as_str(), "tail"]);
            }

            /// Every `null` element is yielded as an item and never dropped,
            /// whatever the count.
            #[test]
            fn null_elements_are_never_dropped(count in 1usize..12) {
                let mut payload = Vec::from(&b"["[..]);
                for i in 0..count {
                    if i > 0 {
                        payload.push(b',');
                    }
                    payload.extend_from_slice(b"null");
                }
                payload.push(b']');

                let decoded = decode_chunked::<Option<i64>>(&payload, 4)
                    .expect("a valid array decodes");
                prop_assert_eq!(decoded.len(), count);
                prop_assert!(decoded.iter().all(Option::is_none));
            }

            /// Optional elements keep their shape: a mix of values and `null`
            /// preserves both the values and the holes.
            #[test]
            fn optional_elements_keep_their_positions(
                values in prop::collection::vec(prop::option::of(0i64..1000), 1..10),
            ) {
                let mut payload = Vec::from(&b"["[..]);
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        payload.push(b',');
                    }
                    match value {
                        Some(v) => payload.extend_from_slice(v.to_string().as_bytes()),
                        None => payload.extend_from_slice(b"null"),
                    }
                }
                payload.push(b']');
                prop_assert_eq!(decode_chunked::<Option<i64>>(&payload, 3), Some(values));
            }

            /// Arbitrary bytes never panic, and never yield more items than the
            /// payload could possibly contain.
            #[test]
            fn arbitrary_bytes_never_panic(
                data in prop::collection::vec(any::<u8>(), 0..256),
                chunk in 1usize..16,
            ) {
                let bound = data.len() + 1;
                let decoded = decode_chunked::<serde_json::Value>(&data, chunk);
                if let Some(items) = decoded {
                    prop_assert!(items.len() <= bound);
                }
            }

            /// Arbitrary bytes never panic when the item type is itself
            /// optional, which is the case that previously lost `null` elements.
            #[test]
            fn arbitrary_bytes_never_panic_for_optional_items(
                data in prop::collection::vec(any::<u8>(), 0..256),
                chunk in 1usize..16,
            ) {
                let bound = data.len() + 1;
                let decoded = decode_chunked::<Option<serde_json::Value>>(&data, chunk);
                if let Some(items) = decoded {
                    prop_assert!(items.len() <= bound);
                }
            }

            /// A payload whose object exceeds `max_length` is rejected rather
            /// than buffered without bound.
            #[test]
            fn oversized_objects_are_rejected(name in nasty_string()) {
                let doc = serde_json::json!([{ "name": name, "value": 1 }]);
                let payload = serde_json::to_vec(&doc).expect("json value serializes");
                let mut codec = JsonArrayCodec::<serde_json::Value>::new_with_max_length(4);
                let mut buf = BytesMut::from(payload.as_slice());
                prop_assert!(codec.decode(&mut buf).is_err());
            }
        }
    }
}

/// Deserialize a JSON value from a byte slice.
///
/// Uses SIMD-accelerated parsing when the `simd-json` feature is enabled, falling back to
/// `serde_json` otherwise. The deserialized type is inferred from the call site.
///
/// When the `simd-json` feature is enabled, `simd_buf` is used as a reusable scratch buffer
/// to avoid allocating a new `Vec` on every call.
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
