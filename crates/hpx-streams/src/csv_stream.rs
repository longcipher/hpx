use crate::error::StreamBodyKind;
use crate::{StreamBodyError, StreamBodyResult};
use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use tokio_util::io::StreamReader;

/// Extension trait for [`hpx::Response`] that provides streaming support for the CSV format.
#[async_trait]
pub trait CsvStreamResponse {
    /// Streams the response as CSV, where each line is a CSV row.
    ///
    /// The stream will [`Deserialize`] entries as type `T` with a maximum size of `max_obj_len`
    /// bytes. If `max_obj_len` is [`usize::MAX`], lines will be read until a newline (`\n`)
    /// character is reached.
    ///
    /// If `with_csv_header` is `true`, the stream will skip the first row (the CSV header).
    ///
    /// The `delimiter` is the byte value of the delimiter character.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use futures::stream::BoxStream as _;
    /// use hpx_streams::CsvStreamResponse as _;
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Clone, Deserialize)]
    /// struct MyTestStructure {
    ///     some_test_field: String
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     const MAX_OBJ_LEN: usize = 64 * 1024;
    ///
    ///     let client = hpx::Client::new()?;
    ///     let _stream = client.get("http://localhost:8080/csv")
    ///         .send()
    ///         .await?
    ///         .csv_stream::<MyTestStructure>(MAX_OBJ_LEN, true, b',');
    ///
    ///     Ok(())
    /// }
    /// ```
    fn csv_stream<T>(
        self,
        max_obj_len: usize,
        with_csv_header: bool,
        delimiter: u8,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de>;
}

#[async_trait]
impl CsvStreamResponse for hpx::Response {
    fn csv_stream<T>(
        self,
        max_obj_len: usize,
        with_csv_header: bool,
        delimiter: u8,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de>,
    {
        let reader = StreamReader::new(
            self.bytes_stream()
                .map_err(std::io::Error::other),
        );

        let codec = tokio_util::codec::LinesCodec::new_with_max_length(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

        #[expect(clippy::bool_to_int_with_if)]
        let skip_header_if_expected = if with_csv_header { 1 } else { 0 };

        frames_reader
            .into_stream()
            .skip(skip_header_if_expected)
            .map(move |frame_res| match frame_res {
                Ok(frame_str) => {
                    let mut csv_reader = csv::ReaderBuilder::new()
                        .delimiter(delimiter)
                        .has_headers(false)
                        .from_reader(frame_str.as_bytes());

                    let mut iter = csv_reader.deserialize::<T>();

                    if let Some(csv_res) = iter.next() {
                        match csv_res {
                            Ok(result) => Ok(result),
                            Err(err) => Err(StreamBodyError::new(
                                StreamBodyKind::CodecError,
                                Some(Box::new(err)),
                                None,
                            )),
                        }
                    } else {
                        Err(StreamBodyError::new(StreamBodyKind::CodecError, None, None))
                    }
                }
                Err(err) => Err(StreamBodyError::new(
                    StreamBodyKind::CodecError,
                    Some(Box::new(err)),
                    None,
                )),
            })
    }
}
