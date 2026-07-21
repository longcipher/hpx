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

        frames_reader
            .into_stream()
            .map(|frame_res| match frame_res {
                Ok(frame_str) => serde_json::from_str(frame_str.as_str()).map_err(|err| {
                    StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(err)), None)
                }),
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

#[cfg(test)]
mod tests {
    // JsonStreamResponse is an extension trait on hpx::Response.
    // The codec logic (JsonArrayCodec) is tested in json_array_codec.rs.
    // Integration tests with a live HTTP server are needed to test this module directly.
}
