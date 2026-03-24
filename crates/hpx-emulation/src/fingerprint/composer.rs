//! Header composition with deduplication and priority.
//!
//! `HeaderComposer` builds HTTP header maps with proper deduplication:
//! custom headers take priority over fingerprint defaults.

use std::collections::HashSet;

use hpx::header::{HeaderMap, HeaderName, HeaderValue};

/// Composes HTTP headers with priority-based deduplication.
///
/// Custom headers (higher priority) override fingerprint headers (lower priority)
/// when they share the same header name. Header names are compared case-insensitively.
///
/// # Example
///
/// ```ignore
/// let composer = HeaderComposer::new()
///     .with_fingerprint_headers(fp_headers)
///     .with_custom_headers(vec![("user-agent", "custom-ua")]);
///
/// let headers = composer.compose();
/// // The custom user-agent overrides the fingerprint one.
/// ```
pub struct HeaderComposer {
    fingerprint_headers: Vec<(String, String)>,
    custom_headers: Vec<(String, String)>,
}

impl HeaderComposer {
    /// Creates a new empty `HeaderComposer`.
    pub fn new() -> Self {
        Self {
            fingerprint_headers: Vec::new(),
            custom_headers: Vec::new(),
        }
    }

    /// Adds fingerprint-derived headers (lower priority).
    pub fn with_fingerprint_headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.fingerprint_headers = headers
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self
    }

    /// Adds custom headers (higher priority, override fingerprint headers).
    pub fn with_custom_headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let new_headers: Vec<(String, String)> = headers
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        // Prepend new headers so they have higher priority
        self.custom_headers = new_headers.into_iter().chain(self.custom_headers).collect();
        self
    }

    /// Composes the final `HeaderMap` with deduplication.
    ///
    /// Returns an error if any header name or value is invalid.
    pub fn compose(self) -> Result<HeaderMap, ComposeError> {
        let mut headers = HeaderMap::new();
        let mut seen: HashSet<String> = HashSet::new();

        // Custom headers first (higher priority)
        for (name, value) in self
            .custom_headers
            .iter()
            .chain(self.fingerprint_headers.iter())
        {
            let lower_name = name.to_lowercase();
            if seen.contains(&lower_name) {
                continue;
            }
            if value.is_empty() {
                continue;
            }
            seen.insert(lower_name);

            let header_name = name
                .parse::<HeaderName>()
                .map_err(|_| ComposeError::InvalidHeaderName(name.clone()))?;
            let header_value = value
                .parse::<HeaderValue>()
                .map_err(|_| ComposeError::InvalidHeaderValue(value.clone()))?;
            headers.insert(header_name, header_value);
        }

        Ok(headers)
    }
}

impl Default for HeaderComposer {
    fn default() -> Self {
        Self::new()
    }
}

/// Error type for header composition.
#[derive(Debug, Clone)]
pub enum ComposeError {
    /// Invalid header name.
    InvalidHeaderName(String),
    /// Invalid header value.
    InvalidHeaderValue(String),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComposeError::InvalidHeaderName(name) => write!(f, "Invalid header name: {name}"),
            ComposeError::InvalidHeaderValue(value) => {
                write!(f, "Invalid header value: {value}")
            }
        }
    }
}

impl std::error::Error for ComposeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_overrides_fingerprint() {
        let composer = HeaderComposer::new()
            .with_fingerprint_headers(vec![
                ("user-agent", "fingerprint-ua"),
                ("accept", "text/html"),
            ])
            .with_custom_headers(vec![("user-agent", "custom-ua")]);

        let headers = composer.compose().unwrap();
        assert_eq!(headers.get("user-agent").unwrap(), "custom-ua");
        assert_eq!(headers.get("accept").unwrap(), "text/html");
    }

    #[test]
    fn test_case_insensitive_dedup() {
        let composer = HeaderComposer::new()
            .with_fingerprint_headers(vec![("User-Agent", "fp-ua")])
            .with_custom_headers(vec![("user-agent", "custom-ua")]);

        let headers = composer.compose().unwrap();
        assert_eq!(headers.get("user-agent").unwrap(), "custom-ua");
    }

    #[test]
    fn test_empty_value_skipped() {
        let composer = HeaderComposer::new()
            .with_fingerprint_headers(vec![("x-empty", ""), ("x-valid", "value")]);

        let headers = composer.compose().unwrap();
        assert!(headers.get("x-empty").is_none());
        assert_eq!(headers.get("x-valid").unwrap(), "value");
    }
}
