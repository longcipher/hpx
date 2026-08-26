use bytes::{Buf, BytesMut};
use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use tokio_util::{codec::Decoder, io::StreamReader};

use crate::{StreamBodyError, StreamBodyResult, error::StreamBodyKind};

/// Incremental decoder yielding one complete RFC 4180 CSV record per item.
///
/// Splitting the byte stream on `\n` (as `LinesCodec` does) corrupts records
/// whose quoted fields contain embedded newlines. This decoder tracks quote
/// state across chunk boundaries so a record is only emitted once its closing
/// unquoted newline has been seen.
#[derive(Debug)]
struct CsvRecordCodec {
    max_obj_len: usize,
    buf: Vec<u8>,
    in_quotes: bool,
}

impl CsvRecordCodec {
    fn new(max_obj_len: usize) -> Self {
        Self {
            max_obj_len,
            buf: Vec::new(),
            in_quotes: false,
        }
    }
}

impl Decoder for CsvRecordCodec {
    type Item = String;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        for (index, byte) in src.iter().enumerate() {
            match byte {
                b'"' => self.in_quotes = !self.in_quotes,
                b'\n' if !self.in_quotes => {
                    let mut record = std::mem::take(&mut self.buf);
                    // Strip a trailing CR from CRLF line endings.
                    if record.last() == Some(&b'\r') {
                        record.pop();
                    }
                    src.advance(index + 1);
                    return Ok(Some(String::from_utf8_lossy(&record).into_owned()));
                }
                _ => {}
            }
            self.buf.push(*byte);

            // Enforce the record limit incrementally so a hostile
            // newline-free stream cannot buffer without bound.
            if self.buf.len() > self.max_obj_len {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("CSV record exceeds maximum length of {}", self.max_obj_len),
                ));
            }
        }

        src.clear();
        Ok(None)
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(src)? {
            Some(item) => Ok(Some(item)),
            None => {
                // A final record without a trailing newline.
                if self.buf.is_empty() {
                    Ok(None)
                } else {
                    let record = std::mem::take(&mut self.buf);
                    self.in_quotes = false;
                    Ok(Some(String::from_utf8_lossy(&record).into_owned()))
                }
            }
        }
    }
}

/// Extension trait for [`hpx::Response`] that provides streaming support for the CSV format.
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
    ///     some_test_field: String,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     const MAX_OBJ_LEN: usize = 64 * 1024;
    ///
    ///     let client = hpx::Client::new()?;
    ///     let _stream = client
    ///         .get("http://localhost:8080/csv")
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

impl CsvStreamResponse for hpx::Response {
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn csv_stream<T>(
        self,
        max_obj_len: usize,
        with_csv_header: bool,
        delimiter: u8,
    ) -> impl futures::Stream<Item = StreamBodyResult<T>> + Send
    where
        T: for<'de> Deserialize<'de>,
    {
        let reader = StreamReader::new(self.bytes_stream().map_err(std::io::Error::other));

        let codec = CsvRecordCodec::new(max_obj_len);
        let frames_reader = tokio_util::codec::FramedRead::new(reader, codec);

        #[expect(clippy::bool_to_int_with_if)]
        let skip_header_if_expected = if with_csv_header { 1 } else { 0 };

        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(delimiter);
        builder.has_headers(false);

        frames_reader
            .into_stream()
            .skip(skip_header_if_expected)
            .map(move |frame_res| match frame_res {
                Ok(frame_str) => {
                    let mut csv_reader = builder.from_reader(frame_str.as_bytes());

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

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use proptest::prelude::*;
    use serde::Deserialize;
    use tokio_util::codec::Decoder;

    use super::CsvRecordCodec;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Record {
        name: String,
        age: u32,
        city: String,
    }

    /// Feed `payload` to the codec in fixed-size chunks and collect records.
    fn decode_chunked(payload: &[u8], chunk_size: usize) -> Vec<String> {
        let mut codec = CsvRecordCodec::new(usize::MAX);
        let mut records = Vec::new();

        for chunk in payload.chunks(chunk_size.max(1)) {
            let mut src = BytesMut::from(chunk);
            while let Some(record) = codec.decode(&mut src).expect("decode") {
                records.push(record);
            }
        }

        let mut src = BytesMut::new();
        if let Some(record) = codec.decode_eof(&mut src).expect("decode_eof") {
            records.push(record);
        }
        records
    }

    #[test]
    fn codec_splits_on_unquoted_newlines() {
        let records = decode_chunked(b"name,age\nBob,25\n", 4);
        assert_eq!(records, vec!["name,age", "Bob,25"]);
    }

    #[test]
    fn codec_keeps_quoted_newlines_intact() {
        let payload = b"name,age\n\"Alice\nSmith\",30\nBob,25";
        let records = decode_chunked(payload, 3);
        assert_eq!(records, vec!["name,age", "\"Alice\nSmith\",30", "Bob,25"]);
    }

    #[test]
    fn codec_handles_escaped_quotes_inside_field() {
        // "" inside a quoted field is an escaped quote; quote state must
        // survive it.
        let payload = b"\"say \"\"hi\"\"\",1\n";
        let records = decode_chunked(payload, 5);
        assert_eq!(records, vec!["\"say \"\"hi\"\"\",1"]);
    }

    #[test]
    fn codec_strips_crlf_outside_quotes() {
        let records = decode_chunked(b"a,b\r\nc,d\r\n", 100);
        assert_eq!(records, vec!["a,b", "c,d"]);
    }

    #[test]
    fn codec_enforces_max_record_length() {
        let mut codec = CsvRecordCodec::new(8);
        let mut src = BytesMut::from(&b"0123456789"[..]);
        let err = codec.decode(&mut src).expect_err("must exceed limit");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    proptest! {
        /// Chunk-boundary invariance: any chunking of a payload must produce
        /// exactly the same records as single-shot decoding.
        #[test]
        fn codec_chunking_is_invariant(
            rows in prop::collection::vec(
                prop::sample::select(vec![
                    "a,1,x",
                    "\"multi\nline\",2,y",
                    "\"quote\"\"d\",3,z",
                    "plain,4,w",
                    "",
                ]),
                1..8,
            ),
            chunk_size in 1usize..16,
        ) {
            let mut payload = rows.join("\n");
            payload.push('\n');

            let expected = decode_chunked(payload.as_bytes(), usize::MAX);
            let actual = decode_chunked(payload.as_bytes(), chunk_size);
            prop_assert_eq!(actual, expected);
        }
    }

    fn deserialize_row(builder: &mut csv::ReaderBuilder, row: &str) -> Option<Record> {
        let mut csv_reader = builder.from_reader(row.as_bytes());
        let mut iter = csv_reader.deserialize::<Record>();
        iter.next()?.ok()
    }

    #[test]
    fn test_csv_builder_reuse_produces_identical_results() {
        let csv_rows = ["Alice,30,NYC", "Bob,25,LA", "Charlie,35,Chicago"];

        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);

        let results: Vec<Record> = csv_rows
            .iter()
            .filter_map(|row| deserialize_row(&mut builder, row))
            .collect();

        assert_eq!(results.len(), 3);
        assert_eq!(
            results[0],
            Record {
                name: "Alice".to_owned(),
                age: 30,
                city: "NYC".to_owned()
            }
        );
        assert_eq!(
            results[1],
            Record {
                name: "Bob".to_owned(),
                age: 25,
                city: "LA".to_owned()
            }
        );
        assert_eq!(
            results[2],
            Record {
                name: "Charlie".to_owned(),
                age: 35,
                city: "Chicago".to_owned()
            }
        );
    }

    #[test]
    fn test_csv_builder_reuse_with_tab_delimiter() {
        let csv_rows = ["Alice\t30\tNYC", "Bob\t25\tLA"];

        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b'\t');
        builder.has_headers(false);

        let results: Vec<Record> = csv_rows
            .iter()
            .filter_map(|row| deserialize_row(&mut builder, row))
            .collect();

        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0],
            Record {
                name: "Alice".to_owned(),
                age: 30,
                city: "NYC".to_owned()
            }
        );
        assert_eq!(
            results[1],
            Record {
                name: "Bob".to_owned(),
                age: 25,
                city: "LA".to_owned()
            }
        );
    }

    #[test]
    fn test_csv_quoted_field_with_commas() {
        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);

        let row = "\"Smith, Jr.\",42,\"New York, NY\"";
        let result = deserialize_row(&mut builder, row);
        assert!(result.is_some());
        let record = result.unwrap();
        assert_eq!(record.name, "Smith, Jr.");
        assert_eq!(record.age, 42);
        assert_eq!(record.city, "New York, NY");
    }

    #[test]
    fn test_csv_quoted_field_with_newlines() {
        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);

        // CSV reader needs the full quoted field including the newline
        let row = "\"Alice\nSmith\",30,NYC";
        let result = deserialize_row(&mut builder, row);
        assert!(result.is_some());
        let record = result.unwrap();
        assert_eq!(record.name, "Alice\nSmith");
    }

    #[test]
    fn test_csv_empty_fields() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct SparseRecord {
            name: String,
            value: String,
        }

        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);

        let row = "hello,";
        let mut csv_reader = builder.from_reader(row.as_bytes());
        let mut iter = csv_reader.deserialize::<SparseRecord>();
        let result = iter.next().unwrap().unwrap();
        assert_eq!(result.name, "hello");
        assert_eq!(result.value, "");
    }

    #[test]
    fn test_csv_invalid_row_returns_none() {
        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);

        // Empty string - no fields to deserialize
        let row = "";
        let result = deserialize_row(&mut builder, row);
        assert!(result.is_none());
    }

    #[test]
    fn test_csv_type_mismatch_fails_gracefully() {
        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);

        // "not_a_number" can't be parsed as u32
        let row = "Alice,not_a_number,NYC";
        let mut csv_reader = builder.from_reader(row.as_bytes());
        let mut iter = csv_reader.deserialize::<Record>();
        let result = iter.next().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_csv_single_row() {
        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);

        let result = deserialize_row(&mut builder, "Solo,42,OnlyTown");
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.name, "Solo");
        assert_eq!(r.age, 42);
        assert_eq!(r.city, "OnlyTown");
    }

    #[test]
    fn test_csv_semicolon_delimiter() {
        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b';');
        builder.has_headers(false);

        let result = deserialize_row(&mut builder, "Alice;30;NYC");
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.name, "Alice");
        assert_eq!(r.age, 30);
        assert_eq!(r.city, "NYC");
    }

    #[test]
    fn test_csv_many_rows() {
        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);

        let mut rows = Vec::new();
        for i in 0..50 {
            rows.push(format!("user{i},{i},city{i}"));
        }

        let results: Vec<Record> = rows
            .iter()
            .filter_map(|row| deserialize_row(&mut builder, row))
            .collect();

        assert_eq!(results.len(), 50);
        assert_eq!(results[0].name, "user0");
        assert_eq!(results[49].city, "city49");
    }

    #[test]
    fn test_csv_field_count_extra_columns_ignored() {
        let mut builder = csv::ReaderBuilder::new();
        builder.delimiter(b',');
        builder.has_headers(false);
        builder.flexible(true);

        let result = deserialize_row(&mut builder, "Alice,30,NYC,extra,data");
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.name, "Alice");
        assert_eq!(r.age, 30);
        assert_eq!(r.city, "NYC");
    }
}
