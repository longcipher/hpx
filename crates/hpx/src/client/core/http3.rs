//! HTTP/3 connection configuration.
//!
//! This module defines [`Http3Options`] for configuring QUIC transport parameters
//! (mapped to `quinn::TransportConfig`), HTTP/3 protocol parameters (mapped to
//! `hpx_h3::client::builder`), fingerprint hooks, and extended CONNECT support for
//! [RFC 9220] WebSocket-over-h3.
//!
//! The structure mirrors [`crate::client::core::http2::Http2Options`] for
//! consistency across protocol versions.
//!
//! [RFC 9220]: https://www.rfc-editor.org/rfc/rfc9220.html

use std::time::Duration;

// Re-export HTTP/3 connector primitives so integration tests (and downstream
// users) can construct `QuicConnector` and `H3Connection` directly via
// `hpx::http3::{QuicConnector, H3Connection, H3Error}`. Marked `#[doc(hidden)]`
// to keep them out of the public API surface — they exist primarily to support
// BDD scenarios in `crates/hpx/tests/http3.rs` (T1.7 scope) and may be
// refactored into a stable public API in a later task.
#[cfg(feature = "http3")]
#[doc(hidden)]
pub use crate::client::conn::quic::{H3Connection, H3Error, QuicConnector};
// Re-export `drive_request` so the HTTP client pipeline (`client::http::client`)
// can drive h3 requests without having to reach into the private `proto`
// module. `pub(crate)` because `drive_request` is itself `pub(crate)`; the
// re-export preserves the original visibility and does NOT widen it.
//
// Without this re-export, `client::http::client` would have to write
// `crate::client::core::proto::h3::client::drive_request`, but `proto` is
// private to `core` (`mod proto;` in `core.rs`), so that path is inaccessible
// from outside `core`. Re-exporting from `core::http3` (which is
// `pub mod http3;`) bypasses the private `proto` boundary.
#[cfg(feature = "http3")]
pub(crate) use crate::client::core::proto::h3::client::drive_request;
// Re-export WebSocket-over-h3 types so integration tests can use them.
// Marked `#[doc(hidden)]` — not part of the stable public API.
#[cfg(feature = "http3")]
#[doc(hidden)]
pub use crate::client::core::proto::h3::client::{H3WebSocket, WsError, WsMessage};

/// Construct a [`ConnectRequest`](crate::client::http::ConnectRequest) for use
/// in integration tests and BDD scenarios.
///
/// `#[doc(hidden)]` — exposed solely so that `crates/hpx/tests/http3.rs` can
/// synthesize a connection request without going through the full
/// [`Client`](crate::Client) pipeline. Not part of the stable public API and
/// may be removed or renamed in a future release.
#[cfg(feature = "http3")]
#[doc(hidden)]
#[inline]
pub fn __test_connect_request(uri: http::Uri) -> crate::client::http::ConnectRequest {
    crate::client::http::ConnectRequest::new(uri, None)
}

/// Low-level QUIC transport configuration escape hatch.
///
/// This is a transparent alias for [`quinn::TransportConfig`] — the same type
/// `QuicConnector` (T1.4) and `tls::quic::build_transport_config` (T1.5)
/// already use internally. Exposing it as `QuicConfig` lets users override the
/// default transport config derived from [`Http3Options`] via
/// [`ClientBuilder::quic_config`](crate::ClientBuilder::quic_config) without
/// having to import `quinn` directly or re-derive every field themselves.
///
/// When [`ClientBuilder::quic_config`](crate::ClientBuilder::quic_config) is
/// called, the supplied `QuicConfig` is stored verbatim on the
/// [`ClientBuilder`](crate::ClientBuilder) and consumed by `QuicConnector` in
/// `Client::build` (T1.7 scope). Until T1.7 lands, the override is stored
/// but not yet applied to the connector; a `// TODO(T1.7)` marker in
/// `Client::build` calls out the wiring point.
///
/// Reqwest parity: reqwest exposes `reqwest::QuicConfig` as a struct with
/// per-field setters. hpx takes the simpler alias approach for Phase 1 (YAGNI);
/// if a structured wrapper becomes useful later (e.g., to validate cross-field
/// invariants), this alias can be promoted to a newtype without breaking
/// callers that construct one via `QuicConfig::default()`.
pub type QuicConfig = quinn::TransportConfig;

/// Identifies an HTTP/3 SETTINGS entry for fingerprint-controlled ordering.
///
/// Variants correspond to the HTTP/3 and QPACK setting identifiers defined in
/// [RFC 9114] and [RFC 9204]. The [`H3SettingId::Grease`] variant allows
/// inserting arbitrary grease identifiers per [RFC 8701] for fingerprint
/// flexibility.
///
/// [RFC 9114]: https://www.rfc-editor.org/rfc/rfc9114.html
/// [RFC 9204]: https://www.rfc-editor.org/rfc/rfc9204.html
/// [RFC 8701]: https://www.rfc-editor.org/rfc/rfc8701.html
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum H3SettingId {
    /// `QPACK_MAX_TABLE_CAPACITY` — [RFC 9204 §3.2.1].
    ///
    /// [RFC 9204 §3.2.1]: https://www.rfc-editor.org/rfc/rfc9204.html#section-3.2.1
    QpackMaxTableCapacity,

    /// `MAX_FIELD_SECTION_SIZE` — [RFC 9114 §4.2.2].
    ///
    /// [RFC 9114 §4.2.2]: https://www.rfc-editor.org/rfc/rfc9114.html#section-4.2.2
    MaxFieldSectionSize,

    /// `QPACK_BLOCKED_STREAMS` — [RFC 9204 §3.2.2].
    ///
    /// [RFC 9204 §3.2.2]: https://www.rfc-editor.org/rfc/rfc9204.html#section-3.2.2
    QpackBlockedStreams,

    /// `H3_NUM_PLACEHOLDERS` — [RFC 9297Extensions] (draft-ietf-httpbis-resilience).
    ///
    /// [RFC 9297Extensions]: https://datatracker.ietf.org/doc/html/draft-ietf-httpbis-resilience
    NumPlaceholders,

    /// A grease setting identifier per [RFC 8701], carrying an arbitrary
    /// 62-bit identifier for fingerprint emulation.
    ///
    /// [RFC 8701]: https://www.rfc-editor.org/rfc/rfc8701.html
    Grease(u64),
}

/// Configuration for an HTTP/3 connection.
///
/// This struct defines QUIC transport parameters (mapped to
/// `quinn::TransportConfig`), HTTP/3 protocol parameters (mapped to
/// `hpx_h3::client::builder`), fingerprint hooks mirroring the HTTP/2
/// `experimental_settings` mechanism, and the extended CONNECT flag for
/// [RFC 9220] WebSocket-over-h3.
///
/// The [`Default`] implementation matches the Chrome 143 baseline observed in
/// the wild. Fields are `Option`-typed where `None` means "do not send the
/// parameter" and `Some(v)` means "advertise `v` to the peer".
///
/// [RFC 9220]: https://www.rfc-editor.org/rfc/rfc9220.html
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Http3Options {
    // ===== QUIC transport parameters (mapped to quinn::TransportConfig) =====
    /// QUIC `max_idle_timeout` transport parameter ([RFC 9000 §18.2]).
    ///
    /// [RFC 9000 §18.2]: https://www.rfc-editor.org/rfc/rfc9000.html#section-18.2
    pub max_idle_timeout: Option<Duration>,

    /// Per-stream flow-control receive window advertised to the peer.
    pub stream_receive_window: Option<u64>,

    /// Connection-level flow-control receive window advertised to the peer.
    pub conn_receive_window: Option<u64>,

    /// Connection-level flow-control send window enforced locally.
    pub send_window: Option<u64>,

    /// Whether to use BBR congestion control. `false` selects CUBIC (the
    /// `quinn` default).
    pub congestion_bbr: bool,

    /// QUIC `initial_max_streams_bidi` transport parameter — maximum
    /// concurrent bidirectional streams the peer may initiate.
    pub max_concurrent_bidi_streams: Option<u64>,

    /// QUIC `initial_max_streams_uni` transport parameter — maximum
    /// concurrent unidirectional streams the peer may initiate.
    pub max_concurrent_uni_streams: Option<u64>,

    /// QUIC `initial_max_data` transport parameter — aggregate connection
    /// flow-control limit.
    pub initial_max_data: Option<u64>,

    /// QUIC `initial_max_stream_data_bidi_local` transport parameter.
    pub initial_max_stream_data_bidi_local: Option<u64>,

    /// QUIC `initial_max_stream_data_bidi_remote` transport parameter.
    pub initial_max_stream_data_bidi_remote: Option<u64>,

    /// QUIC `initial_max_stream_data_uni` transport parameter.
    pub initial_max_stream_data_uni: Option<u64>,

    /// QUIC `max_udp_payload_size` transport parameter
    /// ([RFC 9000 §18.2]). Controls the maximum size of UDP payloads
    /// the endpoint is willing to receive.
    ///
    /// [RFC 9000 §18.2]: https://www.rfc-editor.org/rfc/rfc9000.html#section-18.2
    pub max_udp_payload_size: Option<u32>,

    /// QUIC `active_connection_id_limit` transport parameter
    /// ([RFC 9000 §18.2]).
    ///
    /// [RFC 9000 §18.2]: https://www.rfc-editor.org/rfc/rfc9000.html#section-18.2
    pub active_connection_id_limit: Option<u32>,

    /// QUIC Initial Packet padding size in bytes.
    ///
    /// The QUIC Initial Packet is padded to this size before encryption,
    /// which is observable to network fingerprinters. Chrome pads to 1200
    /// bytes, Firefox to 1232 bytes, and Safari/WebKit does not pad.
    ///
    /// `None` (default) means no explicit padding — the QUIC library
    /// decides the initial packet size. NOTE: quinn 0.11 does not expose
    /// a public API for setting initial packet padding; this field is
    /// stored for fingerprint documentation and emulation purposes and
    /// requires quinn upstream support to take effect.
    pub initial_packet_padding: Option<u16>,

    /// Whether to enable 0-RTT (early data). Maps to
    /// `rustls::ClientConfig::enable_early_data`.
    pub enable_0rtt: bool,

    // ===== h3 protocol parameters (mapped to hpx_h3::client::builder) =====
    /// `MAX_FIELD_SECTION_SIZE` — [RFC 9114 §4.2.2]. Bounds the size of
    /// request/response header blocks.
    ///
    /// [RFC 9114 §4.2.2]: https://www.rfc-editor.org/rfc/rfc9114.html#section-4.2.2
    pub max_field_section_size: Option<u64>,

    /// Whether to send [RFC 8701] GREASE in the SETTINGS frame.
    ///
    /// [RFC 8701]: https://www.rfc-editor.org/rfc/rfc8701.html
    pub send_grease: bool,

    /// `QPACK_MAX_TABLE_CAPACITY` — [RFC 9204 §3.2.1].
    ///
    /// [RFC 9204 §3.2.1]: https://www.rfc-editor.org/rfc/rfc9204.html#section-3.2.1
    pub qpack_max_table_capacity: Option<u64>,

    /// `QPACK_BLOCKED_STREAMS` — [RFC 9204 §3.2.2].
    ///
    /// [RFC 9204 §3.2.2]: https://www.rfc-editor.org/rfc/rfc9204.html#section-3.2.2
    pub qpack_blocked_streams: Option<u64>,

    // ===== Fingerprint hooks (mirrors h2 experimental_settings) =====
    /// Arbitrary `(setting_identifier, value)` pairs appended to the h3
    /// SETTINGS frame for fingerprint emulation.
    pub experimental_settings: Vec<(u64, u64)>,

    /// Explicit ordering of h3 SETTINGS entries for fingerprint control.
    /// Entries not listed here are appended in their default order.
    pub settings_order: Vec<H3SettingId>,

    // ===== Extended CONNECT for RFC 9220 WebSocket over h3 =====
    /// Whether to enable the extended CONNECT protocol for [RFC 9220]
    /// WebSocket-over-h3. Opt-in; Phase 2 enables it.
    ///
    /// [RFC 9220]: https://www.rfc-editor.org/rfc/rfc9220.html
    pub enable_connect_protocol: bool,
}

impl Http3Options {
    /// Returns the default options, then applies `f` to mutate them before returning.
    pub fn customize(f: impl FnOnce(&mut Self)) -> Self {
        let mut opts = Self::default();
        f(&mut opts);
        opts
    }

    /// Setter for `max_idle_timeout`.
    #[inline]
    pub fn with_max_idle_timeout(mut self, v: Option<Duration>) -> Self {
        self.max_idle_timeout = v;
        self
    }

    /// Setter for `stream_receive_window`.
    #[inline]
    pub fn with_stream_receive_window(mut self, v: Option<u64>) -> Self {
        self.stream_receive_window = v;
        self
    }

    /// Setter for `conn_receive_window`.
    #[inline]
    pub fn with_conn_receive_window(mut self, v: Option<u64>) -> Self {
        self.conn_receive_window = v;
        self
    }

    /// Setter for `send_window`.
    #[inline]
    pub fn with_send_window(mut self, v: Option<u64>) -> Self {
        self.send_window = v;
        self
    }

    /// Setter for `congestion_bbr`.
    #[inline]
    pub fn with_congestion_bbr(mut self, v: bool) -> Self {
        self.congestion_bbr = v;
        self
    }

    /// Setter for `max_concurrent_bidi_streams`.
    #[inline]
    pub fn with_max_concurrent_bidi_streams(mut self, v: Option<u64>) -> Self {
        self.max_concurrent_bidi_streams = v;
        self
    }

    /// Setter for `max_concurrent_uni_streams`.
    #[inline]
    pub fn with_max_concurrent_uni_streams(mut self, v: Option<u64>) -> Self {
        self.max_concurrent_uni_streams = v;
        self
    }

    /// Setter for `initial_max_data`.
    #[inline]
    pub fn with_initial_max_data(mut self, v: Option<u64>) -> Self {
        self.initial_max_data = v;
        self
    }

    /// Setter for `initial_max_stream_data_bidi_local`.
    #[inline]
    pub fn with_initial_max_stream_data_bidi_local(mut self, v: Option<u64>) -> Self {
        self.initial_max_stream_data_bidi_local = v;
        self
    }

    /// Setter for `initial_max_stream_data_bidi_remote`.
    #[inline]
    pub fn with_initial_max_stream_data_bidi_remote(mut self, v: Option<u64>) -> Self {
        self.initial_max_stream_data_bidi_remote = v;
        self
    }

    /// Setter for `initial_max_stream_data_uni`.
    #[inline]
    pub fn with_initial_max_stream_data_uni(mut self, v: Option<u64>) -> Self {
        self.initial_max_stream_data_uni = v;
        self
    }

    /// Setter for `max_udp_payload_size`.
    #[inline]
    pub fn with_max_udp_payload_size(mut self, v: Option<u32>) -> Self {
        self.max_udp_payload_size = v;
        self
    }

    /// Setter for `active_connection_id_limit`.
    #[inline]
    pub fn with_active_connection_id_limit(mut self, v: Option<u32>) -> Self {
        self.active_connection_id_limit = v;
        self
    }

    /// Setter for `initial_packet_padding`.
    #[inline]
    pub fn with_initial_packet_padding(mut self, v: Option<u16>) -> Self {
        self.initial_packet_padding = v;
        self
    }

    /// Setter for `enable_0rtt`.
    #[inline]
    pub fn with_enable_0rtt(mut self, v: bool) -> Self {
        self.enable_0rtt = v;
        self
    }

    /// Setter for `max_field_section_size`.
    #[inline]
    pub fn with_max_field_section_size(mut self, v: Option<u64>) -> Self {
        self.max_field_section_size = v;
        self
    }

    /// Setter for `send_grease`.
    #[inline]
    pub fn with_send_grease(mut self, v: bool) -> Self {
        self.send_grease = v;
        self
    }

    /// Setter for `qpack_max_table_capacity`.
    #[inline]
    pub fn with_qpack_max_table_capacity(mut self, v: Option<u64>) -> Self {
        self.qpack_max_table_capacity = v;
        self
    }

    /// Setter for `qpack_blocked_streams`.
    #[inline]
    pub fn with_qpack_blocked_streams(mut self, v: Option<u64>) -> Self {
        self.qpack_blocked_streams = v;
        self
    }

    /// Setter for `experimental_settings`.
    #[inline]
    pub fn with_experimental_settings(mut self, v: Vec<(u64, u64)>) -> Self {
        self.experimental_settings = v;
        self
    }

    /// Setter for `settings_order`.
    #[inline]
    pub fn with_settings_order(mut self, v: Vec<H3SettingId>) -> Self {
        self.settings_order = v;
        self
    }

    /// Setter for `enable_connect_protocol`.
    #[inline]
    pub fn with_enable_connect_protocol(mut self, v: bool) -> Self {
        self.enable_connect_protocol = v;
        self
    }
}

impl Default for Http3Options {
    /// Returns the Chrome 143 baseline for QUIC transport parameters and h3
    /// SETTINGS. These defaults are chosen to match a modern Chromium client
    /// so that `Http3Options::default()` produces a realistic fingerprint.
    #[inline]
    fn default() -> Self {
        Http3Options {
            // QUIC transport parameters (Chrome 143 baseline)
            max_idle_timeout: Some(Duration::from_secs(30)),
            stream_receive_window: Some(8 * 1024 * 1024),
            conn_receive_window: Some(8 * 1024 * 1024),
            send_window: Some(8 * 1024 * 1024),
            congestion_bbr: false,
            max_concurrent_bidi_streams: Some(100),
            max_concurrent_uni_streams: Some(100),
            initial_max_data: Some(10 * 1024 * 1024),
            initial_max_stream_data_bidi_local: Some(8 * 1024 * 1024),
            initial_max_stream_data_bidi_remote: Some(8 * 1024 * 1024),
            initial_max_stream_data_uni: Some(8 * 1024 * 1024),
            max_udp_payload_size: Some(65527),
            active_connection_id_limit: Some(2),
            initial_packet_padding: None,
            enable_0rtt: true,

            // h3 protocol parameters (Chrome 143 baseline)
            max_field_section_size: Some(16 * 1024),
            send_grease: true,
            qpack_max_table_capacity: Some(4096),
            qpack_blocked_streams: Some(100),

            // Fingerprint hooks — empty by default; emulation profiles fill these.
            experimental_settings: Vec::new(),
            settings_order: vec![
                H3SettingId::QpackMaxTableCapacity,
                H3SettingId::MaxFieldSectionSize,
                H3SettingId::QpackBlockedStreams,
            ],

            // RFC 9220 — enabled by default since we support WebSocket.
            enable_connect_protocol: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http3_options_default_matches_chrome_143() {
        let opts = Http3Options::default();

        // QUIC transport parameters (mapped to quinn::TransportConfig)
        assert_eq!(opts.max_idle_timeout, Some(Duration::from_secs(30)));
        assert_eq!(opts.stream_receive_window, Some(8 * 1024 * 1024));
        assert_eq!(opts.conn_receive_window, Some(8 * 1024 * 1024));
        assert_eq!(opts.send_window, Some(8 * 1024 * 1024));
        assert!(!opts.congestion_bbr);
        assert_eq!(opts.max_concurrent_bidi_streams, Some(100));
        assert_eq!(opts.max_concurrent_uni_streams, Some(100));
        assert_eq!(opts.initial_max_data, Some(10 * 1024 * 1024));
        assert_eq!(
            opts.initial_max_stream_data_bidi_local,
            Some(8 * 1024 * 1024)
        );
        assert_eq!(
            opts.initial_max_stream_data_bidi_remote,
            Some(8 * 1024 * 1024)
        );
        assert_eq!(opts.initial_max_stream_data_uni, Some(8 * 1024 * 1024));
        assert_eq!(opts.max_udp_payload_size, Some(65527));
        assert_eq!(opts.active_connection_id_limit, Some(2));
        assert_eq!(opts.initial_packet_padding, None);
        assert!(opts.enable_0rtt);

        // h3 protocol parameters (mapped to hpx_h3::client::builder)
        assert_eq!(opts.max_field_section_size, Some(16 * 1024));
        assert!(opts.send_grease);
        assert_eq!(opts.qpack_max_table_capacity, Some(4096));
        assert_eq!(opts.qpack_blocked_streams, Some(100));

        // Fingerprint hooks (mirrors h2 experimental_settings)
        assert!(opts.experimental_settings.is_empty());
        assert_eq!(
            opts.settings_order,
            vec![
                H3SettingId::QpackMaxTableCapacity,
                H3SettingId::MaxFieldSectionSize,
                H3SettingId::QpackBlockedStreams,
            ]
        );

        // Extended CONNECT for RFC 9220 WebSocket over h3 (enabled by default)
        assert!(opts.enable_connect_protocol);
    }

    #[test]
    fn http3_options_has_initial_packet_padding_field() {
        let opts = Http3Options::default();
        // Default is None (no explicit padding).
        assert_eq!(opts.initial_packet_padding, None);
    }

    #[test]
    fn chrome_96_firefox_88_have_different_initial_packet_padding() {
        // Chrome 96 pads to 1200 bytes (Chrome baseline).
        let chrome_opts = Http3Options {
            initial_packet_padding: Some(1200),
            ..Http3Options::default()
        };
        assert_eq!(chrome_opts.initial_packet_padding, Some(1200));

        // Firefox 88 pads to 1232 bytes (Firefox baseline).
        let ff_opts = Http3Options {
            initial_packet_padding: Some(1232),
            ..Http3Options::default()
        };
        assert_eq!(ff_opts.initial_packet_padding, Some(1232));

        // Safari/WebKit does not pad the initial packet.
        let safari_opts = Http3Options {
            initial_packet_padding: None,
            ..Http3Options::default()
        };
        assert_eq!(safari_opts.initial_packet_padding, None);

        // Verify Chrome and Firefox padding values differ.
        assert_ne!(
            chrome_opts.initial_packet_padding,
            ff_opts.initial_packet_padding
        );
    }

    #[test]
    fn initial_packet_padding_can_be_set_to_chrome_baseline() {
        let opts = Http3Options {
            initial_packet_padding: Some(1200),
            ..Http3Options::default()
        };
        assert_eq!(opts.initial_packet_padding, Some(1200));
    }

    #[test]
    fn initial_packet_padding_can_be_set_to_firefox_baseline() {
        let opts = Http3Options {
            initial_packet_padding: Some(1232),
            ..Http3Options::default()
        };
        assert_eq!(opts.initial_packet_padding, Some(1232));
    }
}
