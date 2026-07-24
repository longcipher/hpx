use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use tokio_util::io::StreamReader;

use crate::{StreamBodyError, StreamBodyResult, error::StreamBodyKind};

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

        let codec = tokio_util::codec::LinesCodec::new_with_max_length(max_obj_len);
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
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Record {
        name: String,
        age: u32,
        city: String,
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
