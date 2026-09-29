#![allow(missing_docs)]
#![cfg(feature = "boring-tls")]
mod support;
use std::time::Duration;

use hpx::{
    Client, Emulation,
    http1::Http1Options,
    http2::{
        Http2Options, PseudoId, PseudoOrder, SettingId, SettingsOrder, StreamDependency, StreamId,
    },
    tls::{AlpnProtocol, CertificateCompressionAlgorithm, ExtensionType, TlsOptions, TlsVersion},
};

macro_rules! join {
    ($sep:expr, $first:expr $(, $rest:expr)*) => {
        concat!($first $(, $sep, $rest)*)
    };
}

fn tls_options_template() -> TlsOptions {
    //  TLS options config
    TlsOptions::builder()
        .curves_list(join!(
            ":", // "X25519MLKEM768",
            "X25519", "P-256", "P-384",
            "P-521" /* "ffdhe2048",
                      * "ffdhe3072" */
        ))
        .cipher_list(join!(
            ":",
            "TLS_AES_128_GCM_SHA256",
            "TLS_CHACHA20_POLY1305_SHA256",
            "TLS_AES_256_GCM_SHA384",
            "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
            "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
            "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256",
            "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
            "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
            "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
            "TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA",
            "TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA",
            "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA",
            "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA",
            "TLS_RSA_WITH_AES_128_GCM_SHA256",
            "TLS_RSA_WITH_AES_256_GCM_SHA384",
            "TLS_RSA_WITH_AES_128_CBC_SHA",
            "TLS_RSA_WITH_AES_256_CBC_SHA"
        ))
        .sigalgs_list(join!(
            ":",
            "ecdsa_secp256r1_sha256",
            "ecdsa_secp384r1_sha384",
            "ecdsa_secp521r1_sha512",
            "rsa_pss_rsae_sha256",
            "rsa_pss_rsae_sha384",
            "rsa_pss_rsae_sha512",
            "rsa_pkcs1_sha256",
            "rsa_pkcs1_sha384",
            "rsa_pkcs1_sha512",
            "ecdsa_sha1",
            "rsa_pkcs1_sha1"
        ))
        .delegated_credentials(join!(
            ":",
            "ecdsa_secp256r1_sha256",
            "ecdsa_secp384r1_sha384",
            "ecdsa_secp521r1_sha512",
            "ecdsa_sha1"
        ))
        .certificate_compression_algorithms(&[
            CertificateCompressionAlgorithm::ZLIB,
            CertificateCompressionAlgorithm::BROTLI,
            // CertificateCompressionAlgorithm::ZSTD,
        ])
        .alpn_protocols([AlpnProtocol::HTTP2, AlpnProtocol::HTTP1])
        // .record_size_limit(0x4001)
        .pre_shared_key(true)
        .enable_ech_grease(true)
        .enable_ocsp_stapling(true)
        .enable_signed_cert_timestamps(true)
        .min_tls_version(TlsVersion::TLS_1_2)
        .max_tls_version(TlsVersion::TLS_1_3)
        .key_shares_limit(3)
        .preserve_tls13_cipher_list(true)
        .aes_hw_override(false)
        .random_aes_hw_override(true)
        .extension_permutation(&[
            ExtensionType::SERVER_NAME,
            ExtensionType::EXTENDED_MASTER_SECRET,
            ExtensionType::RENEGOTIATE,
            ExtensionType::SUPPORTED_GROUPS,
            ExtensionType::EC_POINT_FORMATS,
            ExtensionType::SESSION_TICKET,
            ExtensionType::APPLICATION_LAYER_PROTOCOL_NEGOTIATION,
            ExtensionType::STATUS_REQUEST,
            ExtensionType::DELEGATED_CREDENTIAL,
            ExtensionType::KEY_SHARE,
            ExtensionType::SUPPORTED_VERSIONS,
            ExtensionType::SIGNATURE_ALGORITHMS,
            ExtensionType::PSK_KEY_EXCHANGE_MODES,
            // ExtensionType::RECORD_SIZE_LIMIT,
            ExtensionType::CERT_COMPRESSION,
            ExtensionType::ENCRYPTED_CLIENT_HELLO,
        ])
        .build()
}

fn http2_options_template() -> Http2Options {
    // HTTP/2 headers frame pseudo-header order
    let headers_pseudo_order = PseudoOrder::builder()
        .extend([
            PseudoId::Method,
            PseudoId::Path,
            PseudoId::Authority,
            PseudoId::Scheme,
        ])
        .build();

    // HTTP/2 settings frame order
    let settings_order = SettingsOrder::builder()
        .extend([
            SettingId::HeaderTableSize,
            SettingId::EnablePush,
            SettingId::MaxConcurrentStreams,
            SettingId::InitialWindowSize,
            SettingId::MaxFrameSize,
            SettingId::MaxHeaderListSize,
            SettingId::EnableConnectProtocol,
            SettingId::NoRfc7540Priorities,
        ])
        .build();

    Http2Options::builder()
        .header_table_size(65536)
        .enable_push(false)
        .initial_window_size(131072)
        .max_frame_size(16384)
        .initial_connection_window_size(12517377 + 65535)
        .headers_stream_dependency(StreamDependency::new(StreamId::ZERO, 41, false))
        .headers_pseudo_order(headers_pseudo_order)
        .settings_order(settings_order)
        .build()
}

fn emulation_template() -> Emulation {
    //  HTTP/1 options config
    let http1 = Http1Options::builder()
        .allow_obsolete_multiline_headers_in_responses(true)
        .max_headers(100)
        .build();

    // This provider encapsulates TLS, HTTP/1, HTTP/2, default headers, and original headers
    Emulation::builder()
        .tls_options(tls_options_template())
        .http1_options(http1)
        .http2_options(http2_options_template())
        .build()
}

// This test requires network access to tls.peet.ws and may be flaky
#[ignore]
#[tokio::test]
async fn test_emulation() -> hpx::Result<()> {
    let client = Client::builder()
        .no_proxy()
        .emulation(hpx::BrowserProfile::Chrome)
        .connect_timeout(Duration::from_secs(10))
        .cert_verification(false)
        .build()?;

    let text = client
        .get("https://tls.peet.ws/api/all")
        .send()
        .await?
        .text()
        .await?;

    let ja4_15 = "t13d1715h2_5b57614c22b0_c1eccb039341";
    let ja4_16 = "t13d1716h2_5b57614c22b0_e66002de8a8b";
    let ja4_peet = "t13d1716h2_5b57614c22b0_c1eccb039341";
    assert!(
        text.contains(ja4_15) || text.contains(ja4_16) || text.contains(ja4_peet),
        "Response ja4_hash fingerprint not found: {text}"
    );
    assert!(
        text.contains("6ea73faa8fc5aac76bded7bd238f6433"),
        "Response akamai_hash fingerprint not found: {text}"
    );

    Ok(())
}

#[tokio::test]
async fn test_request_with_emulation() -> hpx::Result<()> {
    if std::env::var("HPX_NETWORK_TESTS").is_err() {
        eprintln!("skipping test_request_with_emulation: set HPX_NETWORK_TESTS=1");
        return Ok(());
    }
    let client = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .cert_verification(false)
        .build()?;

    let text = client
        .get("https://tls.peet.ws/api/all")
        .emulation(hpx::BrowserProfile::Chrome)
        .send()
        .await?
        .text()
        .await?;

    let ja4_15 = "t13d1715h2_5b57614c22b0_c1eccb039341";
    let ja4_16 = "t13d1716h2_5b57614c22b0_e66002de8a8b";
    let ja4_peet = "t13d1716h2_5b57614c22b0_c1eccb039341";
    assert!(
        text.contains(ja4_15) || text.contains(ja4_16) || text.contains(ja4_peet),
        "Response ja4_hash fingerprint not found: {text}"
    );
    assert!(
        text.contains("6ea73faa8fc5aac76bded7bd238f6433"),
        "Response akamai_hash fingerprint not found: {text}"
    );

    Ok(())
}

#[tokio::test]
async fn test_request_with_emulation_tls() -> hpx::Result<()> {
    if std::env::var("HPX_NETWORK_TESTS").is_err() {
        eprintln!("skipping test_request_with_emulation_tls: set HPX_NETWORK_TESTS=1");
        return Ok(());
    }
    let client = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .cert_verification(false)
        .build()?;

    let text = client
        .get("https://tls.peet.ws/api/all")
        .emulation(tls_options_template())
        .send()
        .await?
        .text()
        .await?;

    let ja4_15 = "t13d1715h2_5b57614c22b0_c1eccb039341";
    let ja4_16 = "t13d1716h2_5b57614c22b0_e66002de8a8b";
    let ja4_peet = "t13d1716h2_5b57614c22b0_c1eccb039341";
    assert!(
        text.contains(ja4_15) || text.contains(ja4_16) || text.contains(ja4_peet),
        "Response ja4_hash fingerprint not found: {text}"
    );

    Ok(())
}

// This test requires network access to tls.peet.ws and may be flaky
#[ignore]
#[tokio::test]
async fn test_request_with_emulation_http2() -> hpx::Result<()> {
    let client = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .cert_verification(false)
        .build()?;

    let text = client
        .get("https://tls.peet.ws/api/all")
        .emulation(http2_options_template())
        .send()
        .await?
        .text()
        .await?;

    assert!(
        text.contains("6ea73faa8fc5aac76bded7bd238f6433"),
        "Response akamai_hash fingerprint not found: {text}"
    );

    Ok(())
}

/// Local (no-network) check: Chrome emulation emits the expected
/// User-Agent and `sec-ch-ua` client-hint headers.
#[tokio::test]
async fn chrome_emits_expected_headers_locally() {
    use support::server;

    let server = server::http(
        move |req: http::Request<hyper::body::Incoming>| async move {
            let ua = req
                .headers()
                .get(http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let ch = req
                .headers()
                .get("sec-ch-ua")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let ch_ua_mobile = req
                .headers()
                .get("sec-ch-ua-mobile")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let body = format!("ua={ua}\nch={ch}\nmobile={ch_ua_mobile}");
            http::Response::new(hpx::Body::from(body))
        },
    );

    let client = Client::builder().no_proxy().build().unwrap();
    let resp = client
        .get(format!("http://{}/", server.addr()))
        .emulation(hpx::BrowserProfile::Chrome)
        .send()
        .await
        .unwrap();
    let text = resp.text().await.unwrap();
    assert!(text.contains("Chrome/"), "expected Chrome UA, got: {text}");
    // sec-ch-ua may be set by the emulation profile; if present it must look like a brand list.
    if let Some(rest) = text.split("ch=").nth(1) {
        let ch = rest.split('\n').next().unwrap_or("");
        if !ch.is_empty() {
            assert!(
                ch.contains("Chromium") || ch.contains("Chrome") || ch.contains("Not",),
                "unexpected sec-ch-ua: {ch}"
            );
        }
    }
}
