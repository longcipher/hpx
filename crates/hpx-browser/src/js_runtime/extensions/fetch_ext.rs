//! JS `fetch()` backing ops.
//!
//! Requests are issued through the page's [`HttpClient`], built from the
//! shared [`FetchState::net_config`], so JS-initiated traffic honors the same
//! outbound proxy and SSRF policy as navigations and subresource loads.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use deno_core::{OpState, op2};

use crate::{
    js_runtime::state::{DomState, FetchState},
    net::{HttpClient, RedirectPolicy},
};

/// Error surfaced to JS as a rejected fetch promise.
#[derive(Debug, thiserror::Error, deno_error::JsError)]
#[class(generic)]
pub enum FetchError {
    #[error("invalid URL: {0}")]
    Url(String),
    #[error("fetch client unavailable: {0}")]
    Client(String),
    #[error("fetch failed: {0}")]
    Request(String),
}

/// Parse the bootstrap's JSON header array (`[["k","v"], …]`) into pairs.
fn parse_header_pairs(headers_json: &str) -> Vec<(String, String)> {
    let parsed: Result<Vec<(String, String)>, _> = serde_json::from_str(headers_json);
    parsed.unwrap_or_default()
}

/// Shape consumed by `js/fetch_bootstrap.js`.
fn result_json(
    status: u16,
    status_text: &str,
    url: &str,
    headers: Vec<(String, String)>,
    body: &str,
) -> String {
    serde_json::json!({
        "status": status,
        "statusText": status_text,
        "url": url,
        "headers": headers,
        "body": body,
    })
    .to_string()
}

/// Shared implementation for `op_fetch` / `op_net_fetch_sync`.
///
/// Resolves relative URLs against the document base URL, snapshots the shared
/// net config (no borrow held across the await), and follows up to 10
/// redirects like a browser fetch would.
async fn fetch_impl(
    op_state: Rc<RefCell<OpState>>,
    url: &str,
    method: &str,
    headers_json: &str,
    body: &str,
) -> Result<String, FetchError> {
    let resolved = {
        let state = op_state.borrow();
        let dom_state = state.borrow::<DomState>();
        match dom_state.base_url.as_ref() {
            Some(base) => base
                .join(url)
                .map_err(|e| FetchError::Url(format!("{url}: {e}")))?,
            None => url::Url::parse(url).map_err(|e| FetchError::Url(format!("{url}: {e}")))?,
        }
    };

    // Snapshot a cached HttpClient, rebuilding it only when the shared
    // NetConfig has been swapped since the last fetch (proxy change). This
    // preserves cookies, Accept-CH, and H1-only host memory across calls.
    let client = {
        let mut state = op_state.borrow_mut();
        let fetch_state = state.borrow_mut::<FetchState>();
        let config = &*fetch_state.net_config.load();
        if !Arc::ptr_eq(&fetch_state.last_config, config) {
            let client = HttpClient::from_net_config(config)
                .map_err(|e| FetchError::Client(e.to_string()))?;
            fetch_state.client = client;
            fetch_state.last_config = config.clone();
        }
        fetch_state.client.clone()
    };

    let method = if method.trim().is_empty() {
        "GET"
    } else {
        method
    };
    let headers = parse_header_pairs(headers_json);
    let body_bytes = (!body.is_empty()).then(|| body.as_bytes());

    let resp = client
        .request(
            method,
            resolved.as_str(),
            body_bytes,
            &headers,
            RedirectPolicy::Follow(10),
        )
        .await
        .map_err(|e| FetchError::Request(e.to_string()))?;

    let body = resp.text();
    Ok(result_json(
        resp.status,
        &resp.status_text,
        &resp.url,
        resp.headers.into_iter().collect(),
        &body,
    ))
}

/// JS `fetch(input, init)` — method, headers, and body come from the bootstrap.
#[op2]
#[string]
pub(crate) async fn op_fetch(
    op_state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[string] method: String,
    #[string] headers_json: String,
    #[string] body: String,
) -> Result<String, FetchError> {
    fetch_impl(op_state, &url, &method, &headers_json, &body).await
}

/// Legacy sync-style fetch shim (GET unless `method` says otherwise).
#[op2]
#[string]
pub(crate) async fn op_net_fetch_sync(
    op_state: Rc<RefCell<OpState>>,
    #[string] url: String,
    #[string] method: String,
) -> Result<String, FetchError> {
    fetch_impl(op_state, &url, &method, "[]", "").await
}

deno_core::extension!(fetch_extension, ops = [op_fetch, op_net_fetch_sync],);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_pairs_from_json() {
        let pairs = parse_header_pairs(r#"[["accept","*/*"],["x-test","1"]]"#);
        assert_eq!(
            pairs,
            vec![
                ("accept".to_string(), "*/*".to_string()),
                ("x-test".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn malformed_headers_json_yields_no_headers() {
        assert!(parse_header_pairs("not json").is_empty());
        assert!(parse_header_pairs("[]").is_empty());
    }

    #[test]
    fn result_json_matches_bootstrap_shape() {
        let json = result_json(200, "OK", "https://example.com/", vec![], "body");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], 200);
        assert_eq!(parsed["statusText"], "OK");
        assert_eq!(parsed["url"], "https://example.com/");
        assert_eq!(parsed["headers"], serde_json::json!([]));
        assert_eq!(parsed["body"], "body");
    }
}
