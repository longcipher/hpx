use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use tokio_util::io::StreamReader;

use crate::{
    StreamBodyError, StreamBodyResult, error::StreamBodyKind, json_array_codec::JsonArrayCodec,
};

/// Extension trait for [`hpx::Response`] that provides streaming support for the JSON array
/// and JSON Lines (NL/NewLines) formats.
pub trait JsonStreamResponse {
    /// Streams the response as a JSON array.
    ///
    /// The stream will [`Deserialize`] entries as type `T` with a maximum size of `max_obj_len`
    /// bytes. If `max_obj_len` is [`usize::MAX`], lines will be read until a newline (`\n`)
    /// character is reached.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::stream::BoxStream as _;
    /// use hpx_streams::JsonStreamResponse as _;
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Clone, Deserialize)]
    /// struct MyTestStructure {
    ///     some_test_field: String,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     const MAX_OBJ_LEN: usize = 64 * 1024;
    ///
    ///     let client = hpx::Client::new()?;
    ///     let _stream = client
    ///         .get("http://localhost:8080/json-array")
    ///         .send()
    ///         .await?
    ///         .json_array_stream::<MyTestStructure>(MAX_OBJ_LEN);
    ///
    ///     Ok(())
    /// }
    /// ```
    fn json_array_stream<T>(
        self,
        max_obj_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de> + Send;

    /// Streams the response as a JSON array with a custom initial buffer capacity.
    ///
    /// `buf_capacity` is the initial capacity of the stream's decoding buffer.
    fn json_array_stream_with_capacity<T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de> + Send;

    /// Streams the response as JSON lines (NL/NewLines), where each line contains a JSON object.
    ///
    /// The stream will [`Deserialize`] entries as type `T` with a maximum size of `max_obj_len`
    /// bytes. If `max_obj_len` is [`usize::MAX`], lines will be read until a newline (`\n`)
    /// character is reached.
    fn json_nl_stream<T>(
        self,
        max_obj_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de> + Send;

    /// Streams the response as JSON lines (NL/NewLines) with a custom initial buffer capacity.
    fn json_nl_stream_with_capacity<T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de> + Send;
}

const INITIAL_CAPACITY: usize = 8 * 1024;

impl JsonStreamResponse for hpx::Response {
    fn json_nl_stream<T>(
        self,
        max_obj_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de> + Send,
    {
        self.json_nl_stream_with_capacity(max_obj_len, INITIAL_CAPACITY)
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn json_nl_stream_with_capacity<T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de> + Send,
    {
        let reader = StreamReader::new(self.bytes_stream().map_err(std::io::Error::other));

        let codec = tokio_util::codec::LinesCodec::new_with_max_length(max_obj_len);
        let frames_reader =
            tokio_util::codec::FramedRead::with_capacity(reader, codec, buf_capacity);

        // Reusable buffer for simd-json to avoid per-line allocation.
        #[cfg(feature = "simd-json")]
        let mut simd_buf = Vec::with_capacity(4096);

        frames_reader
            .into_stream()
            .map(move |frame_res| match frame_res {
                Ok(frame_str) => parse_json_line(
                    frame_str.as_str(),
                    #[cfg(feature = "simd-json")]
                    &mut simd_buf,
                ),
                Err(err) => Err(StreamBodyError::new(
                    StreamBodyKind::CodecError,
                    Some(Box::new(err)),
                    None,
                )),
            })
    }

    fn json_array_stream<T>(
        self,
        max_obj_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de> + Send,
    {
        self.json_array_stream_with_capacity(max_obj_len, INITIAL_CAPACITY)
    }

    fn json_array_stream_with_capacity<T>(
        self,
        max_obj_len: usize,
        buf_capacity: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de> + Send,
    {
        let reader = StreamReader::new(self.bytes_stream().map_err(std::io::Error::other));

        let codec = JsonArrayCodec::<T>::new_with_max_length(max_obj_len);
        let frames_reader =
            tokio_util::codec::FramedRead::with_capacity(reader, codec, buf_capacity);

        frames_reader.into_stream()
    }
}

/// Deserialize a JSON value from a line of text.
///
/// Uses SIMD-accelerated parsing when the `simd-json` feature is enabled, falling back to
/// `serde_json` otherwise.
///
/// When the `simd-json` feature is enabled, `simd_buf` is used as a reusable scratch buffer
/// to avoid allocating a new `Vec` on every call.
#[allow(unused_variables)]
fn parse_json_line<T>(
    s: &str,
    #[cfg(feature = "simd-json")] simd_buf: &mut Vec<u8>,
) -> Result<T, StreamBodyError>
where
    T: for<'de> Deserialize<'de>,
{
    #[cfg(not(feature = "simd-json"))]
    {
        serde_json::from_str(s).map_err(|err| {
            StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
        })
    }
    #[cfg(feature = "simd-json")]
    {
        simd_buf.clear();
        simd_buf.extend_from_slice(s.as_bytes());
        simd_json::from_slice(simd_buf).map_err(|err| {
            StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Item {
        name: String,
        value: u32,
    }

    /// Helper that works with both the `simd-json` and non-simd-json features.
    #[allow(unused_variables)]
    fn test_parse<T: for<'de> serde::Deserialize<'de>>(s: &str) -> Result<T, StreamBodyError> {
        #[cfg(not(feature = "simd-json"))]
        {
            parse_json_line(s)
        }
        #[cfg(feature = "simd-json")]
        {
            let mut buf = Vec::with_capacity(4096);
            parse_json_line(s, &mut buf)
        }
    }

    #[test]
    fn parse_json_line_valid_object() {
        let line = r#"{"name":"alice","value":1}"#;
        let result: Item = test_parse(line).unwrap();
        assert_eq!(result.name, "alice");
        assert_eq!(result.value, 1);
    }

    #[test]
    fn parse_json_line_invalid_json() {
        let line = "not json at all";
        let result: Result<Item, _> = test_parse(line);
        assert!(result.is_err());
    }

    #[test]
    fn parse_json_line_empty_object() {
        let line = "{}";
        let result: serde_json::Value = test_parse(line).unwrap();
        assert_eq!(result, serde_json::json!({}));
    }

    #[test]
    fn parse_json_line_nested_object() {
        let line = r#"{"name":"nested","value":42,"extra":{"key":"val"}}"#;
        let result: serde_json::Value = test_parse(line).unwrap();
        assert_eq!(result["name"], "nested");
        assert_eq!(result["extra"]["key"], "val");
    }

    #[test]
    fn parse_json_line_array_value() {
        let line = r#"[1, 2, 3]"#;
        let result: Vec<u32> = test_parse(line).unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn parse_json_line_string_value() {
        let line = r#""hello""#;
        let result: String = test_parse(line).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn parse_json_line_number_value() {
        let line = "12345";
        let result: i64 = test_parse(line).unwrap();
        assert_eq!(result, 12345);
    }

    #[test]
    fn parse_json_line_boolean_value() {
        let line = "true";
        let result: bool = test_parse(line).unwrap();
        assert!(result);
    }

    #[test]
    fn parse_json_line_null_value() {
        let line = "null";
        let result: Option<String> = test_parse(line).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_json_line_empty_string_returns_error() {
        let result: Result<Item, _> = test_parse("");
        assert!(result.is_err());
    }

    #[test]
    fn parse_json_line_type_mismatch_returns_error() {
        let result: Result<Item, _> = test_parse("123");
        assert!(result.is_err());
    }

    #[test]
    fn parse_json_line_string_with_escapes() {
        let line = r#""hello\nworld""#;
        let result: String = test_parse(line).unwrap();
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn error_display_shows_kind() {
        let result: Result<Item, _> = test_parse("bad");
        let err = result.unwrap_err();
        let display = format!("{err}");
        assert!(display.contains("Frame/codec error"));
    }

    #[test]
    fn error_kind_accessor() {
        let result: Result<Item, _> = test_parse("bad");
        let err = result.unwrap_err();
        assert!(matches!(err.kind(), StreamBodyKind::CodecError));
    }

    #[test]
    fn initial_capacity_is_8kib() {
        // Pin the streaming buffer size: 8 KiB (8 * 1024 bytes).
        assert_eq!(INITIAL_CAPACITY, 8 * 1024);
    }
}
