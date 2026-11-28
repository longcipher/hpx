//! Wire-level self-check for the browser emulation profiles.
//!
//! `hpx_emulation::verify` turns the observable parts of a request into
//! deterministic fingerprints. This test closes the loop between the two
//! sides of the wire:
//!
//! - the *client side* fingerprint is computed from the profile, i.e. from
//!   the headers the emulation claims to send;
//! - the *server side* fingerprint is computed from the bytes a server
//!   actually received.
//!
//! Both must agree, so a profile that silently drops or renames a header is
//! caught before it reaches production. The HTTP/2 fingerprint is pinned to a
//! recorded value as well, so a change to the SETTINGS order, the pseudo
//! header order or the window sizes shows up as an explicit diff instead of a
//! silent anti-bot regression.
//!
//! Header *order* is deliberately not part of the HTTP/1.1 fingerprint:
//! `HeaderMap` does not preserve insertion order, so the header sequence a
//! client emits is not defined by the profile. The fingerprint therefore
//! covers the header *set*; the HTTP/2 SETTINGS and pseudo header order —
//! which the profile does control — is covered by [`Http2Signature`].

#![cfg(all(feature = "emulation", feature = "http1"))]
#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used
)]

use std::{collections::BTreeSet, net::SocketAddr};

use hpx::{Client, EmulationFactory as _, http2::PseudoId};
use hpx_emulation::{
    Emulation,
    verify::{HeaderSignature, Http2Signature, Ja4h, diff},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    sync::oneshot,
};

/// The header the compression feature decides on, so it is not part of the
/// fingerprint: `hpx` manages `Accept-Encoding` itself.
const FEATURE_DEPENDENT_HEADER: &str = "accept-encoding";

/// The profile the wire self-check runs against.
const PROFILE: Emulation = Emulation::Chrome152;

/// The HTTP/2 fingerprint of [`PROFILE`], recorded from the profile's
/// `Http2Options`. A change to the SETTINGS order, the pseudo header order or
/// the window sizes must be reflected here.
const PROFILE_HTTP2_FINGERPRINT: &str = "1:65536;2:0;3:0;4:6291456;5:0;6:262144;8:0;9:0|15728640|0|:method,:authority,:scheme,:path,:protocol,:status";

/// The header names the profile declares, in canonical (sorted) order.
fn profile_header_names(kind: Emulation) -> Vec<String> {
    let mut emulation = kind.emulation();
    let mut names: Vec<String> = emulation
        .headers_mut()
        .keys()
        .map(|name| name.as_str().to_owned())
        .filter(|name| name != FEATURE_DEPENDENT_HEADER)
        .collect();
    names.sort();
    names
}

/// The fingerprint of a set of header names.
fn header_fingerprint(names: &[String]) -> HeaderSignature {
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    HeaderSignature::new("GET", "11", &refs)
}

/// The JA4H fingerprint of a set of header names.
///
/// The profile does not send cookies and does not send a `Referer`, so both
/// are reported as absent.
fn ja4h_fingerprint(names: &[String]) -> Ja4h {
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    Ja4h::new(
        "GET",
        hpx_emulation::verify::HttpVersion::Http11,
        &refs,
        &[],
        Some("en-US,en;q=0.9"),
    )
}

/// The HTTP/2 fingerprint of a profile.
fn http2_fingerprint(kind: Emulation) -> Http2Signature {
    let mut emulation = kind.emulation();
    let options = emulation
        .http2_options_mut()
        .clone()
        .expect("profile carries HTTP/2 options");

    let settings_order = options
        .settings_order
        .clone()
        .expect("profile pins the SETTINGS order");
    let settings: Vec<(u16, u32)> = (&settings_order)
        .into_iter()
        .map(|id| {
            let id = u16::from(*id);
            let value = match id {
                // SETTINGS_HEADER_TABLE_SIZE
                0x1 => options.header_table_size.unwrap_or_default(),
                // SETTINGS_ENABLE_PUSH
                0x2 => u32::from(options.enable_push.unwrap_or(false)),
                // SETTINGS_MAX_CONCURRENT_STREAMS
                0x3 => options.max_concurrent_streams.unwrap_or_default(),
                // SETTINGS_INITIAL_WINDOW_SIZE
                0x4 => options.initial_window_size,
                // SETTINGS_MAX_FRAME_SIZE
                0x5 => options.max_frame_size.unwrap_or_default(),
                // SETTINGS_MAX_HEADER_LIST_SIZE
                0x6 => options.max_header_list_size.unwrap_or_default(),
                // SETTINGS_ENABLE_CONNECT_PROTOCOL
                0x8 => u32::from(options.enable_connect_protocol.unwrap_or(false)),
                // SETTINGS_NO_RFC7540_PRIORITIES
                0x9 => u32::from(options.no_rfc7540_priorities.unwrap_or(false)),
                _ => 0,
            };
            (id, value)
        })
        .collect();

    let pseudo_order = options
        .headers_pseudo_order
        .clone()
        .expect("profile pins the pseudo header order");
    let pseudo_order: Vec<&str> = (&pseudo_order)
        .into_iter()
        .map(|id| match id {
            PseudoId::Method => ":method",
            PseudoId::Scheme => ":scheme",
            PseudoId::Authority => ":authority",
            PseudoId::Path => ":path",
            PseudoId::Protocol => ":protocol",
            PseudoId::Status => ":status",
        })
        .collect();

    Http2Signature::new(
        &settings,
        Some(options.initial_conn_window_size),
        &[],
        &pseudo_order,
    )
}

/// Start a minimal HTTP/1.1 server that records the request head it receives
/// and answers with a fixed body.
async fn capture_request() -> (SocketAddr, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            let read = socket.read(&mut byte).await.expect("read");
            if read == 0 {
                break;
            }
            head.push(byte[0]);
        }
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .await;
        let _ = tx.send(String::from_utf8_lossy(&head).into_owned());
    });
    (addr, rx)
}

/// The header names of a raw request head, in lower case.
fn observed_header_names(head: &str) -> Vec<String> {
    head.split("\r\n")
        .skip(1)
        .filter_map(|line| line.split(':').next())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
        .collect()
}

/// The profile's headers must reach the server unchanged.
///
/// This is the bidirectional half of the self-check: the fingerprint the
/// client computes from the profile and the fingerprint the server computes
/// from the received bytes must be identical.
#[tokio::test]
async fn profile_headers_reach_the_server_intact() {
    let (addr, captured) = capture_request().await;

    let client = Client::builder().build().expect("client");
    let response = client
        .get(format!("http://{addr}/"))
        .emulation(PROFILE)
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), hpx::StatusCode::OK);

    let head = captured.await.expect("captured request");
    let observed = observed_header_names(&head);
    let observed: BTreeSet<String> = observed.into_iter().collect();

    let profile = profile_header_names(PROFILE);
    let profile_set: BTreeSet<String> = profile.iter().cloned().collect();

    for name in &profile {
        assert!(
            observed.contains(name),
            "profile header `{name}` never reached the server; observed: {observed:?}"
        );
    }

    // The server side fingerprint is computed over the profile headers only,
    // so it is comparable with the client side fingerprint.
    let server_side: Vec<String> = profile_set
        .iter()
        .filter(|name| observed.contains(*name))
        .cloned()
        .collect();
    let mut server_side = server_side;
    server_side.sort();

    let client_fp = header_fingerprint(&profile);
    let server_fp = header_fingerprint(&server_side);
    assert!(
        diff(&client_fp.render(), &server_fp.render()).is_empty(),
        "client and server fingerprints disagree: client={} server={}",
        client_fp.render(),
        server_fp.render()
    );

    // The same must hold for the JA4H fingerprint.
    let client_ja4h = ja4h_fingerprint(&profile);
    let server_ja4h = ja4h_fingerprint(&server_side);
    assert!(
        diff(&client_ja4h.render(), &server_ja4h.render()).is_empty(),
        "client and server JA4H fingerprints disagree: client={} server={}",
        client_ja4h.render(),
        server_ja4h.render()
    );
}

/// The profile's HTTP/2 fingerprint must not drift.
#[cfg(feature = "http2")]
#[test]
fn profile_http2_fingerprint_matches_the_golden_value() {
    let signature = http2_fingerprint(PROFILE);
    assert!(
        diff(PROFILE_HTTP2_FINGERPRINT, &signature.render()).is_empty(),
        "HTTP/2 fingerprint drifted: expected={PROFILE_HTTP2_FINGERPRINT} actual={}",
        signature.render()
    );
}

/// The profile's HTTP/1.1 fingerprint must not drift.
#[test]
fn profile_header_fingerprint_matches_the_golden_value() {
    let names = profile_header_names(PROFILE);
    let signature = header_fingerprint(&names);
    assert_eq!(signature.prefix(), "ge11nn100000");
    assert_eq!(signature.hash(), "8bfb648a00ff");
}

/// The profile's JA4H fingerprint must not drift.
#[test]
fn profile_ja4h_fingerprint_matches_the_golden_value() {
    let names = profile_header_names(PROFILE);
    let signature = ja4h_fingerprint(&names);
    assert_eq!(signature.prefix(), "ge11nn10enus");
    assert_eq!(
        signature.render(),
        "ge11nn10enus_8bfb648a00ff_000000000000_000000000000"
    );
}

/// The fingerprint must react to a changed header set.
#[test]
fn fingerprint_changes_when_a_header_is_removed() {
    let mut names = profile_header_names(PROFILE);
    let before = header_fingerprint(&names).render();
    names.retain(|name| name != "priority");
    let after = header_fingerprint(&names).render();
    assert_ne!(before, after);
}

/// A profile that drops a header must get a different fingerprint.
#[test]
fn chrome_profiles_with_different_headers_have_distinct_fingerprints() {
    // Chrome 123 does not send the `priority` header, Chrome 152 does.
    let chrome123 = header_fingerprint(&profile_header_names(Emulation::Chrome123)).render();
    let chrome152 = header_fingerprint(&profile_header_names(Emulation::Chrome152)).render();
    assert_ne!(chrome123, chrome152);
}
