//! Pure helpers for reading operating-system proxy configuration.
//!
//! The functions in this module never touch the operating system: they turn
//! the raw values found in the system configuration into the normalized
//! strings understood by the proxy matcher. Keeping them free of platform
//! APIs makes the parsing rules testable on every platform, not only on the
//! platform they come from.

use super::matcher::Builder;

/// The per-protocol proxy endpoints found in the Windows `ProxyServer`
/// registry value.
///
/// Windows stores the value either as a single `host:port` pair that applies
/// to every protocol, or as a `;`-separated list of `scheme=host:port`
/// entries.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct WindowsProxyServer {
    /// Endpoint used for plain HTTP requests.
    pub(super) http: Option<String>,
    /// Endpoint used for HTTPS requests.
    pub(super) https: Option<String>,
    /// Endpoint used for SOCKS proxying.
    pub(super) socks: Option<String>,
    /// Endpoint used for every protocol (the plain `host:port` form).
    pub(super) all: Option<String>,
}

/// Parse the Windows `ProxyServer` registry value.
///
/// Two shapes are supported:
///
/// * `proxy.example.com:8080` — a single endpoint for every protocol.
/// * `http=proxy:8080;https=proxy:8443;socks=proxy:1080` — one endpoint per
///   protocol. Unknown protocol keys are ignored.
pub(super) fn parse_windows_proxy_server(value: &str) -> WindowsProxyServer {
    let mut parsed = WindowsProxyServer::default();

    let trimmed = value.trim();
    if trimmed.is_empty() {
        return parsed;
    }

    if !trimmed.contains('=') {
        parsed.all = Some(trimmed.to_owned());
        return parsed;
    }

    for part in trimmed.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let Some((scheme, endpoint)) = part.split_once('=') else {
            continue;
        };

        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            continue;
        }

        match scheme.trim().to_ascii_lowercase().as_str() {
            "http" => parsed.http = Some(endpoint.to_owned()),
            "https" => parsed.https = Some(endpoint.to_owned()),
            "socks" => parsed.socks = Some(endpoint.to_owned()),
            _ => {}
        }
    }

    parsed
}

/// Apply a parsed Windows `ProxyServer` value to the matcher builder.
///
/// The single-endpoint form applies to both HTTP and HTTPS. The per-protocol
/// form maps `http=`/`https=` to the matching matcher slot and `socks=` to
/// the catch-all slot, which the matcher uses as the fallback for protocols
/// without a dedicated entry.
pub(super) fn apply_windows_proxy_server(builder: &mut Builder, value: &str) {
    let parsed = parse_windows_proxy_server(value);

    let all = parsed.all.as_deref().map(strip_endpoint_scheme);

    if let Some(http) = all {
        if builder.http.is_empty() {
            builder.http = super::normalize_endpoint(http);
        }
        if builder.https.is_empty() {
            builder.https = super::normalize_endpoint(http);
        }
    }

    if let Some(http) = parsed.http.as_deref().map(strip_endpoint_scheme)
        && builder.http.is_empty()
    {
        builder.http = super::normalize_endpoint(http);
    }

    if let Some(https) = parsed.https.as_deref().map(strip_endpoint_scheme)
        && builder.https.is_empty()
    {
        builder.https = super::normalize_endpoint(https);
    }

    if let Some(socks) = parsed.socks.as_deref().map(strip_endpoint_scheme)
        && builder.all.is_empty()
    {
        builder.all = socks_endpoint(socks);
    }
}

/// Parse the Windows `ProxyOverride` registry value into a `NO_PROXY` list.
///
/// Windows separates the entries with `;`. The `<local>` token (hostnames
/// without a dot) is preserved: the matcher understands it. A leading `*.`
/// wildcard is rewritten to the leading-dot domain form, which matches the
/// domain and all of its subdomains.
pub(super) fn parse_windows_proxy_override(value: &str) -> String {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| match entry.strip_prefix("*.") {
            Some(domain) => format!(".{domain}"),
            None => entry.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Build the matcher value for a SOCKS system proxy endpoint.
///
/// A missing port is left out so that the matcher can apply the SOCKS default
/// port of `1080`.
pub(super) fn socks_endpoint(host: &str) -> String {
    format!("socks5://{host}")
}

/// Apply a macOS system SOCKS proxy endpoint to the matcher builder.
///
/// macOS configures the SOCKS proxy independently of the HTTP and HTTPS web
/// proxies, and uses it for the protocols that have no dedicated proxy. The
/// matcher uses the catch-all slot the same way, so the SOCKS endpoint is
/// stored there and only applies to the protocols without their own entry.
pub(super) fn apply_macos_socks_proxy(builder: &mut Builder, endpoint: Option<&str>) {
    if let Some(endpoint) = endpoint
        && builder.all.is_empty()
    {
        builder.all = socks_endpoint(endpoint);
    }
}

/// Remove a leading `scheme://` from a proxy endpoint.
///
/// Windows stores bare `host:port` endpoints, but a user may have typed a
/// scheme into the registry value by hand.
fn strip_endpoint_scheme(endpoint: &str) -> &str {
    match endpoint.find("://") {
        Some(index) => &endpoint[index + 3..],
        None => endpoint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder() -> Builder {
        Builder::default()
    }

    #[test]
    fn windows_single_endpoint_applies_to_all_protocols() {
        let parsed = parse_windows_proxy_server("proxy.local:8080");
        assert_eq!(parsed.all.as_deref(), Some("proxy.local:8080"));
        assert_eq!(parsed.http, None);
        assert_eq!(parsed.https, None);
        assert_eq!(parsed.socks, None);
    }

    #[test]
    fn windows_per_protocol_endpoints_are_parsed() {
        let parsed = parse_windows_proxy_server(
            "http=plain.local:8080;https=secure.local:8443;socks=socks.local:1080",
        );
        assert_eq!(parsed.all, None);
        assert_eq!(parsed.http.as_deref(), Some("plain.local:8080"));
        assert_eq!(parsed.https.as_deref(), Some("secure.local:8443"));
        assert_eq!(parsed.socks.as_deref(), Some("socks.local:1080"));
    }

    #[test]
    fn windows_unknown_protocol_keys_are_ignored() {
        let parsed = parse_windows_proxy_server("ftp=ftp.local:21;http=plain.local:8080");
        assert_eq!(parsed.http.as_deref(), Some("plain.local:8080"));
        assert_eq!(parsed.https, None);
        assert_eq!(parsed.socks, None);
    }

    #[test]
    fn windows_empty_value_yields_no_proxy() {
        assert_eq!(
            parse_windows_proxy_server("  "),
            WindowsProxyServer::default()
        );
    }

    #[test]
    fn windows_single_endpoint_is_applied_to_http_and_https() {
        let mut b = builder();
        apply_windows_proxy_server(&mut b, "proxy.local:8080");

        assert_eq!(b.http, "http://proxy.local:8080");
        assert_eq!(b.https, "http://proxy.local:8080");
        assert_eq!(b.all, "");
    }

    #[test]
    fn windows_per_protocol_endpoints_are_applied() {
        let mut b = builder();
        apply_windows_proxy_server(
            &mut b,
            "http=plain.local:8080;https=secure.local:8443;socks=socks.local:1080",
        );

        assert_eq!(b.http, "http://plain.local:8080");
        assert_eq!(b.https, "http://secure.local:8443");
        assert_eq!(b.all, "socks5://socks.local:1080");
    }

    #[test]
    fn windows_socks_only_falls_back_to_the_catch_all_slot() {
        let mut b = builder();
        apply_windows_proxy_server(&mut b, "socks=socks.local:1080");

        assert_eq!(b.http, "");
        assert_eq!(b.https, "");
        assert_eq!(b.all, "socks5://socks.local:1080");
    }

    #[test]
    fn windows_endpoint_with_a_scheme_is_normalized() {
        let mut b = builder();
        apply_windows_proxy_server(&mut b, "http=http://proxy.local:8080");

        assert_eq!(b.http, "http://proxy.local:8080");
    }

    #[test]
    fn windows_existing_environment_values_win() {
        let mut b = builder();
        b.http = "http://from-env:1".to_owned();
        apply_windows_proxy_server(&mut b, "proxy.local:8080");

        assert_eq!(b.http, "http://from-env:1");
        assert_eq!(b.https, "http://proxy.local:8080");
    }

    #[test]
    fn windows_proxy_override_is_converted_to_a_no_proxy_list() {
        assert_eq!(
            parse_windows_proxy_override("localhost;127.0.0.1;*.example.com;<local>"),
            "localhost,127.0.0.1,.example.com,<local>"
        );
    }

    #[test]
    fn windows_proxy_override_drops_empty_entries() {
        assert_eq!(parse_windows_proxy_override("a;;b;"), "a,b");
    }

    #[test]
    fn socks_endpoint_uses_the_socks5_scheme() {
        assert_eq!(
            socks_endpoint("proxy.local:1080"),
            "socks5://proxy.local:1080"
        );
    }

    #[test]
    fn macos_socks_proxy_is_applied_to_the_catch_all_slot() {
        let mut b = builder();
        apply_macos_socks_proxy(&mut b, Some("socks.local:1080"));

        assert_eq!(b.all, "socks5://socks.local:1080");
        assert_eq!(b.http, "");
        assert_eq!(b.https, "");
    }

    #[test]
    fn macos_socks_proxy_does_not_override_an_existing_catch_all() {
        let mut b = builder();
        b.all = "http://from-env:1".to_owned();
        apply_macos_socks_proxy(&mut b, Some("socks.local:1080"));

        assert_eq!(b.all, "http://from-env:1");
    }

    #[test]
    fn macos_without_a_socks_proxy_leaves_the_builder_untouched() {
        let mut b = builder();
        apply_macos_socks_proxy(&mut b, None);

        assert_eq!(b.all, "");
    }
}
