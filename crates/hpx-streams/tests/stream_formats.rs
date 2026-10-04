//! End-to-end tests for the CSV and length-prefixed Protobuf stream decoders.
//!
//! Both formats are framing-heavy: CSV has to track RFC 4180 quote state so a
//! newline inside a quoted field does not split a record, and Protobuf has to
//! reassemble a LEB128 length prefix that may straddle several TCP segments.
//! The tests below drive each decoder over a real socket with bodies split at
//! the boundaries that break them.

#![cfg(any(feature = "csv", feature = "protobuf"))]

mod support;

#[cfg(feature = "csv")]
mod csv {
    use hpx_streams::CsvStreamResponse as _;
    use serde::Deserialize;

    use crate::support::{ChunkedServer, collect_ok, partition_stream};

    /// Flat CSV row.
    #[derive(Debug, Deserialize, PartialEq, Clone)]
    struct Row {
        id: u32,
        name: String,
    }

    fn csv_body(rows: &str) -> String {
        rows.to_owned()
    }

    #[tokio::test]
    async fn simple_rows() {
        let body = csv_body("1,alice\n2,bob\n");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let rows: Vec<Row> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b','),
        )
        .await;
        assert_eq!(
            rows,
            vec![
                Row {
                    id: 1,
                    name: "alice".into()
                },
                Row {
                    id: 2,
                    name: "bob".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn headers_are_skipped_when_requested() {
        let body = csv_body("id,name\n1,alice\n");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let rows: Vec<Row> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, true, b','),
        )
        .await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "alice");
    }

    #[tokio::test]
    async fn headers_are_parsed_as_data_when_not_skipped() {
        // With `with_csv_header = false` nothing is dropped, so the header line
        // is handed to the deserializer like any other record. Fields are typed
        // as strings here so the header text itself is a valid record.
        #[derive(Debug, Deserialize, PartialEq)]
        struct StrRow {
            first: String,
            second: String,
        }
        let body = csv_body("id,name\n1,alice\n");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let rows: Vec<StrRow> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b','),
        )
        .await;
        assert_eq!(rows.len(), 2, "the header line becomes a data row");
        assert_eq!(
            rows[0],
            StrRow {
                first: "id".into(),
                second: "name".into()
            }
        );
        assert_eq!(rows[1].second, "alice");
    }

    #[tokio::test]
    async fn crlf_line_endings_are_normalized() {
        let body = csv_body("1,alice\r\n2,bob\r\n");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let rows: Vec<Row> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b','),
        )
        .await;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1],
            Row {
                id: 2,
                name: "bob".into()
            }
        );
    }

    #[tokio::test]
    async fn final_record_without_newline_is_flushed() {
        let body = csv_body("1,alice\n2,bob");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let rows: Vec<Row> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b','),
        )
        .await;
        assert_eq!(rows.len(), 2, "the trailing record must reach the consumer");
    }

    #[tokio::test]
    async fn newline_inside_quoted_field_does_not_split_record() {
        // The decisive RFC 4180 case: a literal newline inside a quoted field.
        // A decoder that splits on `\n` emits two fragments and loses the row.
        let body = csv_body("1,\"line one\nline two\"\n2,bob\n");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let rows: Vec<Row> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b','),
        )
        .await;

        assert_eq!(rows.len(), 2, "the quoted newline must not create a row");
        assert_eq!(rows[0].name, "line one\nline two");
        assert_eq!(rows[1].name, "bob");
    }

    #[tokio::test]
    async fn quoted_field_with_commas_and_escaped_quotes() {
        // A field containing the delimiter, and a field containing doubled
        // quotes, both round-trip through the record splitter intact.
        #[derive(Debug, Deserialize, PartialEq)]
        struct Quoted {
            id: u32,
            embedded_comma: String,
            embedded_quotes: String,
        }
        let body = csv_body("1,\"a,b\",\"say \"\"hi\"\"\"\n");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let rows: Vec<Quoted> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b','),
        )
        .await;
        assert_eq!(
            rows,
            vec![Quoted {
                id: 1,
                embedded_comma: "a,b".into(),
                embedded_quotes: r#"say "hi""#.into(),
            }]
        );
    }

    #[tokio::test]
    async fn quoted_field_split_across_segments() {
        // The opening quote and the newline land in different TCP segments, so
        // the quote state has to survive between `decode` calls.
        let body = csv_body("1,\"multi\nline\"\n2,bob\n");
        for chunk in [1usize, 3, 11] {
            let server = ChunkedServer::serve_chunks(body.as_bytes(), &[chunk], "text/csv");
            let rows: Vec<Row> = collect_ok(
                crate::support::get_ok(&server.url("/csv"))
                    .await
                    .csv_stream(1024, false, b','),
            )
            .await;
            assert_eq!(rows.len(), 2, "chunk size {chunk} split a quoted record");
            assert_eq!(rows[0].name, "multi\nline");
        }
    }

    #[tokio::test]
    async fn record_over_max_length_is_rejected() {
        let body = csv_body("1,aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n2,bob\n");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let (rows, _) = partition_stream::<_, Row>(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(16, false, b','),
        )
        .await;
        assert!(
            rows.is_empty(),
            "an over-long record must not be surfaced, got {rows:?}"
        );
    }

    #[tokio::test]
    async fn malformed_row_reports_error_and_stream_continues() {
        // `not-a-number` cannot deserialize into `u32`, so the row is an error
        // item; the following well-formed rows must still arrive.
        let body = csv_body("1,alice\nnot-a-number,bad\n2,bob\n");
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let (rows, errs) = partition_stream::<_, Row>(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b','),
        )
        .await;

        assert_eq!(errs.len(), 1, "exactly the malformed row is an error");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "alice");
        assert_eq!(rows[1].name, "bob");
    }

    #[tokio::test]
    async fn empty_body_yields_no_rows() {
        let server = ChunkedServer::serve(b"", "text/csv");
        let rows: Vec<Row> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b','),
        )
        .await;
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn semicolon_delimiter_is_honoured() {
        let body = "1;alice\n2;bob\n";
        let server = ChunkedServer::serve(body.as_bytes(), "text/csv");
        let rows: Vec<Row> = collect_ok(
            crate::support::get_ok(&server.url("/csv"))
                .await
                .csv_stream(1024, false, b';'),
        )
        .await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "alice");
    }

    #[tokio::test]
    async fn results_are_stable_across_chunk_boundaries() {
        let body = csv_body("1,alice\n2,\"b,ob\"\n3,carol\n");
        for chunk in [1usize, 2, 5, 13] {
            let server = ChunkedServer::serve_chunks(body.as_bytes(), &[chunk], "text/csv");
            let rows: Vec<Row> = collect_ok(
                crate::support::get_ok(&server.url("/csv"))
                    .await
                    .csv_stream(1024, false, b','),
            )
            .await;
            assert_eq!(rows.len(), 3, "chunk size {chunk} lost a row");
            assert_eq!(rows[1].name, "b,ob");
        }
    }
}

#[cfg(feature = "protobuf")]
mod protobuf {
    use hpx_streams::{ProtobufStreamResponse as _, error::StreamBodyKind};
    use prost::Message as _;

    use crate::support::{ChunkedServer, collect_ok, encode_len_prefixed, partition_stream};

    #[derive(Clone, PartialEq, prost::Message)]
    struct Msg {
        #[prost(string, tag = "1")]
        name: String,
        #[prost(uint32, tag = "2")]
        value: u32,
    }

    fn msg(name: &str, value: u32) -> Msg {
        Msg {
            name: name.to_owned(),
            value,
        }
    }

    #[tokio::test]
    async fn single_message() {
        let body = encode_len_prefixed(&[msg("alice", 1)]);
        let server = ChunkedServer::serve(&body, "application/protobuf");
        let items: Vec<Msg> = collect_ok(
            crate::support::get_ok(&server.url("/pb"))
                .await
                .protobuf_stream(4096),
        )
        .await;
        assert_eq!(items, vec![msg("alice", 1)]);
    }

    #[tokio::test]
    async fn many_messages_in_order() {
        let expected: Vec<Msg> = (0..25).map(|i| msg(&format!("m{i}"), i)).collect();
        let body = encode_len_prefixed(&expected);
        let server = ChunkedServer::serve(&body, "application/protobuf");
        let items: Vec<Msg> = collect_ok(
            crate::support::get_ok(&server.url("/pb"))
                .await
                .protobuf_stream(65536),
        )
        .await;
        assert_eq!(items, expected);
    }

    #[tokio::test]
    async fn zero_length_message_is_delivered() {
        // An empty message is still a message: the length prefix is 0 and there
        // are no body bytes. Treating `0` as "not yet read" would silently drop
        // it and desynchronize everything after it.
        let expected = vec![Msg::default(), msg("after", 2), Msg::default()];
        let body = encode_len_prefixed(&expected);
        let server = ChunkedServer::serve(&body, "application/protobuf");
        let items: Vec<Msg> = collect_ok(
            crate::support::get_ok(&server.url("/pb"))
                .await
                .protobuf_stream(4096),
        )
        .await;
        assert_eq!(items, expected, "empty messages must not desync the stream");
    }

    #[tokio::test]
    async fn multi_byte_varint_length_prefix() {
        // A 300-byte body forces a two-byte LEB128 prefix, so the length
        // decoding branch for `byte >= 0x80` is exercised end to end.
        let expected = vec![msg(&"x".repeat(290), 1), msg("small", 2)];
        let body = encode_len_prefixed(&expected);
        assert!(body[0] >= 0x80, "expected a multi-byte varint prefix");
        let server = ChunkedServer::serve(&body, "application/protobuf");
        let items: Vec<Msg> = collect_ok(
            crate::support::get_ok(&server.url("/pb"))
                .await
                .protobuf_stream(65536),
        )
        .await;
        assert_eq!(items, expected);
    }

    #[tokio::test]
    async fn varint_split_across_segments() {
        let expected = vec![msg(&"y".repeat(300), 7), msg("next", 8)];
        let body = encode_len_prefixed(&expected);
        for chunk in [1usize, 2, 17, 64] {
            let server = ChunkedServer::serve_chunks(&body, &[chunk], "application/protobuf");
            let items: Vec<Msg> = collect_ok(
                crate::support::get_ok(&server.url("/pb"))
                    .await
                    .protobuf_stream(65536),
            )
            .await;
            assert_eq!(items, expected, "chunk size {chunk} corrupted the stream");
        }
    }

    #[tokio::test]
    async fn message_body_split_across_segments() {
        let expected = vec![msg("split-me", 9)];
        let body = encode_len_prefixed(&expected);
        for chunk in [1usize, 3, 5] {
            let server = ChunkedServer::serve_chunks(&body, &[chunk], "application/protobuf");
            let items: Vec<Msg> = collect_ok(
                crate::support::get_ok(&server.url("/pb"))
                    .await
                    .protobuf_stream(4096),
            )
            .await;
            assert_eq!(items, expected, "chunk size {chunk} corrupted the message");
        }
    }

    #[tokio::test]
    async fn object_over_max_length_is_rejected() {
        let body = encode_len_prefixed(&[msg("a-fairly-long-name", 1)]);
        let server = ChunkedServer::serve(&body, "application/protobuf");
        let (items, errs) = partition_stream::<_, Msg>(
            crate::support::get_ok(&server.url("/pb"))
                .await
                .protobuf_stream(4),
        )
        .await;
        assert!(items.is_empty());
        assert_eq!(errs, vec![StreamBodyKind::MaxLenReachedError]);
    }

    #[tokio::test]
    async fn malformed_message_body_reports_codec_error() {
        // Length prefix says 8 bytes, but the bytes are not a valid message
        // (field number 0 is reserved).
        let mut body = Vec::new();
        body.extend_from_slice(&[8u8]);
        body.extend_from_slice(&[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
        let server = ChunkedServer::serve(&body, "application/protobuf");
        let (items, errs) = partition_stream::<_, Msg>(
            crate::support::get_ok(&server.url("/pb"))
                .await
                .protobuf_stream(4096),
        )
        .await;
        assert!(items.is_empty());
        assert_eq!(errs, vec![StreamBodyKind::CodecError]);
    }

    #[tokio::test]
    async fn empty_body_yields_no_messages() {
        let server = ChunkedServer::serve(b"", "application/protobuf");
        let items: Vec<Msg> = collect_ok(
            crate::support::get_ok(&server.url("/pb"))
                .await
                .protobuf_stream(4096),
        )
        .await;
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn stream_is_independent_of_chunk_boundaries() {
        let expected: Vec<Msg> = (0..12).map(|i| msg(&format!("row-{i}"), i)).collect();
        let body = encode_len_prefixed(&expected);
        for chunk in 1..=body.len().min(64) {
            let server = ChunkedServer::serve_chunks(&body, &[chunk], "application/protobuf");
            let items: Vec<Msg> = collect_ok(
                crate::support::get_ok(&server.url("/pb"))
                    .await
                    .protobuf_stream(65536),
            )
            .await;
            assert_eq!(items, expected, "chunk size {chunk} changed the result");
        }
    }

    #[tokio::test]
    async fn encoding_round_trips_through_the_decoder() {
        // The fixture itself is checked so a broken encoder cannot make the
        // decoder assertions vacuous.
        let expected = vec![msg("round", 1)];
        let body = encode_len_prefixed(&expected);
        let decoded = Msg::decode(&body[1..]).expect("fixture must decode");
        assert_eq!(decoded, expected[0]);
    }
}
