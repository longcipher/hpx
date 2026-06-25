use std::marker::PhantomData;

use bytes::{Buf, BytesMut};
use serde::Deserialize;

use crate::{StreamBodyError, error::StreamBodyKind};

#[derive(Clone, Debug)]
pub(crate) struct JsonArrayCodec<T> {
    max_length: usize,
    json_cursor: JsonCursor,
    _ph: PhantomData<T>,
}

#[derive(Clone, Debug)]
struct JsonCursor {
    pub current_offset: usize,
    pub array_is_opened: bool,
    pub delimiter_expected: bool,
    pub quote_opened: bool,
    pub escaped: bool,
    pub opened_brackets: usize,
    pub current_obj_pos: usize,
    /// When Some(pos), we are accumulating a primitive value (number/bool/null/string)
    /// that started at `pos` in the buffer. A quoted string also uses this.
    pub current_primitive_start: Option<usize>,
}

impl<T> JsonArrayCodec<T> {
    pub(crate) const fn new_with_max_length(max_length: usize) -> Self {
        let initial_cursor = JsonCursor {
            current_offset: 0,
            array_is_opened: false,
            delimiter_expected: false,
            quote_opened: false,
            escaped: false,
            opened_brackets: 0,
            current_obj_pos: 0,
            current_primitive_start: None,
        };

        Self {
            max_length,
            json_cursor: initial_cursor,
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
                    self.json_cursor.opened_brackets += 1;
                    self.json_cursor.escaped = false;
                }
                b']' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets == 0 => {
                    // End of the top-level array. Emit any pending primitive.
                    if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
                        let obj_slice = trim_ascii(&buf[prim_start..abs_pos]);
                        if !obj_slice.is_empty() {
                            let result = serde_json::from_slice(obj_slice).map_err(|err| {
                                StreamBodyError::new(
                                    StreamBodyKind::CodecError,
                                    Some(Box::new(err)),
                                    None,
                                )
                            });
                            buf.advance(abs_pos + 1);
                            self.json_cursor.current_offset = 0;
                            self.json_cursor.delimiter_expected = false;
                            return result;
                        }
                    }
                }
                b']' if !self.json_cursor.quote_opened && self.json_cursor.opened_brackets > 0 => {
                    self.json_cursor.opened_brackets -= 1;
                    self.json_cursor.escaped = false;
                    if self.json_cursor.opened_brackets == 0 {
                        // Closed a nested array/object item
                        self.json_cursor.delimiter_expected = true;
                        let obj_slice = &buf[self.json_cursor.current_obj_pos..=abs_pos];
                        let result = serde_json::from_slice(obj_slice).map_err(|err| {
                            StreamBodyError::new(
                                StreamBodyKind::CodecError,
                                Some(Box::new(err)),
                                None,
                            )
                        });
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
                            let result = serde_json::from_slice(obj_slice).map_err(|err| {
                                StreamBodyError::new(
                                    StreamBodyKind::CodecError,
                                    Some(Box::new(err)),
                                    None,
                                )
                            });
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
                    // Inside a nested object/array
                    self.json_cursor.quote_opened = !self.json_cursor.quote_opened;
                }
                b'\\' if self.json_cursor.quote_opened => {
                    self.json_cursor.escaped = !self.json_cursor.escaped;
                }
                b'{' if !self.json_cursor.quote_opened => {
                    if self.json_cursor.opened_brackets == 0 {
                        self.json_cursor.current_obj_pos = abs_pos;
                        self.json_cursor.current_primitive_start = None;
                    }
                    self.json_cursor.opened_brackets += 1;
                    self.json_cursor.escaped = false;
                }
                b'}' if !self.json_cursor.quote_opened => {
                    self.json_cursor.opened_brackets -= 1;
                    self.json_cursor.escaped = false;
                    if self.json_cursor.opened_brackets == 0 {
                        self.json_cursor.delimiter_expected = true;
                        let obj_slice = &buf[self.json_cursor.current_obj_pos..=abs_pos];
                        let result = serde_json::from_slice(obj_slice).map_err(|err| {
                            StreamBodyError::new(
                                StreamBodyKind::CodecError,
                                Some(Box::new(err)),
                                None,
                            )
                        });
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
                            let result = serde_json::from_slice(obj_slice).map_err(|err| {
                                StreamBodyError::new(
                                    StreamBodyKind::CodecError,
                                    Some(Box::new(err)),
                                    None,
                                )
                            });
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

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<T>, StreamBodyError> {
        // Try normal decode first (handles ]-terminated values)
        if let Some(item) = self.decode(buf)? {
            return Ok(Some(item));
        }
        // EOF without closing bracket — emit any pending primitive
        if let Some(prim_start) = self.json_cursor.current_primitive_start.take() {
            let obj_slice = trim_ascii(&buf[prim_start..buf.len()]);
            if !obj_slice.is_empty() {
                let result = serde_json::from_slice(obj_slice).map_err(|err| {
                    StreamBodyError::new(
                        StreamBodyKind::CodecError,
                        Some(Box::new(err)),
                        None,
                    )
                })?;
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
        loop {
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
}

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
