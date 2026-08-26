//! Domain-value parsers shared by download frontends.
//!
//! These helpers own the *domain* semantics of download options (speed
//! limits, checksums, proxy URLs). CLI shells keep only thin adapters that
//! translate [`DownloadError`] into their own error types, so alternative
//! frontends (TUI, service API) reuse identical parsing without duplicating
//! rules.

use crate::{
    error::DownloadError,
    types::{ChecksumSpec, HashAlgorithm, ProxyConfig, ProxyKind},
};

/// Parse a human-readable speed string into bytes per second.
///
/// Supported formats:
/// - `"1024"` → raw bytes per second
/// - `"1KB/s"`, `"500KB/s"` → kilobytes per second
/// - `"1MB/s"`, `"2.5MB/s"` → megabytes per second
/// - `"1GB/s"` → gigabytes per second
///
/// # Errors
///
/// Returns [`DownloadError::InvalidConfiguration`] for malformed numbers.
pub fn parse_speed_limit(s: &str) -> Result<u64, DownloadError> {
    let s = s.trim();

    let parse_factor = |suffix: &str, factor: f64| -> Result<u64, DownloadError> {
        let n: f64 = s
            .strip_suffix(suffix)
            .ok_or_else(|| {
                DownloadError::InvalidConfiguration(format!("invalid speed limit '{s}'"))
            })?
            .trim()
            .parse()
            .map_err(|_| {
                DownloadError::InvalidConfiguration(format!("invalid speed limit '{s}'"))
            })?;
        Ok((n * factor) as u64)
    };

    if s.ends_with("GB/s") {
        return parse_factor("GB/s", 1_073_741_824.0);
    }
    if s.ends_with("MB/s") {
        return parse_factor("MB/s", 1_048_576.0);
    }
    if s.ends_with("KB/s") {
        return parse_factor("KB/s", 1_024.0);
    }

    s.parse::<u64>()
        .map_err(|_| DownloadError::InvalidConfiguration(format!("invalid speed limit '{s}'")))
}

/// Parse a checksum specification string in the format `"algorithm:hex_value"`.
///
/// Supported algorithms: `md5`, `sha1`, `sha256`. The algorithm name is
/// case-insensitive.
///
/// # Errors
///
/// Returns [`DownloadError::InvalidConfiguration`] for unknown algorithms or
/// malformed input.
pub fn parse_checksum(s: &str) -> Result<ChecksumSpec, DownloadError> {
    let invalid = |msg: String| DownloadError::InvalidConfiguration(msg);

    let s = s.trim();
    let (algo_str, expected) = s.split_once(':').ok_or_else(|| {
        invalid("invalid checksum format, expected 'algorithm:hex_value'".to_string())
    })?;

    if expected.is_empty() {
        return Err(invalid("checksum hash value must not be empty".to_string()));
    }

    let algorithm = match algo_str.to_lowercase().as_str() {
        "md5" => HashAlgorithm::Md5,
        "sha1" => HashAlgorithm::Sha1,
        "sha256" => HashAlgorithm::Sha256,
        other => {
            return Err(invalid(format!(
                "unknown hash algorithm '{other}', expected one of: md5, sha1, sha256"
            )));
        }
    };

    Ok(ChecksumSpec {
        algorithm,
        expected: expected.to_string(),
    })
}

/// Parse a proxy URL string into a [`ProxyConfig`].
///
/// Detects the protocol kind from the URL scheme:
/// - `"http://..."` → [`ProxyKind::Http`]
/// - `"https://..."` → [`ProxyKind::Https`]
/// - `"socks5://..."` → [`ProxyKind::Socks5`]
///
/// # Errors
///
/// Returns [`DownloadError::InvalidConfiguration`] for unrecognized schemes.
pub fn parse_proxy_config(url: &str) -> Result<ProxyConfig, DownloadError> {
    let kind = if url.starts_with("http://") {
        ProxyKind::Http
    } else if url.starts_with("https://") {
        ProxyKind::Https
    } else if url.starts_with("socks5://") {
        ProxyKind::Socks5
    } else {
        return Err(DownloadError::InvalidConfiguration(format!(
            "unknown proxy scheme in '{url}', expected one of: http://, https://, socks5://"
        )));
    };
    Ok(ProxyConfig {
        url: url.to_string(),
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_limit_raw_bytes() {
        assert_eq!(parse_speed_limit("1024").unwrap(), 1024);
        assert_eq!(parse_speed_limit("0").unwrap(), 0);
    }

    #[test]
    fn speed_limit_units() {
        assert_eq!(parse_speed_limit("1KB/s").unwrap(), 1_024);
        assert_eq!(parse_speed_limit("2.5MB/s").unwrap(), 2_621_440);
        assert_eq!(parse_speed_limit("1GB/s").unwrap(), 1_073_741_824);
    }

    #[test]
    fn speed_limit_rejects_garbage() {
        assert!(parse_speed_limit("abc").is_err());
        assert!(parse_speed_limit("12XB/s").is_err());
    }

    #[test]
    fn checksum_case_insensitive_algorithm() {
        let spec = parse_checksum("SHA256:deadbeef").unwrap();
        assert!(matches!(spec.algorithm, HashAlgorithm::Sha256));
        assert_eq!(spec.expected, "deadbeef");
    }

    #[test]
    fn checksum_rejects_missing_colon_and_unknown_algo() {
        assert!(parse_checksum("deadbeef").is_err());
        assert!(parse_checksum("crc32:deadbeef").is_err());
        assert!(parse_checksum("md5:").is_err());
    }

    #[test]
    fn proxy_kinds_by_scheme() {
        assert!(matches!(
            parse_proxy_config("http://p:8080").unwrap().kind,
            ProxyKind::Http
        ));
        assert!(matches!(
            parse_proxy_config("https://p:8443").unwrap().kind,
            ProxyKind::Https
        ));
        assert!(matches!(
            parse_proxy_config("socks5://p:1080").unwrap().kind,
            ProxyKind::Socks5
        ));
        assert!(parse_proxy_config("ftp://p").is_err());
    }
}
