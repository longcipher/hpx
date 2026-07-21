//! RFC 7838 Alternative Services (`Alt-Svc`) header parser.
//!
//! Parses HTTP `Alt-Svc` response headers into [`AltSvc`] entries that
//! describe alternative ways to reach the origin (e.g., HTTP/3 endpoints).
//!
//! # Format (RFC 7838 §3)
//!
//! ```text
//! Alt-Svc = clear / 1#alt-value
//! alt-value = protocol-id "=" alt-authority *( OWS ";" OWS parameter )
//! protocol-id = token
//! alt-authority = quoted-string
//! parameter = token "=" ( token / quoted-string )
//! clear = "clear"
//! ```
//!
//! The `alt-authority` contains a `host:port` pair, or just `:port` (meaning
//! same host as the origin). If the port is omitted, 443 is assumed.

use http::HeaderValue;

/// A single alternative service entry parsed from an `Alt-Svc` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AltSvc {
    /// Protocol identifier (e.g., `"h3"`, `"h2"`).
    pub protocol: String,
    /// Hostname of the alternative service (empty string = same host as origin).
    pub host: String,
    /// Port number of the alternative service.
    pub port: u16,
    /// Maximum age in seconds (defaults to 86400 = 24 hours).
    pub max_age: u32,
    /// Whether the entry persists across network changes.
    pub persist: bool,
}

/// Parse an `Alt-Svc` header value.
///
/// Returns `None` if the header value cannot be parsed (no valid entries).
/// Returns `Some(vec![])` if the value is `"clear"` (case-insensitive),
/// signalling that all alternative services should be invalidated.
/// Returns `Some(vec![...])` with one or more [`AltSvc`] entries on success.
pub(crate) fn parse_alt_svc(header: &HeaderValue) -> Option<Vec<AltSvc>> {
    let value = header.to_str().ok()?;
    let value = value.trim();

    // "clear" directive (case-insensitive) — invalidate all cached entries.
    if value.eq_ignore_ascii_case("clear") {
        return Some(Vec::new());
    }

    let mut entries = Vec::new();
    for part in split_entries(value) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(entry) = parse_alt_value(part) {
            entries.push(entry);
        }
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

// ---------------------------------------------------------------------------
// Alt-Svc cache
// ---------------------------------------------------------------------------

/// An entry in the Alt-Svc cache with insertion time for TTL tracking.
#[derive(Debug, Clone)]
struct AltSvcEntry {
    alt_svc: AltSvc,
    inserted_at: std::time::Instant,
}

/// A per-authority cache for `Alt-Svc` entries.
///
/// The key is a `(host, port)` tuple representing the authority.
/// Entries are evicted based on their `max_age` when retrieved.
pub(crate) struct AltSvcCache {
    entries: tokio::sync::RwLock<std::collections::HashMap<(String, u16), Vec<AltSvcEntry>>>,
}

impl AltSvcCache {
    /// Create an empty cache.
    pub(crate) fn new() -> Self {
        Self {
            entries: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Insert entries for an authority.
    ///
    /// If `entries` is empty (clear signal), remove all entries for that
    /// authority.
    pub(crate) async fn insert(&self, authority: (String, u16), entries: Vec<AltSvc>) {
        if entries.is_empty() {
            self.entries.write().await.remove(&authority);
            return;
        }
        let now = std::time::Instant::now();
        let alt_svc_entries: Vec<AltSvcEntry> = entries
            .into_iter()
            .map(|alt_svc| AltSvcEntry {
                alt_svc,
                inserted_at: now,
            })
            .collect();
        self.entries
            .write()
            .await
            .insert(authority, alt_svc_entries);
    }

    /// Returns non-expired entries for an authority.
    ///
    /// Entries where `inserted_at + Duration::from_secs(max_age) < now` are
    /// expired and filtered out. If all entries have expired, returns `None`.
    pub(crate) async fn get(&self, authority: &(String, u16)) -> Option<Vec<AltSvc>> {
        let guard = self.entries.read().await;
        let cached = guard.get(authority)?;
        let now = std::time::Instant::now();
        let alive: Vec<AltSvc> = cached
            .iter()
            .filter(|e| {
                let max_age = std::time::Duration::from_secs(u64::from(e.alt_svc.max_age));
                e.inserted_at + max_age >= now
            })
            .map(|e| e.alt_svc.clone())
            .collect();
        if alive.is_empty() { None } else { Some(alive) }
    }

    /// Remove all entries for an authority.
    #[allow(dead_code)] // HTTP/3 alt-svc invalidation API; reserved for future use.
    pub(crate) async fn invalidate(&self, authority: &(String, u16)) {
        self.entries.write().await.remove(authority);
    }
}

// ---------------------------------------------------------------------------
// H3 failure tracker (circuit breaker)
// ---------------------------------------------------------------------------

/// Tracks QUIC handshake failures per-authority to implement a circuit
/// breaker pattern for HTTP/3 (`h3`) connections.
///
/// When a QUIC connection to an authority `(host, port)` fails (e.g.,
/// handshake timeout, unreachable port), the failure is recorded with a
/// cooldown period. Subsequent requests to the same authority skip h3
/// and fall back to h2/h1 until the cooldown expires.
///
/// The cooldown defaults to 60 seconds. Failed authorities are
/// automatically cleared from the tracker after the cooldown expires,
/// allowing h3 retries on the next request.
///
/// # Thread safety
///
/// `H3FailureTracker` is wrapped in `Arc` and shared across `HttpClient`
/// clones. The internal map uses `tokio::sync::RwLock` for concurrent
/// read/write access.
pub(crate) struct H3FailureTracker {
    failures: tokio::sync::RwLock<std::collections::HashMap<(String, u16), std::time::Instant>>,
    cooldown: std::time::Duration,
}

impl H3FailureTracker {
    /// Create a new failure tracker with the given cooldown duration.
    pub(crate) fn new(cooldown: std::time::Duration) -> Self {
        Self {
            failures: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            cooldown,
        }
    }

    /// Returns `true` if the authority has a recent failure and is still
    /// within the cooldown period.
    pub(crate) async fn is_blocked(&self, authority: &(String, u16)) -> bool {
        let guard = self.failures.read().await;
        match guard.get(authority) {
            Some(failure_time) => {
                let elapsed = failure_time.elapsed();
                elapsed < self.cooldown
            }
            None => false,
        }
    }

    /// Records a failure for the given authority.
    ///
    /// The authority is blocked from h3 attempts for the duration of the
    /// cooldown period.
    pub(crate) async fn record_failure(&self, authority: (String, u16)) {
        self.failures
            .write()
            .await
            .insert(authority, std::time::Instant::now());
    }

    /// Clears the failure record for the given authority.
    ///
    /// Called when a successful h3 connection is established, resetting
    /// the circuit breaker.
    pub(crate) async fn clear(&self, authority: &(String, u16)) {
        self.failures.write().await.remove(authority);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Split a comma-separated list, respecting quoted strings (so commas inside
/// `"..."` are not treated as delimiters).
fn split_entries(input: &str) -> Vec<&str> {
    split_quoted(input, ',')
}

/// Split a semicolon-separated list, respecting quoted strings.
fn split_params(input: &str) -> Vec<&str> {
    split_quoted(input, ';')
}

/// Split `input` by `delimiter`, ignoring delimiters that appear inside
/// quoted strings (between `"` characters).
fn split_quoted(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;

    for (i, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '"' => in_quotes = !in_quotes,
            '\\' => escaped = true,
            c if c == delimiter && !in_quotes => {
                parts.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < input.len() {
        parts.push(&input[start..]);
    }
    parts
}

/// Parse a single `alt-value` entry:
/// `protocol-id "=" alt-authority *( ";" parameter )`
fn parse_alt_value(input: &str) -> Option<AltSvc> {
    let input = input.trim();

    // 1. protocol-id "="
    let eq_pos = input.find('=')?;
    let protocol = input[..eq_pos].trim();
    if !is_token(protocol) {
        return None;
    }

    let rest = input[eq_pos + 1..].trim();

    // 2. alt-authority (quoted string)
    let (authority, rest) = parse_quoted_string(rest)?;
    let (host, port) = parse_authority(&authority)?;

    // 3. Optional parameters
    let mut max_age: u32 = 86400;
    let mut persist = false;

    let rest = rest.trim();
    for param_str in split_params(rest) {
        let param_str = param_str.trim();
        if param_str.is_empty() {
            continue;
        }

        let param_eq = match param_str.find('=') {
            Some(p) => p,
            None => continue,
        };
        let name = param_str[..param_eq].trim();
        let value_part = param_str[param_eq + 1..].trim();

        let value = if value_part.starts_with('"') {
            match parse_quoted_string(value_part) {
                Some((v, _)) => v,
                None => continue,
            }
        } else if is_token(value_part) {
            value_part.to_string()
        } else {
            continue;
        };

        match name {
            "ma" => {
                if let Ok(v) = value.parse::<u32>() {
                    max_age = v;
                }
            }
            "persist" => {
                persist = value == "1";
            }
            _ => {
                // Unknown parameters are ignored per RFC 7838.
            }
        }
    }

    Some(AltSvc {
        protocol: protocol.to_string(),
        host,
        port,
        max_age,
        persist,
    })
}

/// Parse a quoted string, returning the unescaped content and the remainder
/// of the input after the closing quote.
///
/// Returns `None` if the input does not start with `"` or if the quoted
/// string is malformed (e.g., missing closing quote, invalid escape).
fn parse_quoted_string(input: &str) -> Option<(String, &str)> {
    let input = input.trim();
    let mut chars = input.char_indices();

    // Expect opening DQUOTE.
    match chars.next() {
        Some((_, '"')) => {}
        _ => return None,
    }

    let mut result = String::new();
    let mut escaped = false;

    for (pos, ch) in chars {
        if escaped {
            // quoted-pair: only DQUOTE and backslash are valid after escape.
            match ch {
                '"' | '\\' => result.push(ch),
                _ => return None,
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            // Closing DQUOTE — return content and remainder.
            let rest = &input[pos + ch.len_utf8()..];
            return Some((result, rest));
        } else {
            // qdtext: any visible ASCII character.
            result.push(ch);
        }
    }

    // Reached end of input without finding closing quote.
    None
}

/// Parse the `alt-authority` content (already unquoted) into a `(host, port)` pair.
///
/// Supported formats:
/// - `":443"` → empty host (same as origin), port 443
/// - `"example.com:443"` → host="example.com", port=443
/// - `"example.com"` → host="example.com", port=443 (default)
fn parse_authority(authority: &str) -> Option<(String, u16)> {
    if authority.is_empty() {
        return None;
    }

    // ":port" — same host, explicit port.
    if let Some(port_str) = authority.strip_prefix(':') {
        let port = port_str.parse::<u16>().ok()?;
        return Some((String::new(), port));
    }

    // "host:port" or "host"
    if let Some(colon_pos) = authority.rfind(':') {
        let host = &authority[..colon_pos];
        let port_str = &authority[colon_pos + 1..];
        let port = port_str.parse::<u16>().ok()?;
        if host.is_empty() {
            return None;
        }
        Some((host.to_string(), port))
    } else {
        // Host only, default port.
        Some((authority.to_string(), 443))
    }
}

/// Check whether `s` is a valid HTTP `token` per RFC 7230 §3.2.6.
///
/// A token is a non-empty sequence of characters from the `tchar` set:
/// `!#$%&'*+-.^_`|~` plus digits and letters.
fn is_token(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '!' | '#'
                    | '$'
                    | '%'
                    | '&'
                    | '\''
                    | '*'
                    | '+'
                    | '-'
                    | '.'
                    | '^'
                    | '_'
                    | '`'
                    | '|'
                    | '~'
            )
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;

    // Helper to create a HeaderValue from a static string.
    fn hv(s: &'static str) -> HeaderValue {
        HeaderValue::from_static(s)
    }

    #[test]
    fn alt_svc_parse_valid() {
        let entries = parse_alt_svc(&hv(r#"h3=":443""#)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].protocol, "h3");
        assert_eq!(entries[0].host, "");
        assert_eq!(entries[0].port, 443);
        assert_eq!(entries[0].max_age, 86400);
        assert!(!entries[0].persist);
    }

    #[test]
    fn alt_svc_parse_clear() {
        let entries = parse_alt_svc(&hv("clear")).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn alt_svc_parse_malformed() {
        assert!(parse_alt_svc(&hv("garbage")).is_none());
    }

    #[test]
    fn alt_svc_parse_multiple() {
        let entries = parse_alt_svc(&hv(r#"h3=":443", h2="alt.example.com:8443""#)).unwrap();
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].protocol, "h3");
        assert_eq!(entries[0].host, "");
        assert_eq!(entries[0].port, 443);

        assert_eq!(entries[1].protocol, "h2");
        assert_eq!(entries[1].host, "alt.example.com");
        assert_eq!(entries[1].port, 8443);
    }

    #[test]
    fn alt_svc_parse_with_ma() {
        let entries = parse_alt_svc(&hv(r#"h3=":443"; ma=3600"#)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].max_age, 3600);
    }

    #[test]
    fn alt_svc_parse_with_persist() {
        let entries = parse_alt_svc(&hv(r#"h3=":443"; persist=1"#)).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].persist);
    }

    #[test]
    fn alt_svc_parse_with_host() {
        let entries = parse_alt_svc(&hv(r#"h3="alt.example.com:443""#)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host, "alt.example.com");
        assert_eq!(entries[0].port, 443);
    }

    #[test]
    fn alt_svc_parse_case_insensitive_clear() {
        let entries = parse_alt_svc(&hv("CLEAR")).unwrap();
        assert!(entries.is_empty());
    }

    // ------------------------------------------------------------------
    // AltSvcCache tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn alt_svc_cache_insert_lookup() {
        let cache = AltSvcCache::new();
        let authority = ("example.com".to_string(), 443);
        let entries = vec![AltSvc {
            protocol: "h3".to_string(),
            host: String::new(),
            port: 443,
            max_age: 86400,
            persist: false,
        }];
        cache.insert(authority.clone(), entries).await;
        let result = cache.get(&authority).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].protocol, "h3");
        assert_eq!(result[0].host, "");
        assert_eq!(result[0].port, 443);
    }

    #[tokio::test]
    async fn alt_svc_cache_expiry() {
        let cache = AltSvcCache::new();
        let authority = ("example.com".to_string(), 443);
        let entries = vec![AltSvc {
            protocol: "h3".to_string(),
            host: String::new(),
            port: 443,
            max_age: 0,
            persist: false,
        }];
        cache.insert(authority.clone(), entries).await;
        let result = cache.get(&authority).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn alt_svc_cache_invalidation() {
        let cache = AltSvcCache::new();
        let authority = ("example.com".to_string(), 443);
        let entries = vec![AltSvc {
            protocol: "h3".to_string(),
            host: String::new(),
            port: 443,
            max_age: 86400,
            persist: false,
        }];
        cache.insert(authority.clone(), entries).await;
        assert!(cache.get(&authority).await.is_some());
        cache.invalidate(&authority).await;
        assert!(cache.get(&authority).await.is_none());
    }

    // ------------------------------------------------------------------
    // H3FailureTracker tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn h3_failure_tracker_is_blocked_after_failure() {
        let tracker = H3FailureTracker::new(std::time::Duration::from_secs(60));
        let authority = ("example.com".to_string(), 443);
        // Initially not blocked.
        assert!(!tracker.is_blocked(&authority).await);
        // Record a failure.
        tracker.record_failure(authority.clone()).await;
        // Now blocked.
        assert!(tracker.is_blocked(&authority).await);
    }

    #[tokio::test]
    async fn h3_failure_tracker_clear() {
        let tracker = H3FailureTracker::new(std::time::Duration::from_secs(60));
        let authority = ("example.com".to_string(), 443);
        tracker.record_failure(authority.clone()).await;
        assert!(tracker.is_blocked(&authority).await);
        tracker.clear(&authority).await;
        assert!(!tracker.is_blocked(&authority).await);
    }

    #[tokio::test]
    async fn h3_failure_tracker_cooldown_expires() {
        let tracker = H3FailureTracker::new(std::time::Duration::from_millis(100));
        let authority = ("example.com".to_string(), 443);
        tracker.record_failure(authority.clone()).await;
        assert!(tracker.is_blocked(&authority).await);
        // Wait for cooldown to expire.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!tracker.is_blocked(&authority).await);
    }

    #[tokio::test]
    async fn h3_failure_tracker_different_authorities() {
        let tracker = H3FailureTracker::new(std::time::Duration::from_secs(60));
        let a1 = ("example.com".to_string(), 443);
        let a2 = ("example.com".to_string(), 8443);
        tracker.record_failure(a1.clone()).await;
        assert!(tracker.is_blocked(&a1).await);
        assert!(!tracker.is_blocked(&a2).await);
    }

    #[tokio::test]
    async fn alt_svc_cache_clear_signal() {
        let cache = AltSvcCache::new();
        let authority = ("example.com".to_string(), 443);
        let entries = vec![AltSvc {
            protocol: "h3".to_string(),
            host: String::new(),
            port: 443,
            max_age: 86400,
            persist: false,
        }];
        cache.insert(authority.clone(), entries).await;
        assert!(cache.get(&authority).await.is_some());
        // Insert empty vec (clear signal).
        cache.insert(authority.clone(), Vec::new()).await;
        assert!(cache.get(&authority).await.is_none());
    }
}
