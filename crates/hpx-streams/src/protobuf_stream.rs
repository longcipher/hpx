use async_trait::async_trait;
use futures::TryStreamExt;
use tokio_util::io::StreamReader;

use crate::{StreamBodyResult, protobuf_len_codec::ProtobufLenPrefixCodec};

/// Extension trait for [`hpx::Response`] that provides streaming support for the [Protobuf
/// format].
///
/// [Protobuf format]: https://protobuf.dev/programming-guides/encoding/
#[async_trait]
pub trait ProtobufStreamResponse {
    /// Streams the response as batches of Protobuf messages.
    ///
    /// The stream will deserialize [`prost::Message`]s as type `T` with a maximum size of
    /// `max_obj_len` bytes.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::prelude::*;
    /// use hpx_streams::ProtobufStreamResponse as _;
    ///
    /// #[derive(Clone, prost::Message)]
    /// struct MyTestStructure {
    ///     #[prost(string, tag = "1")]
    ///     some_test_field: String,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     const MAX_OBJ_LEN: usize = 64 * 1024;
    ///
    ///     let client = hpx::Client::new()?;
    ///     let stream = client
    ///         .get("http://localhost:8080/protobuf")
    ///         .send()
    ///         .await?
    ///         .protobuf_stream::<MyTestStructure>(MAX_OBJ_LEN);
    ///     let _items: Vec<MyTestStructure> = stream.try_collect().await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    fn protobuf_stream<T>(
        self,
        max_obj_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: prost::Message + Default + Send;
}

#[async_trait]
impl ProtobufStreamResponse for hpx::Response {
    fn protobuf_stream<T>(
        self,
        max_obj_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: prost::Message + Default + Send,
    {
        let reader = StreamReader::new(self.bytes_stream().map_err(std::io::Error::other));

        let codec = ProtobufLenPrefixCodec::<T>::new_with_max_length(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

        frames_reader.into_stream()
    }
}

#[cfg(test)]
mod tests {
    // ProtobufStreamResponse is an extension trait on hpx::Response.
    // The codec logic (ProtobufLenPrefixCodec) is tested in protobuf_len_codec.rs.
    // Integration tests with a live HTTP server are needed to test this module directly.
}
