//! WebSocket permessage-deflate extension (RFC 7692).
//!
//! This module implements per-message deflate compression for WebSocket
//! connections. It handles:
//! - Extension negotiation (parsing `Sec-WebSocket-Extensions` headers)
//! - Compression of outgoing messages
//! - Decompression of incoming messages
//! - Context takeover (shared zlib state across messages)

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress};

/// Negotiated parameters for the permessage-deflate extension.
///
/// These are parsed from the server's `Sec-WebSocket-Extensions` response
/// header after a successful handshake.
#[derive(Debug, Clone)]
pub struct WebSocketExtensions {
    /// Whether the client requested no context takeover.
    pub client_no_context_takeover: bool,
    /// Whether the server requested no context takeover.
    pub server_no_context_takeover: bool,
    /// The client's maximum LZ77 window bits (1–15).
    pub client_max_window_bits: Option<u8>,
    /// The server's maximum LZ77 window bits (1–15).
    pub server_max_window_bits: Option<u8>,
}

impl Default for WebSocketExtensions {
    fn default() -> Self {
        Self {
            client_no_context_takeover: false,
            server_no_context_takeover: false,
            client_max_window_bits: None,
            server_max_window_bits: None,
        }
    }
}

impl WebSocketExtensions {
    /// Returns `true` if permessage-deflate was negotiated.
    #[inline]
    pub fn has_deflate(&self) -> bool {
        // We consider deflate negotiated if we have any non-default parameters
        // OR if the extension was simply offered and accepted. The presence
        // of this struct at all means negotiation occurred.
        true
    }
}

/// Parse the `Sec-WebSocket-Extensions` response header value to extract
/// negotiated permessage-deflate parameters.
///
/// Returns `Ok(None)` if no permessage-deflate extension is present.
/// Returns `Ok(Some(extensions))` with the parsed parameters.
/// Returns `Err` if the header value is malformed.
///
/// # RFC 7692 Negotiation
///
/// The server responds with the extension parameters it accepts:
/// ```text
/// Sec-WebSocket-Extensions: permessage-deflate; client_no_context_takeover; server_max_window_bits=12
/// ```
pub fn negotiate_permessage_deflate(
    header_value: &str,
) -> Result<Option<WebSocketExtensions>, ParseError> {
    // Split by comma to handle multiple extensions, then find permessage-deflate
    for part in header_value.split(',') {
        let part = part.trim();
        // Check if this part starts with "permessage-deflate"
        let params = if let Some(stripped) = part.strip_prefix("permessage-deflate") {
            stripped.trim()
        } else {
            continue;
        };

        let mut extensions = WebSocketExtensions::default();

        if params.is_empty() {
            // Just "permessage-deflate" with no parameters
            return Ok(Some(extensions));
        }

        // Parse parameters separated by semicolons
        for param in params.split(';') {
            let param = param.trim();
            if param.is_empty() {
                continue;
            }

            if let Some(value) = param.strip_prefix("server_max_window_bits=") {
                let value = value.trim().trim_matches('"');
                let bits: u8 = value
                    .parse()
                    .map_err(|_| ParseError::InvalidParameter(param.to_string()))?;
                if bits < 8 || bits > 15 {
                    return Err(ParseError::InvalidParameter(format!(
                        "server_max_window_bits={bits} out of range (8–15)"
                    )));
                }
                extensions.server_max_window_bits = Some(bits);
            } else if let Some(value) = param.strip_prefix("client_max_window_bits=") {
                let value = value.trim().trim_matches('"');
                let bits: u8 = value
                    .parse()
                    .map_err(|_| ParseError::InvalidParameter(param.to_string()))?;
                if bits < 8 || bits > 15 {
                    return Err(ParseError::InvalidParameter(format!(
                        "client_max_window_bits={bits} out of range (8–15)"
                    )));
                }
                extensions.client_max_window_bits = Some(bits);
            } else if param == "client_no_context_takeover" {
                extensions.client_no_context_takeover = true;
            } else if param == "server_no_context_takeover" {
                extensions.server_no_context_takeover = true;
            }
            // Unknown parameters are ignored per RFC 7692 §5.1
        }

        return Ok(Some(extensions));
    }

    Ok(None)
}

/// Error type for extension negotiation parsing.
#[derive(Debug)]
pub enum ParseError {
    /// An extension parameter was malformed or invalid.
    InvalidParameter(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::InvalidParameter(param) => {
                write!(f, "invalid permessage-deflate parameter: {param}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// WebSocket deflate codec for per-message compression/decompression.
///
/// Uses raw deflate (no zlib header) for compatibility with RFC 7692.
/// Supports context takeover by reusing the compressor/decompressor state
/// across messages.
pub struct DeflateCodec {
    /// Compressor state (shared across messages if context takeover is enabled).
    compress: Compress,
    /// Decompressor state (shared across messages if context takeover is enabled).
    decompress: Decompress,
    /// Whether to reset the compressor after each message.
    client_no_context_takeover: bool,
    /// Whether to reset the decompressor after each message.
    server_no_context_takeover: bool,
    /// The window bits for the compressor.
    #[allow(dead_code)]
    client_max_window_bits: Option<u8>,
    /// The window bits for the decompressor.
    #[allow(dead_code)]
    server_max_window_bits: Option<u8>,
}

impl DeflateCodec {
    /// Create a new `DeflateCodec` with the given negotiated extensions.
    pub fn new(extensions: &WebSocketExtensions) -> Self {
        let compression = Compression::default();
        Self {
            compress: Compress::new(compression, false), // raw deflate, no zlib wrapper
            decompress: Decompress::new(false),          // raw deflate, no zlib wrapper
            client_no_context_takeover: extensions.client_no_context_takeover,
            server_no_context_takeover: extensions.server_no_context_takeover,
            client_max_window_bits: extensions.client_max_window_bits,
            server_max_window_bits: extensions.server_max_window_bits,
        }
    }

    /// Compress a payload for sending.
    ///
    /// Returns the compressed bytes. If `client_no_context_takeover` is set,
    /// the compressor state is reset after compression (by using `FlushFinish`
    /// and recreating the compressor).
    pub fn compress(&mut self, data: &[u8]) -> Result<Vec<u8>, DeflateError> {
        if data.is_empty() {
            // Empty payloads are not compressed (RFC 7692 §7.2.3.5)
            return Ok(Vec::new());
        }

        let output_bound = data.len() + 16 + data.len() / 6;
        let mut compressed = Vec::with_capacity(output_bound);

        // Feed all data with Sync flush to produce a sync marker
        let before_out = self.compress.total_out();
        compressed.resize(output_bound, 0);
        self.compress
            .compress(data, &mut compressed[..], FlushCompress::Sync)
            .map_err(|e| DeflateError::Compress(format!("{e}")))?;
        let bytes_written = (self.compress.total_out() - before_out) as usize;
        compressed.truncate(bytes_written);

        if self.client_no_context_takeover {
            self.compress = Compress::new(Compression::default(), false);
        }

        Ok(compressed)
    }

    /// Decompress a payload received from the server.
    ///
    /// Returns the decompressed bytes. If `server_no_context_takeover` is set,
    /// the decompressor state is reset after decompression.
    pub fn decompress(&mut self, data: &[u8]) -> Result<Vec<u8>, DeflateError> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        let output_bound = (data.len() * 8).max(1024);
        let mut decompressed = Vec::with_capacity(output_bound);

        let before_out = self.decompress.total_out();
        decompressed.resize(output_bound, 0);
        self.decompress
            .decompress(data, &mut decompressed[..], FlushDecompress::Sync)
            .map_err(|e| DeflateError::Decompress(format!("{e}")))?;
        let bytes_written = (self.decompress.total_out() - before_out) as usize;
        decompressed.truncate(bytes_written);

        if self.server_no_context_takeover {
            self.decompress = Decompress::new(false);
        }

        Ok(decompressed)
    }
}

/// Error type for deflate operations.
#[derive(Debug)]
pub enum DeflateError {
    /// Compression error.
    Compress(String),
    /// Decompression error.
    Decompress(String),
}

impl std::fmt::Display for DeflateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeflateError::Compress(msg) => write!(f, "deflate compression error: {msg}"),
            DeflateError::Decompress(msg) => write!(f, "deflate decompression error: {msg}"),
        }
    }
}

impl std::error::Error for DeflateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_header() {
        let result = negotiate_permessage_deflate("").ok();
        assert!(result.flatten().is_none());
    }

    #[test]
    fn test_parse_basic_permessage_deflate() {
        let result = negotiate_permessage_deflate("permessage-deflate").ok();
        let ext = result.flatten();
        assert!(ext.is_some());
        let ext = ext.unwrap();
        assert!(!ext.client_no_context_takeover);
        assert!(!ext.server_no_context_takeover);
        assert!(ext.client_max_window_bits.is_none());
        assert!(ext.server_max_window_bits.is_none());
    }

    #[test]
    fn test_parse_client_no_context_takeover() {
        let result =
            negotiate_permessage_deflate("permessage-deflate; client_no_context_takeover").ok();
        let ext = result.flatten();
        assert!(ext.is_some());
        let ext = ext.unwrap();
        assert!(ext.client_no_context_takeover);
        assert!(!ext.server_no_context_takeover);
    }

    #[test]
    fn test_parse_server_no_context_takeover() {
        let result =
            negotiate_permessage_deflate("permessage-deflate; server_no_context_takeover").ok();
        let ext = result.flatten();
        assert!(ext.is_some());
        let ext = ext.unwrap();
        assert!(!ext.client_no_context_takeover);
        assert!(ext.server_no_context_takeover);
    }

    #[test]
    fn test_parse_server_max_window_bits() {
        let result =
            negotiate_permessage_deflate("permessage-deflate; server_max_window_bits=12").ok();
        let ext = result.flatten();
        assert!(ext.is_some());
        let ext = ext.unwrap();
        assert_eq!(ext.server_max_window_bits, Some(12));
    }

    #[test]
    fn test_parse_client_max_window_bits() {
        let result =
            negotiate_permessage_deflate("permessage-deflate; client_max_window_bits=10").ok();
        let ext = result.flatten();
        assert!(ext.is_some());
        let ext = ext.unwrap();
        assert_eq!(ext.client_max_window_bits, Some(10));
    }

    #[test]
    fn test_parse_multiple_parameters() {
        let result = negotiate_permessage_deflate(
            "permessage-deflate; client_no_context_takeover; server_max_window_bits=12",
        )
        .ok();
        let ext = result.flatten();
        assert!(ext.is_some());
        let ext = ext.unwrap();
        assert!(ext.client_no_context_takeover);
        assert!(!ext.server_no_context_takeover);
        assert_eq!(ext.server_max_window_bits, Some(12));
    }

    #[test]
    fn test_parse_non_deflate_extension_ignored() {
        let result = negotiate_permessage_deflate("some-other-extension").ok();
        let ext = result.flatten();
        assert!(ext.is_none());
    }

    #[test]
    fn test_parse_deflate_with_other_extensions() {
        let result = negotiate_permessage_deflate(
            "some-other-extension, permessage-deflate; client_no_context_takeover",
        )
        .ok();
        let ext = result.flatten();
        assert!(ext.is_some());
        let ext = ext.unwrap();
        assert!(ext.client_no_context_takeover);
    }

    #[test]
    fn test_compress_decompress_roundtrip() {
        let extensions = WebSocketExtensions::default();
        let mut codec = DeflateCodec::new(&extensions);

        let payloads: Vec<Vec<u8>> = vec![
            b"Hello, World!".to_vec(),
            vec![],
            b"A longer message that should compress well. ".repeat(10),
            b"Short".to_vec(),
        ];

        for payload in &payloads {
            let compressed = codec.compress(payload).ok();
            assert!(compressed.is_some());
            let compressed = compressed.unwrap();

            if payload.is_empty() {
                assert!(compressed.is_empty());
            }

            let decompressed = codec.decompress(&compressed).ok();
            assert!(decompressed.is_some());
            assert_eq!(decompressed.unwrap(), *payload);
        }
    }

    #[test]
    fn test_compress_decompress_no_context_takeover() {
        let extensions = WebSocketExtensions {
            client_no_context_takeover: true,
            server_no_context_takeover: true,
            ..Default::default()
        };
        let mut codec = DeflateCodec::new(&extensions);

        let payload = b"Test message for no context takeover";
        let compressed = codec.compress(payload).ok();
        assert!(compressed.is_some());
        let compressed = compressed.unwrap();

        let decompressed = codec.decompress(&compressed).ok();
        assert!(decompressed.is_some());
        assert_eq!(decompressed.unwrap(), payload);
    }

    #[test]
    fn test_multi_message_roundtrip() {
        let extensions = WebSocketExtensions::default();
        let mut codec = DeflateCodec::new(&extensions);

        let messages = vec![
            b"First message".to_vec(),
            b"Second message with more content".to_vec(),
            b"Third".to_vec(),
        ];

        for msg in &messages {
            let compressed = codec.compress(msg).ok().unwrap();
            let decompressed = codec.decompress(&compressed).ok().unwrap();
            assert_eq!(decompressed, *msg);
        }
    }

    #[test]
    fn test_web_socket_extensions_default() {
        let ext = WebSocketExtensions::default();
        assert!(!ext.client_no_context_takeover);
        assert!(!ext.server_no_context_takeover);
        assert!(ext.client_max_window_bits.is_none());
        assert!(ext.server_max_window_bits.is_none());
        assert!(ext.has_deflate());
    }

    #[test]
    fn test_parse_server_max_window_bits_with_quotes() {
        let result =
            negotiate_permessage_deflate("permessage-deflate; server_max_window_bits=\"12\"").ok();
        let ext = result.flatten();
        assert!(ext.is_some());
        let ext = ext.unwrap();
        assert_eq!(ext.server_max_window_bits, Some(12));
    }
}
