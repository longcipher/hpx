//! Contract tests for the zero-copy, on-demand JSON frame view.
//!
//! These tests define the observable behaviour first: a market-data frame is
//! parsed in place, path lookups return borrowed scalars, and no owned copy of
//! any field is ever made.

#![cfg(feature = "borrowed-json")]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    missing_docs
)]

use hpx_streams::{BorrowedJsonFrame, PathSegment as P};

/// A realistic depth-update frame: a handful of hot fields plus a large
/// redundant payload, which is exactly the case where full DOM/struct
/// deserialization wastes work.
fn depth_frame() -> Vec<u8> {
    let mut frame = String::from(
        r#"{"e":"depthUpdate","E":1764321000123,"s":"BTCUSDT","U":100,"u":101,"bids":[["67213.50","1.25000000"],["67213.40","0.50000000"]],"asks":[["67213.60","2.00000000"]],"meta":{"source":"matching-engine","flags":[true,false,null]},"unused":""#,
    );
    frame.extend(std::iter::repeat_n('x', 3500));
    frame.push_str(r#""}"#);
    frame.into_bytes()
}

#[test]
fn extracts_top_of_book_from_a_large_frame() {
    let mut buf = depth_frame();
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");

    assert_eq!(frame.get_str(&[P::Key("e")]), Some("depthUpdate"));
    assert_eq!(frame.get_str(&[P::Key("s")]), Some("BTCUSDT"));
    assert_eq!(frame.get_u64(&[P::Key("E")]), Some(1_764_321_000_123));
    assert_eq!(frame.get_u64(&[P::Key("U")]), Some(100));
    assert_eq!(
        frame.get_str(&[P::Key("bids"), P::Index(0), P::Index(0)]),
        Some("67213.50")
    );
    assert_eq!(
        frame.get_str(&[P::Key("bids"), P::Index(1), P::Index(1)]),
        Some("0.50000000")
    );
    assert_eq!(
        frame.get_str(&[P::Key("asks"), P::Index(0), P::Index(1)]),
        Some("2.00000000")
    );
}

#[test]
fn extracts_nested_scalars_and_flags() {
    let mut buf = depth_frame();
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");

    assert_eq!(
        frame.get_str(&[P::Key("meta"), P::Key("source")]),
        Some("matching-engine")
    );
    assert_eq!(
        frame.get_bool(&[P::Key("meta"), P::Key("flags"), P::Index(0)]),
        Some(true)
    );
    assert_eq!(
        frame.get_bool(&[P::Key("meta"), P::Key("flags"), P::Index(1)]),
        Some(false)
    );
    assert!(frame.is_null(&[P::Key("meta"), P::Key("flags"), P::Index(2)]));
}

#[test]
fn borrowed_strings_point_into_the_original_buffer() {
    let mut buf = depth_frame();
    let base = buf.as_ptr() as usize;
    let end = base + buf.len();

    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");
    let symbol = frame.get_str(&[P::Key("s")]).expect("symbol");
    let price = frame
        .get_str(&[P::Key("bids"), P::Index(0), P::Index(0)])
        .expect("best bid");

    for value in [symbol, price] {
        let ptr = value.as_ptr() as usize;
        assert!(
            ptr >= base && ptr + value.len() <= end,
            "returned string must borrow from the input buffer"
        );
    }
}

#[test]
fn invalid_json_is_rejected() {
    for broken in [
        b"{not json".as_slice(),
        b"{\"a\":1".as_slice(),
        b"".as_slice(),
    ] {
        let mut buf = broken.to_vec();
        assert!(
            BorrowedJsonFrame::parse(&mut buf).is_err(),
            "malformed input must not produce a frame"
        );
    }
}

#[test]
fn trailing_content_is_rejected() {
    let mut buf = b"{\"a\":1} garbage".to_vec();
    assert!(BorrowedJsonFrame::parse(&mut buf).is_err());
}

#[test]
fn missing_paths_return_none() {
    let mut buf = depth_frame();
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");

    assert_eq!(frame.get_str(&[P::Key("missing")]), None);
    assert_eq!(frame.get_str(&[P::Key("meta"), P::Key("missing")]), None);
    assert_eq!(frame.get_str(&[P::Key("bids"), P::Index(99)]), None);
    assert_eq!(frame.get_str(&[P::Key("bids"), P::Key("0")]), None);
    assert_eq!(frame.get_u64(&[P::Index(0)]), None);
}

#[test]
fn type_mismatches_return_none() {
    let mut buf = depth_frame();
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");

    assert_eq!(frame.get_u64(&[P::Key("s")]), None);
    assert_eq!(frame.get_str(&[P::Key("E")]), None);
    assert_eq!(frame.get_bool(&[P::Key("U")]), None);
    assert_eq!(frame.get_f64(&[P::Key("bids")]), None);
    assert_eq!(frame.get_i64(&[P::Key("meta")]), None);
    assert!(!frame.is_null(&[P::Key("s")]));
}

#[test]
fn all_scalar_kinds_are_readable() {
    let mut buf =
        br#"{"t":"abc","pos":42,"neg":-7,"frac":1.5,"yes":true,"no":false,"nil":null}"#.to_vec();
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");

    assert_eq!(frame.get_str(&[P::Key("t")]), Some("abc"));
    assert_eq!(frame.get_u64(&[P::Key("pos")]), Some(42));
    assert_eq!(frame.get_i64(&[P::Key("neg")]), Some(-7));
    assert!((frame.get_f64(&[P::Key("frac")]).expect("frac") - 1.5).abs() < f64::EPSILON);
    assert_eq!(frame.get_bool(&[P::Key("yes")]), Some(true));
    assert_eq!(frame.get_bool(&[P::Key("no")]), Some(false));
    assert!(frame.is_null(&[P::Key("nil")]));
}

#[test]
fn escaped_strings_are_unescaped_in_place() {
    let mut buf = br#"{"s":"BTC\u002fUSDT","q":"a\"b\\c"}"#.to_vec();
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");

    assert_eq!(frame.get_str(&[P::Key("s")]), Some("BTC/USDT"));
    assert_eq!(frame.get_str(&[P::Key("q")]), Some("a\"b\\c"));
}

#[test]
fn contains_key_is_scoped_to_the_selected_value() {
    let mut buf = depth_frame();
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");

    assert!(frame.contains_key(&[], "s"));
    assert!(frame.contains_key(&[P::Key("meta")], "flags"));
    assert!(!frame.contains_key(&[P::Key("meta")], "s"));
    assert!(!frame.contains_key(&[P::Key("bids")], "flags"));
    assert!(!frame.contains_key(&[P::Key("missing")], "flags"));
}

#[test]
fn empty_path_addresses_the_root() {
    let mut buf = br#""root-string""#.to_vec();
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("valid JSON frame");
    assert_eq!(frame.get_str(&[]), Some("root-string"));
}

#[test]
fn path_segment_conversions() {
    assert_eq!(P::from("bids"), P::Key("bids"));
    assert_eq!(P::from(3usize), P::Index(3));
    assert_ne!(P::from("bids"), P::from("asks"));
}

#[test]
fn reused_buffer_can_parse_successive_frames() {
    let mut buf = br#"{"s":"BTCUSDT","n":1}"#.to_vec();
    {
        let frame = BorrowedJsonFrame::parse(&mut buf).expect("first frame");
        assert_eq!(frame.get_str(&[P::Key("s")]), Some("BTCUSDT"));
        assert_eq!(frame.get_u64(&[P::Key("n")]), Some(1));
    }

    buf.clear();
    buf.extend_from_slice(br#"{"s":"ETHUSDT","n":2}"#);
    let frame = BorrowedJsonFrame::parse(&mut buf).expect("second frame");
    assert_eq!(frame.get_str(&[P::Key("s")]), Some("ETHUSDT"));
    assert_eq!(frame.get_u64(&[P::Key("n")]), Some(2));
}

mod properties {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Every key present in the source map must be readable through the
        /// zero-copy view, regardless of key order or value length.
        #[test]
        fn key_lookup_matches_source_map(
            entries in proptest::collection::btree_map("[a-z]{1,8}", "[A-Za-z0-9]{0,16}", 1..8),
        ) {
            let mut buf = serde_json::to_vec(&entries).expect("serialize map");
            let frame = BorrowedJsonFrame::parse(&mut buf).expect("parse frame");
            for (key, value) in &entries {
                prop_assert_eq!(frame.get_str(&[P::Key(key.as_str())]), Some(value.as_str()));
            }
        }
    }
}
