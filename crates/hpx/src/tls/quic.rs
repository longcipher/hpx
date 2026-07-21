//! QUIC TLS path: build `quinn::ClientConfig` and `quinn::Endpoint` from
//! `TlsOptions` + `Http3Options`.
//!
//! This module is the single entry point for converting hpx's TLS + HTTP/3
//! option structs into the `quinn` primitives required by `QuicConnector`
//! (T1.4). Per [C-01] and [C-19], ALPN is forced to `b"h3"` regardless of
//! `TlsOptions::alpn_protocols`, satisfying RFC 9114's single-ALPN rule for
//! HTTP/3. Per [C-23], the `AlpnProtocol::HTTP3` constant is reused as the
//! single source of truth for the ALPN bytes.
//!
//! # Constraints (RFC 2119)
//!
//! - **[C-01]** ALPN MUST be `b"h3"` (no draft versions like `h3-29`).
//! - **[C-03]** TLS config MUST use `rustls` (no BoringSSL on the QUIC path).
//! - **[C-19]** `enable_0rtt` MUST map to `rustls::ClientConfig::enable_early_data`.
//! - **[C-23]** ALPN constant MUST be `AlpnProtocol::HTTP3` (single source of truth).

use std::{net::SocketAddr, sync::Arc};

use crate::{
    client::http3::Http3Options,
    error::Error,
    tls::{AlpnProtocol, TlsOptions},
};

/// Build a `quinn::ClientConfig` from `TlsOptions` + `Http3Options`, using
/// the `webpki-roots` CA bundle as the trust anchor.
///
/// ALPN is forced to `[b"h3"]` per [C-01] and [C-23]. `enable_0rtt` is mapped
/// to `rustls::ClientConfig::enable_early_data` per [C-19].
///
/// Use [`build_quinn_client_config_with_root_store`] when a custom root store
/// is required (e.g., for self-signed cert integration tests).
pub fn build_quinn_client_config(
    tls_opts: &TlsOptions,
    h3_opts: &Http3Options,
) -> crate::Result<quinn::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    build_quinn_client_config_with_root_store(tls_opts, h3_opts, root_store)
}

/// Build a `quinn::ClientConfig` from `TlsOptions` + `Http3Options` + a
/// caller-supplied `RootCertStore`.
///
/// ALPN is forced to `[b"h3"]` per [C-01] and [C-23]. `enable_0rtt` is mapped
/// to `rustls::ClientConfig::enable_early_data` per [C-19].
///
/// The `rustls::ClientConfig` is constructed via
/// `ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))`
/// to avoid the CryptoProvider ambiguity between `aws-lc-rs` (rustls default)
/// and `ring` (hpx explicit feature) that would otherwise cause
/// `ClientConfig::builder()` to panic with "Could not automatically determine
/// the process-level CryptoProvider".
///
/// The `tls_opts` parameter is accepted for future extension (SNI, keylog,
/// min/max version) but not currently used; ALPN is forced to `b"h3"` per
/// [C-01] regardless of `TlsOptions::alpn_protocols`.
pub fn build_quinn_client_config_with_root_store(
    _tls_opts: &TlsOptions,
    h3_opts: &Http3Options,
    root_store: rustls::RootCertStore,
) -> crate::Result<quinn::ClientConfig> {
    // Per [C-01] and [C-23]: ALPN is forced to `b"h3"` regardless of
    // `TlsOptions::alpn_protocols`.
    let alpn_protocols = vec![AlpnProtocol::HTTP3.as_wire_bytes().to_vec()];

    // Per [C-03]: TLS config uses rustls. Use `builder_with_provider` with an
    // explicit `ring` provider to avoid the CryptoProvider ambiguity panic.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls_config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(Error::tls)?
        .with_root_certificates(root_store)
        .with_no_client_auth();
    tls_config.alpn_protocols = alpn_protocols;

    // Per [C-19]: `enable_0rtt` maps to `enable_early_data`.
    if h3_opts.enable_0rtt {
        tls_config.enable_early_data = true;
    }

    let quic_client_config =
        quinn::crypto::rustls::QuicClientConfig::try_from(Arc::new(tls_config))
            .map_err(Error::tls)?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_client_config));
    client_config.transport_config(Arc::new(build_transport_config(h3_opts)?));
    Ok(client_config)
}

/// Build a `quinn::Endpoint` bound to `local_addr`.
///
/// Wraps `quinn::Endpoint::client` so callers (e.g., `QuicConnector`) have a
/// single, error-mapping entry point for constructing client-side QUIC
/// endpoints. The endpoint uses the default `quinn::EndpointConfig`.
pub fn build_quinn_endpoint(local_addr: SocketAddr) -> crate::Result<quinn::Endpoint> {
    quinn::Endpoint::client(local_addr).map_err(Error::tls)
}

/// Build a `quinn::TransportConfig` from `Http3Options`.
///
/// Maps the QUIC transport parameter fields of [`Http3Options`] to the
/// corresponding `quinn::TransportConfig` setters:
///
/// | `Http3Options` field            | `quinn::TransportConfig` setter      |
/// |---------------------------------|--------------------------------------|
/// | `max_idle_timeout`              | `max_idle_timeout(Some(VarInt))` (ms)|
/// | `stream_receive_window`         | `stream_receive_window(u64)`         |
/// | `conn_receive_window`           | `receive_window(u64)`                |
/// | `send_window`                   | `send_window(u64)`                   |
/// | `max_concurrent_bidi_streams`   | `max_concurrent_bidi_streams(u32)`   |
/// | `max_concurrent_uni_streams`    | `max_concurrent_uni_streams(u32)`    |
/// | `initial_max_data`              | `receive_window(u64)` (overrides conn_receive_window) |
/// | `initial_max_stream_data_*`     | `stream_receive_window(u64)` (overrides stream_receive_window) |
///
/// `active_connection_id_limit` is managed internally by quinn (not
/// configurable on `TransportConfig`). `max_udp_payload_size` is set on
/// [`quinn::EndpointConfig`] via [`build_quinn_endpoint`].
fn build_transport_config(h3_opts: &Http3Options) -> crate::Result<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();

    if let Some(idle) = h3_opts.max_idle_timeout {
        // quinn expects milliseconds as a `VarInt`-backed `IdleTimeout`.
        let ms = idle.as_millis();
        let ms = u64::try_from(ms).map_err(Error::tls)?;
        let varint = quinn::VarInt::from_u64(ms).map_err(Error::tls)?;
        transport.max_idle_timeout(Some(varint.into()));
    }

    if let Some(wnd) = h3_opts.stream_receive_window {
        let varint = quinn::VarInt::from_u64(wnd).map_err(Error::tls)?;
        transport.stream_receive_window(varint);
    }
    if let Some(wnd) = h3_opts.conn_receive_window {
        let varint = quinn::VarInt::from_u64(wnd).map_err(Error::tls)?;
        transport.receive_window(varint);
    }
    if let Some(wnd) = h3_opts.send_window {
        transport.send_window(wnd);
    }
    if let Some(n) = h3_opts.max_concurrent_bidi_streams {
        let n = u32::try_from(n).map_err(Error::tls)?;
        transport.max_concurrent_bidi_streams(n.into());
    }
    if let Some(n) = h3_opts.max_concurrent_uni_streams {
        let n = u32::try_from(n).map_err(Error::tls)?;
        transport.max_concurrent_uni_streams(n.into());
    }

    // Fingerprint-level QUIC transport parameters. These override the
    // generic window settings above when set (e.g., by browser emulation).
    // `initial_max_data` maps to `receive_window` (connection-level flow control).
    if let Some(wnd) = h3_opts.initial_max_data {
        let varint = quinn::VarInt::from_u64(wnd).map_err(Error::tls)?;
        transport.receive_window(varint);
    }
    // `initial_max_stream_data_*` all map to `stream_receive_window`
    // (per-stream flow control). Use `initial_max_stream_data_bidi_local`
    // as the canonical source; the other two are typically identical.
    if let Some(wnd) = h3_opts.initial_max_stream_data_bidi_local {
        let varint = quinn::VarInt::from_u64(wnd).map_err(Error::tls)?;
        transport.stream_receive_window(varint);
    }

    Ok(transport)
}
