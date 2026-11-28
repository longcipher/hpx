//! Real ClientHello wire captures for the modern Chrome profiles.
//!
//! A raw loopback listener records the first TLS record produced by the
//! client. That lets the test assert on the bytes actually written to the
//! socket, without trusting the configuration structs or needing a TLS
//! server certificate.

#![cfg(feature = "boring-tls")]
#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used
)]

use std::time::Duration;

use hpx::{Client, tls::TlsOptions};
use hpx_emulation::Emulation;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};

/// Read one complete TLS handshake record from a raw TCP connection.
async fn capture_clienthello() -> (u16, Vec<u8>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let port = addr.port();

    let capture = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        socket.set_nodelay(true).expect("nodelay");
        let mut header = [0u8; 5];
        socket.read_exact(&mut header).await.expect("record header");
        assert_eq!(header[0], 22, "expected a TLS handshake record");
        let record_len = u16::from_be_bytes([header[3], header[4]]) as usize;
        let mut body = vec![0u8; record_len];
        socket.read_exact(&mut body).await.expect("record body");
        // Close the socket so the client's handshake fails fast.
        let _ = socket.shutdown().await;
        body
    });

    let client = Client::builder()
        .emulation(Emulation::Chrome152)
        .build()
        .expect("client");
    // The handshake is expected to fail: the listener is not a TLS server.
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        client.get(format!("https://127.0.0.1:{port}/")).send(),
    )
    .await;

    let body = tokio::time::timeout(Duration::from_secs(5), capture)
        .await
        .expect("capture finished")
        .expect("capture task");
    (port, body)
}

/// Parse the extension list out of a ClientHello handshake body.
fn extensions(handshake: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut cursor = 0usize;
    let mut take = |n: usize| -> &[u8] {
        let slice = &handshake[cursor..cursor + n];
        cursor += n;
        slice
    };

    assert_eq!(take(1)[0], 1, "ClientHello handshake type");
    let hs_len = {
        let b = take(3);
        ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | b[2] as usize
    };
    assert_eq!(
        hs_len,
        handshake.len() - 4,
        "handshake length matches record"
    );
    let _legacy_version = take(2);
    let _random = take(32);
    let session_id_len = take(1)[0] as usize;
    let _session_id = take(session_id_len);
    let cipher_suites_len = {
        let b = take(2);
        u16::from_be_bytes([b[0], b[1]]) as usize
    };
    let _cipher_suites = take(cipher_suites_len);
    let compression_len = take(1)[0] as usize;
    let _compression = take(compression_len);

    let ext_len = {
        let b = take(2);
        u16::from_be_bytes([b[0], b[1]]) as usize
    };
    let ext_block = take(ext_len);
    let mut out = Vec::new();
    let mut rest = ext_block;
    while !rest.is_empty() {
        assert!(rest.len() >= 4, "truncated extension header");
        let kind = u16::from_be_bytes([rest[0], rest[1]]);
        let len = u16::from_be_bytes([rest[2], rest[3]]) as usize;
        assert!(rest.len() >= 4 + len, "truncated extension body");
        out.push((kind, rest[4..4 + len].to_vec()));
        rest = &rest[4 + len..];
    }
    out
}

#[tokio::test]
async fn chrome152_sends_empty_trust_anchors_extension() {
    let (_port, body) = capture_clienthello().await;
    let exts = extensions(&body);

    // TLSEXT_TYPE_trust_anchors = 51764 = 0xCA34.
    let trust_anchors: Vec<_> = exts.iter().filter(|(kind, _)| *kind == 0xca34).collect();
    assert_eq!(
        trust_anchors.len(),
        1,
        "Chrome 152 must send exactly one trust_anchors extension"
    );
    // BoringSSL encodes the list as a two-byte length followed by IDs. An
    // empty list is therefore `00 00` on the wire.
    assert_eq!(
        trust_anchors[0].1,
        vec![0, 0],
        "Chrome 152 sends an empty trust_anchors list"
    );
}

#[tokio::test]
async fn chrome152_advertises_ml_dsa_and_signature_grease() {
    let (_port, body) = capture_clienthello().await;
    let exts = extensions(&body);

    // signature_algorithms = 13; body is a 2-byte list length followed by
    // u16 pairs.
    let sigalgs = exts
        .iter()
        .find(|(kind, _)| *kind == 13)
        .map(|(_, data)| data.clone())
        .expect("signature_algorithms extension");
    assert!(sigalgs.len() >= 2, "sigalg list needs a length prefix");
    let declared = u16::from_be_bytes([sigalgs[0], sigalgs[1]]) as usize;
    assert_eq!(
        declared,
        sigalgs.len() - 2,
        "sigalg list length matches body"
    );
    let (sigalg_pairs, _) = sigalgs[2..].as_chunks::<2>();
    let values: Vec<u16> = sigalg_pairs
        .iter()
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();

    // BoringSSL inserts signature-algorithm GREASE itself; its exact
    // codepoint is randomised per connection (0x?a?a per RFC 8701).
    assert!(
        values.iter().any(|value| {
            let hi = (*value >> 8) as u8;
            let lo = *value as u8;
            hi == lo && (hi & 0x0f) == 0x0a
        }),
        "missing signature-algorithm GREASE: {values:?}"
    );
    for codepoint in [0x0904, 0x0905, 0x0906] {
        assert!(
            values.contains(&codepoint),
            "missing Chrome 152 ML-DSA codepoint {codepoint:#06x}: {values:?}"
        );
    }
}

#[tokio::test]
async fn tls_options_controls_extension_presence() {
    // A default TlsOptions must not emit trust_anchors: the extension is
    // opt-in and only modern Chrome profiles request it.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();
    let capture = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut header = [0u8; 5];
        socket.read_exact(&mut header).await.expect("header");
        let mut body = vec![0u8; u16::from_be_bytes([header[3], header[4]]) as usize];
        socket.read_exact(&mut body).await.expect("body");
        let _ = socket.shutdown().await;
        body
    });

    let client = Client::builder()
        .tls_options(TlsOptions::default())
        .build()
        .expect("client");
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        client.get(format!("https://127.0.0.1:{port}/")).send(),
    )
    .await;

    let body = capture.await.expect("capture task");
    let exts = extensions(&body);
    assert!(
        exts.iter().all(|(kind, _)| *kind != 0xca34),
        "default TlsOptions must omit trust_anchors"
    );
}

#[tokio::test]
async fn distinct_profiles_produce_distinct_clienthellos() {
    let (_a, first) = capture_clienthello().await;
    // The random 32 bytes differ per connection; compare the ordered list of
    // extension IDs, which is stable and is what a WAF fingerprints.
    let first_ids: Vec<u16> = extensions(&first)
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();
    let capture = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut header = [0u8; 5];
        socket.read_exact(&mut header).await.expect("header");
        let mut body = vec![0u8; u16::from_be_bytes([header[3], header[4]]) as usize];
        socket.read_exact(&mut body).await.expect("body");
        let _ = socket.shutdown().await;
        body
    });
    let client = Client::builder()
        .emulation(Emulation::Chrome151)
        .build()
        .expect("client");
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        client.get(format!("https://127.0.0.1:{port}/")).send(),
    )
    .await;
    let second = capture.await.expect("capture task");
    let second_ids: Vec<u16> = extensions(&second)
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();

    assert!(
        first_ids.contains(&0xca34),
        "Chrome 152 capture should contain trust_anchors"
    );
    assert!(
        !second_ids.contains(&0xca34),
        "Chrome 151 capture should not contain trust_anchors"
    );
}
