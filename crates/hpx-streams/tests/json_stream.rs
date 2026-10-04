//! End-to-end tests for the JSON array and JSON Lines stream decoders.
//!
//! Each test serves a body over a real loopback socket so the codec sees the
//! same chunk boundaries a production response would have, then drives
//! `json_array_stream` / `json_nl_stream` on the public `hpx::Response`.
//!
//! Coverage is organised around the ways a JSON stream actually breaks in the
//! wild: values straddling TCP segments, bracket characters inside strings,
//! escape sequences, empty and whitespace-only payloads, primitive vs composite
//! elements, and responses that end mid-value.

#![cfg(feature = "json")]

mod support;

use hpx_streams::{JsonStreamResponse as _, error::StreamBodyKind};
use serde::Deserialize;
use support::{ChunkedServer, collect_ok, partition_stream};

/// Record type used by the composite tests.
#[derive(Debug, Clone, Deserialize, PartialEq)]
struct Item {
    name: String,
    value: i64,
}

/// Record type with a nested object, to reach the bracket-depth tracking.
#[derive(Debug, Clone, Deserialize, PartialEq)]
struct Nested {
    id: u32,
    inner: Inner,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct Inner {
    x: i32,
    y: Option<String>,
}

// ---------------------------------------------------------------------------
// JSON array: happy paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn json_array_single_segment_composite_items() {
    let server = ChunkedServer::serve(
        br#"[{"name":"alice","value":1},{"name":"bob","value":2}]"#,
        "application/json",
    );
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/items"))
            .await
            .json_array_stream(1024),
    )
    .await;

    assert_eq!(
        items,
        vec![
            Item {
                name: "alice".into(),
                value: 1
            },
            Item {
                name: "bob".into(),
                value: 2
            },
        ]
    );
}

#[tokio::test]
async fn json_array_primitive_items() {
    let server = ChunkedServer::serve(b"[10, 20, 30, -4, 0]", "application/json");
    let items: Vec<i64> = collect_ok(
        support::get_ok(&server.url("/n"))
            .await
            .json_array_stream(64),
    )
    .await;
    assert_eq!(items, vec![10, 20, 30, -4, 0]);
}

#[tokio::test]
async fn json_array_string_items() {
    let server = ChunkedServer::serve(br#"["alpha","beta",""]"#, "application/json");
    let items: Vec<String> = collect_ok(
        support::get_ok(&server.url("/s"))
            .await
            .json_array_stream(256),
    )
    .await;
    assert_eq!(items, vec!["alpha", "beta", ""]);
}

#[tokio::test]
async fn json_array_nested_objects() {
    let server = ChunkedServer::serve(
        br#"[{"id":1,"inner":{"x":10,"y":"deep"}},{"id":2,"inner":{"x":-1}}]"#,
        "application/json",
    );
    let items: Vec<Nested> = collect_ok(
        support::get_ok(&server.url("/n"))
            .await
            .json_array_stream(1024),
    )
    .await;

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].inner.x, 10);
    assert_eq!(items[0].inner.y.as_deref(), Some("deep"));
    assert_eq!(items[1].inner.y, None);
}

#[tokio::test]
async fn json_array_of_arrays() {
    let server = ChunkedServer::serve(b"[[1,2],[3,4,5],[]]", "application/json");
    let items: Vec<Vec<i64>> = collect_ok(
        support::get_ok(&server.url("/n"))
            .await
            .json_array_stream(256),
    )
    .await;
    assert_eq!(items, vec![vec![1, 2], vec![3, 4, 5], Vec::<i64>::new()]);
}

#[tokio::test]
async fn json_array_booleans_and_nulls() {
    let server = ChunkedServer::serve(b"[true,false,null,true]", "application/json");
    let items: Vec<Option<bool>> = collect_ok(
        support::get_ok(&server.url("/n"))
            .await
            .json_array_stream(128),
    )
    .await;
    assert_eq!(items, vec![Some(true), Some(false), None, Some(true)]);
}

// ---------------------------------------------------------------------------
// JSON array: hostile strings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn json_array_brackets_inside_strings_do_not_split_items() {
    // `[` and `]` inside a quoted string must be tracked as string content, not
    // as array delimiters, or the item is emitted early and the tail is lost.
    let server = ChunkedServer::serve(br#"["a[b]c","d]e[f","[]"]"#, "application/json");
    let items: Vec<String> = collect_ok(
        support::get_ok(&server.url("/s"))
            .await
            .json_array_stream(256),
    )
    .await;
    assert_eq!(items, vec!["a[b]c", "d]e[f", "[]"]);
}

#[tokio::test]
async fn json_array_escaped_quotes_and_backslashes() {
    let server = ChunkedServer::serve(
        br#"["he said \"hi\"","back\\slash","tab\there","A"]"#,
        "application/json",
    );
    let items: Vec<String> = collect_ok(
        support::get_ok(&server.url("/s"))
            .await
            .json_array_stream(256),
    )
    .await;
    assert_eq!(items[0], r#"he said "hi""#);
    assert_eq!(items[1], r"back\slash");
    assert_eq!(items[2], "tab\there");
    assert_eq!(items[3], "A");
}

#[tokio::test]
async fn json_array_bracket_inside_nested_string() {
    // The `]` here sits inside a value string of a nested object. Treating it
    // as a delimiter would close the object early and desync every later item.
    #[derive(Debug, Deserialize, PartialEq)]
    struct Note {
        note: String,
    }
    let server = ChunkedServer::serve(br#"[{"note":"a]b"},{"note":"c[d"}]"#, "application/json");
    let items: Vec<Note> = collect_ok(
        support::get_ok(&server.url("/n"))
            .await
            .json_array_stream(512),
    )
    .await;
    assert_eq!(
        items,
        vec![Note { note: "a]b".into() }, Note { note: "c[d".into() },]
    );
}

#[tokio::test]
async fn json_array_braces_inside_strings() {
    let server = ChunkedServer::serve(br#"["{not an object}","}{"]"#, "application/json");
    let items: Vec<String> = collect_ok(
        support::get_ok(&server.url("/s"))
            .await
            .json_array_stream(128),
    )
    .await;
    assert_eq!(items, vec!["{not an object}", "}{"]);
}

// ---------------------------------------------------------------------------
// JSON array: chunk-boundary behaviour
// ---------------------------------------------------------------------------

/// The critical property: the decoder's answer must not depend on how the
/// network happened to split the body. Every chunk size from 1 byte upwards is
/// exercised against the same payload.
#[tokio::test]
async fn json_array_is_independent_of_chunk_boundaries() {
    let payload =
        br#"[{"name":"alice","value":1},{"name":"bob","value":2},{"name":"carol","value":3}]"#;
    let expected: Vec<Item> = vec![
        Item {
            name: "alice".into(),
            value: 1,
        },
        Item {
            name: "bob".into(),
            value: 2,
        },
        Item {
            name: "carol".into(),
            value: 3,
        },
    ];

    for chunk in 1..=payload.len() {
        let server = ChunkedServer::serve_chunks(payload, &[chunk], "application/json");
        let items: Vec<Item> = collect_ok(
            support::get_ok(&server.url("/items"))
                .await
                .json_array_stream(1024),
        )
        .await;
        assert_eq!(
            items, expected,
            "decoding differed when the body was split into {chunk}-byte chunks"
        );
    }
}

#[tokio::test]
async fn json_array_split_inside_escape_sequence() {
    // The `\` and the escaped `"` land in different TCP segments, so the escape
    // state has to survive between `decode` calls.
    let server = ChunkedServer::serve_chunks(br#"["ab\"cd","ef"]"#, &[9], "application/json");
    let items: Vec<String> = collect_ok(
        support::get_ok(&server.url("/s"))
            .await
            .json_array_stream(256),
    )
    .await;
    assert_eq!(items, vec![r#"ab"cd"#, "ef"]);
}

#[tokio::test]
async fn json_array_large_payload_split_across_many_segments() {
    let mut body = String::from("[");
    for i in 0..500 {
        if i > 0 {
            body.push(',');
        }
        body.push_str(&format!(r#"{{"name":"item-{i}","value":{i}}}"#));
    }
    body.push(']');

    let server = ChunkedServer::serve_chunks(body.as_bytes(), &[7], "application/json");
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/many"))
            .await
            .json_array_stream(64 * 1024),
    )
    .await;

    assert_eq!(items.len(), 500);
    assert_eq!(items[0].name, "item-0");
    assert_eq!(items[499].value, 499);
}

// ---------------------------------------------------------------------------
// JSON array: boundary and error paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn json_array_empty_body_yields_no_items() {
    let server = ChunkedServer::serve(b"", "application/json");
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/e"))
            .await
            .json_array_stream(1024),
    )
    .await;
    assert!(items.is_empty(), "an empty body must produce no items");
}

#[tokio::test]
async fn json_array_empty_array_yields_no_items() {
    let server = ChunkedServer::serve(b"[]", "application/json");
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/e"))
            .await
            .json_array_stream(1024),
    )
    .await;
    assert!(items.is_empty());
}

#[tokio::test]
async fn json_array_whitespace_only_array_yields_no_items() {
    let server = ChunkedServer::serve(b"[   \n\t ]", "application/json");
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/w"))
            .await
            .json_array_stream(1024),
    )
    .await;
    assert!(items.is_empty());
}

#[tokio::test]
async fn json_array_missing_closing_bracket_drops_unterminated_tail() {
    // The final object never closes, so it must not be emitted. The items
    // before it are still valid and must survive.
    let server = ChunkedServer::serve(
        br#"[{"name":"alice","value":1},{"name":"truncated""#,
        "application/json",
    );
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/t"))
            .await
            .json_array_stream(1024),
    )
    .await;
    assert_eq!(
        items,
        vec![Item {
            name: "alice".into(),
            value: 1
        }]
    );
}

#[tokio::test]
async fn json_array_trailing_primitive_flushed_at_eof() {
    // `42` has no trailing comma or bracket, so it is only emitted when the
    // stream reaches EOF.
    let server = ChunkedServer::serve(b"[1, 2, 42", "application/json");
    let items: Vec<i64> = collect_ok(
        support::get_ok(&server.url("/p"))
            .await
            .json_array_stream(64),
    )
    .await;
    assert_eq!(items, vec![1, 2, 42]);
}

#[tokio::test]
async fn json_array_garbage_payload_reports_codec_error() {
    let server = ChunkedServer::serve(b"[not json at all]", "application/json");
    let (_, errs) = partition_stream::<_, i64>(
        support::get_ok(&server.url("/g"))
            .await
            .json_array_stream(64),
    )
    .await;
    assert!(
        errs.contains(&StreamBodyKind::CodecError),
        "expected a codec error, got {errs:?}"
    );
}

#[tokio::test]
async fn json_array_type_mismatch_reports_codec_error() {
    // Well-formed JSON, wrong shape for `Item`.
    let server = ChunkedServer::serve(br#"[{"name":"a"}]"#, "application/json");
    let (_, errs) = partition_stream::<_, Item>(
        support::get_ok(&server.url("/m"))
            .await
            .json_array_stream(64),
    )
    .await;
    assert_eq!(errs, vec![StreamBodyKind::CodecError]);
}

#[tokio::test]
async fn json_array_object_length_over_limit_reports_max_len() {
    let server = ChunkedServer::serve(br#"[{"name":"alice","value":1}]"#, "application/json");
    let (_, errs) = partition_stream::<_, Item>(
        support::get_ok(&server.url("/l"))
            .await
            .json_array_stream(4),
    )
    .await;
    assert_eq!(errs, vec![StreamBodyKind::MaxLenReachedError]);
}

#[tokio::test]
async fn json_array_leading_comma_reports_codec_error() {
    // A delimiter where a value is expected is a framing violation.
    let server = ChunkedServer::serve(b"[,1,2]", "application/json");
    let (_, errs) = partition_stream::<_, i64>(
        support::get_ok(&server.url("/c"))
            .await
            .json_array_stream(64),
    )
    .await;
    assert_eq!(errs, vec![StreamBodyKind::CodecError]);
}

#[tokio::test]
async fn json_array_custom_initial_capacity_does_not_change_result() {
    let payload = br#"[{"name":"a","value":1},{"name":"b","value":2}]"#;
    for capacity in [1usize, 8, 4096] {
        let server = ChunkedServer::serve(payload, "application/json");
        let items: Vec<Item> = collect_ok(
            support::get_ok(&server.url("/items"))
                .await
                .json_array_stream_with_capacity(1024, capacity),
        )
        .await;
        assert_eq!(items.len(), 2, "capacity {capacity} changed the result");
    }
}

// ---------------------------------------------------------------------------
// JSON Lines (NL) stream
// ---------------------------------------------------------------------------

#[tokio::test]
async fn json_nl_basic_lines() {
    let server = ChunkedServer::serve(
        b"{\"name\":\"a\",\"value\":1}\n{\"name\":\"b\",\"value\":2}\n",
        "application/x-ndjson",
    );
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/nd"))
            .await
            .json_nl_stream(1024),
    )
    .await;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].name, "a");
    assert_eq!(items[1].value, 2);
}

#[tokio::test]
async fn json_nl_crlf_line_endings() {
    let server = ChunkedServer::serve(
        b"{\"name\":\"a\",\"value\":1}\r\n{\"name\":\"b\",\"value\":2}\r\n",
        "application/x-ndjson",
    );
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/nd"))
            .await
            .json_nl_stream(1024),
    )
    .await;
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].name, "b");
}

#[tokio::test]
async fn json_nl_final_line_without_newline() {
    let server = ChunkedServer::serve(
        b"{\"name\":\"a\",\"value\":1}\n{\"name\":\"b\",\"value\":2}",
        "application/x-ndjson",
    );
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/nd"))
            .await
            .json_nl_stream(1024),
    )
    .await;
    assert_eq!(items.len(), 2, "the last line must be flushed at EOF");
}

#[tokio::test]
async fn json_nl_split_across_segments() {
    let payload = b"{\"name\":\"a\",\"value\":1}\n{\"name\":\"b\",\"value\":2}\n{\"name\":\"c\",\"value\":3}\n";
    for chunk in [1usize, 5, 17] {
        let server = ChunkedServer::serve_chunks(payload, &[chunk], "application/x-ndjson");
        let items: Vec<Item> = collect_ok(
            support::get_ok(&server.url("/nd"))
                .await
                .json_nl_stream(1024),
        )
        .await;
        assert_eq!(
            items.len(),
            3,
            "chunk size {chunk} lost or duplicated a line"
        );
        assert_eq!(items[2].name, "c");
    }
}

#[tokio::test]
async fn json_nl_blank_lines_are_reported_as_codec_errors() {
    // Each physical line is framed independently, so a blank line reaches the
    // JSON parser as an empty document and is reported as a per-item error
    // rather than being silently skipped. The good lines around it still
    // decode, which is what matters for resumable NDJSON feeds.
    let server = ChunkedServer::serve(
        b"\n{\"name\":\"a\",\"value\":1}\n\n\n",
        "application/x-ndjson",
    );
    let (items, errs) = partition_stream::<_, Item>(
        support::get_ok(&server.url("/nd"))
            .await
            .json_nl_stream(1024),
    )
    .await;

    assert_eq!(
        items,
        vec![Item {
            name: "a".into(),
            value: 1
        }],
        "the non-blank line must still decode"
    );
    assert_eq!(
        errs,
        vec![
            StreamBodyKind::CodecError,
            StreamBodyKind::CodecError,
            StreamBodyKind::CodecError
        ],
        "each of the three blank lines is reported once"
    );
}

#[tokio::test]
async fn json_nl_invalid_line_does_not_stop_the_stream() {
    // A malformed line is surfaced as an error *item*, not a stream-ending
    // codec failure, so a single bad record in a long feed does not force the
    // consumer to reconnect and re-read everything.
    let server = ChunkedServer::serve(
        b"{\"name\":\"a\",\"value\":1}\n{oops}\n{\"name\":\"b\",\"value\":2}\n",
        "application/x-ndjson",
    );
    let (items, errs) = partition_stream::<_, Item>(
        support::get_ok(&server.url("/nd"))
            .await
            .json_nl_stream(1024),
    )
    .await;

    assert_eq!(items.len(), 2, "both well-formed lines must decode");
    assert_eq!(items[0].name, "a");
    assert_eq!(
        items[1].name, "b",
        "the line after the bad one must survive"
    );
    assert_eq!(errs, vec![StreamBodyKind::CodecError]);
}

#[tokio::test]
async fn json_nl_line_over_limit_reports_error() {
    let server = ChunkedServer::serve(
        b"{\"name\":\"aaaaaaaaaaaaaaaaaaaa\",\"value\":1}\n",
        "application/x-ndjson",
    );
    let (_, errs) =
        partition_stream::<_, Item>(support::get_ok(&server.url("/nd")).await.json_nl_stream(8))
            .await;
    assert!(!errs.is_empty(), "an over-long line must be rejected");
}

#[tokio::test]
async fn json_nl_empty_body_yields_no_items() {
    let server = ChunkedServer::serve(b"", "application/x-ndjson");
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/nd"))
            .await
            .json_nl_stream(1024),
    )
    .await;
    assert!(items.is_empty());
}

#[tokio::test]
async fn json_nl_custom_capacity_matches_default() {
    let payload = b"{\"name\":\"a\",\"value\":1}\n{\"name\":\"b\",\"value\":2}\n";
    let server = ChunkedServer::serve(payload, "application/x-ndjson");
    let items: Vec<Item> = collect_ok(
        support::get_ok(&server.url("/nd"))
            .await
            .json_nl_stream_with_capacity(1024, 1),
    )
    .await;
    assert_eq!(items.len(), 2);
}
