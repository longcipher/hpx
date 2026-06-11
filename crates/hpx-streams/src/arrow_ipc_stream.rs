use crate::arrow_ipc_len_codec::ArrowIpcCodec;
use crate::StreamBodyResult;
use arrow::array::RecordBatch;
use async_trait::async_trait;
use futures::TryStreamExt;

/// Extension trait for [`hpx::Response`] that provides streaming support for the [Apache Arrow
/// IPC format].
///
/// [Apache Arrow IPC format]: https://arrow.apache.org/docs/format/Columnar.html#serialization-and-interprocess-communication-ipc
#[async_trait]
pub trait ArrowIpcStreamResponse {
    /// Streams the response as batches of Arrow IPC messages.
    ///
    /// The stream will deserialize entries into [`RecordBatch`]es with a maximum object size of
    /// `max_obj_len` bytes.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use arrow::array::RecordBatch;
    /// use futures::prelude::*;
    /// use hpx_streams::ArrowIpcStreamResponse as _;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     const MAX_OBJ_LEN: usize = 64 * 1024;
    ///
    ///     let client = hpx::Client::new()?;
    ///     let stream = client.get("http://localhost:8080/arrow")
    ///         .send()
    ///         .await?
    ///         .arrow_ipc_stream(MAX_OBJ_LEN);
    ///     let _items: Vec<RecordBatch> = stream.try_collect().await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    fn arrow_ipc_stream(
        self,
        max_obj_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<RecordBatch>> + Send;
}

#[async_trait]
impl ArrowIpcStreamResponse for hpx::Response {
    fn arrow_ipc_stream(
        self,
        max_obj_len: usize,
    ) -> impl futures::Stream<Item = StreamBodyResult<RecordBatch>> + Send {
        let reader = tokio_util::io::StreamReader::new(
            self.bytes_stream()
                .map_err(std::io::Error::other),
        );

        let codec = ArrowIpcCodec::new_with_max_length(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

        frames_reader.into_stream()
    }
}
