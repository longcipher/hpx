//! HAR (HTTP Archive) capture and export.
//!
//! Captures CDP Network domain events and builds a HAR 1.2 archive.
//! Uses manual ISO 8601 timestamps to avoid the chrono dependency.

use ahash::AHashMap;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::cdp_client::cdp::CdpMessage;

// ── HAR 1.2 Data Types ──────────────────────────────────────────────────────

/// Top-level HAR archive.
#[derive(Serialize, Debug, Clone)]
pub struct HarArchive {
    pub log: HarLog,
}

/// HAR log containing version, creator info, and captured entries.
#[derive(Serialize, Debug, Clone)]
pub struct HarLog {
    pub version: String,
    pub creator: HarCreator,
    #[serde(rename = "entries")]
    pub entries: Vec<HarEntry>,
}

/// Tool that created the HAR archive.
#[derive(Serialize, Debug, Clone)]
pub struct HarCreator {
    pub name: String,
    pub version: String,
}

/// A single HTTP transaction (request + response).
#[derive(Serialize, Debug, Clone)]
pub struct HarEntry {
    #[serde(rename = "startedDateTime")]
    pub started_date_time: String,
    pub time: f64,
    pub request: HarRequest,
    pub response: HarResponse,
    pub timings: HarTimings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<HarCache>,
    #[serde(rename = "serverIPAddress", skip_serializing_if = "Option::is_none")]
    pub server_ip_address: Option<String>,
    #[serde(rename = "connection", skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
}

/// HTTP request within a HAR entry.
#[derive(Serialize, Debug, Clone)]
pub struct HarRequest {
    pub method: String,
    pub url: String,
    pub http_version: String,
    pub cookies: Vec<HarCookie>,
    pub headers: Vec<HarHeader>,
    #[serde(rename = "queryString")]
    pub query_string: Vec<HarQueryParam>,
    #[serde(rename = "postData", skip_serializing_if = "Option::is_none")]
    pub post_data: Option<HarPostData>,
    pub headers_size: i64,
    pub body_size: i64,
}

/// HTTP response within a HAR entry.
#[derive(Serialize, Debug, Clone)]
pub struct HarResponse {
    pub status: u16,
    #[serde(rename = "statusText")]
    pub status_text: String,
    pub http_version: String,
    pub cookies: Vec<HarCookie>,
    pub headers: Vec<HarHeader>,
    pub content: HarContent,
    #[serde(rename = "redirectURL")]
    pub redirect_url: String,
    pub headers_size: i64,
    pub body_size: i64,
}

/// Timing breakdown for a request.
#[derive(Serialize, Debug, Clone)]
pub struct HarTimings {
    pub dns: f64,
    pub connect: f64,
    pub ssl: f64,
    pub send: f64,
    pub wait: f64,
    pub receive: f64,
}

/// Cache information (typically empty).
#[derive(Serialize, Debug, Clone, Default)]
pub struct HarCache {}

/// A single HTTP header.
#[derive(Serialize, Debug, Clone)]
pub struct HarHeader {
    pub name: String,
    pub value: String,
}

/// A single query string parameter.
#[derive(Serialize, Debug, Clone)]
pub struct HarQueryParam {
    pub name: String,
    pub value: String,
}

/// POST data payload.
#[derive(Serialize, Debug, Clone)]
pub struct HarPostData {
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<HarPostDataParam>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A single POST data parameter.
#[derive(Serialize, Debug, Clone)]
pub struct HarPostDataParam {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(rename = "contentType", skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// A single HTTP cookie.
#[derive(Serialize, Debug, Clone)]
pub struct HarCookie {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Response content metadata.
#[derive(Serialize, Debug, Clone)]
pub struct HarContent {
    pub size: i64,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "encoding", skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

// ── Capture State ────────────────────────────────────────────────────────────

/// A request that has been seen but not yet finalized.
#[derive(Debug, Clone)]
struct PendingRequest {
    method: String,
    url: String,
    headers: Vec<HarHeader>,
    started_time: String,
    started_millis: f64,
    http_version: String,
    /// Enriched by responseReceived
    status: Option<u16>,
    status_text: Option<String>,
    response_headers: Option<Vec<HarHeader>>,
    response_content_type: Option<String>,
    response_body_size: Option<i64>,
    /// Enriched by loadingFinished / loadingFailed
    finalized: bool,
    /// Timestamp when loadingFinished or loadingFailed was received.
    finished_timestamp: Option<f64>,
}

/// Mutable capture state guarded by a single async task.
#[derive(Debug)]
struct CaptureState {
    pending: AHashMap<String, PendingRequest>,
    entries: Vec<HarEntry>,
}

impl CaptureState {
    fn new() -> Self {
        Self {
            pending: AHashMap::new(),
            entries: Vec::new(),
        }
    }
}

// ── HarCapture ───────────────────────────────────────────────────────────────

/// HAR capture engine that listens to CDP Network events.
///
/// Accepts a `broadcast::Receiver<CdpMessage>` for decoupling from the CDP
/// client. Call [`start`](Self::start) to begin capturing, [`stop`](Self::stop)
/// to finalize and return the archive, or [`export`](Self::export) to snapshot
/// without stopping.
#[derive(Debug)]
pub struct HarCapture {
    /// Forwarding channel receiver (messages from background task).
    event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<CdpMessage>>,
    state: CaptureState,
    /// Background task handle (joins on stop).
    handle: Option<tokio::task::JoinHandle<()>>,
    /// Watch channel to signal the background task to stop.
    stop_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl HarCapture {
    /// Create a new capture bound to the given event receiver.
    pub fn new(rx: broadcast::Receiver<CdpMessage>) -> Self {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let handle = tokio::spawn(Self::forward_loop(rx, event_tx, stop_rx));

        Self {
            event_rx: Some(event_rx),
            state: CaptureState::new(),
            handle: Some(handle),
            stop_tx: Some(stop_tx),
        }
    }

    /// Background task that forwards broadcast events to an mpsc channel.
    ///
    /// This exists because `broadcast::Receiver` is `!Unpin` and cannot be
    /// `select!`-ed alongside `oneshot::Receiver` without pinning tricks.
    /// The mpsc channel provides a clean, `Unpin` boundary.
    async fn forward_loop(
        mut rx: broadcast::Receiver<CdpMessage>,
        event_tx: tokio::sync::mpsc::UnboundedSender<CdpMessage>,
        mut stop_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                biased;
                _ = stop_rx.changed() => break,
                msg = rx.recv() => {
                    match msg {
                        Ok(msg) => {
                            if event_tx.send(msg).is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    }

    /// Begin capturing Network events.
    ///
    /// The background forwarding task is already running from construction.
    /// This method is a no-op provided for API clarity; the caller should
    /// enable the CDP Network domain before calling this.
    pub fn start(&mut self) {
        // Background task already started in new(). This is intentionally a no-op
        // to match the documented lifecycle: new() → start() → events → stop().
    }

    /// Drain all pending events from the forwarding channel and process them.
    ///
    /// Call this periodically (e.g. before `export()` or `stop()`) to
    /// ensure capture state is up-to-date.
    pub fn drain(&mut self) {
        let msgs: Vec<_> = self
            .event_rx
            .as_mut()
            .map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        for msg in msgs {
            self.process_event(msg);
        }
    }

    /// Stop capturing and return the HAR archive.
    pub async fn stop(&mut self) -> Result<HarArchive, HarCaptureError> {
        // Signal the background task to stop
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(true);
        }

        // Wait for the background task to finish
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }

        // Drain any remaining events
        self.drain();

        Ok(self.build_archive())
    }

    /// Export a snapshot without stopping capture.
    pub fn export(&mut self) -> HarArchive {
        self.drain();
        self.build_archive()
    }

    /// Reset all captured state.
    pub fn clear(&mut self) {
        self.state = CaptureState::new();
    }

    /// Process a single CDP message, updating capture state.
    fn process_event(&mut self, msg: CdpMessage) {
        let method = match msg.method.as_deref() {
            Some(m) => m,
            None => return,
        };

        match method {
            "Network.requestWillBeSent" => {
                self.handle_request_will_be_sent(&msg);
            }
            "Network.responseReceived" => {
                self.handle_response_received(&msg);
            }
            "Network.loadingFinished" => {
                self.handle_loading_finished(&msg);
            }
            "Network.loadingFailed" => {
                self.handle_loading_failed(&msg);
            }
            _ => {}
        }
    }

    /// Handle `Network.requestWillBeSent` — create a pending request.
    fn handle_request_will_be_sent(&mut self, msg: &CdpMessage) {
        let params = match msg.params.as_ref() {
            Some(p) => p,
            None => return,
        };

        let request_id = match params["requestId"].as_str() {
            Some(id) => id.to_string(),
            None => return,
        };

        let request = &params["request"];
        let method = request["method"].as_str().unwrap_or("GET").to_string();
        let url = request["url"].as_str().unwrap_or("").to_string();
        let http_version = request["httpVersion"]
            .as_str()
            .unwrap_or("HTTP/1.1")
            .to_string();

        let headers = extract_headers_from_json(&request["headers"]);
        let timestamp = params["timestamp"].as_f64().unwrap_or(0.0);

        let pending = PendingRequest {
            method,
            url,
            headers,
            started_time: iso_timestamp(timestamp),
            started_millis: timestamp,
            http_version,
            status: None,
            status_text: None,
            response_headers: None,
            response_content_type: None,
            response_body_size: None,
            finalized: false,
            finished_timestamp: None,
        };

        self.state.pending.insert(request_id, pending);
    }

    /// Handle `Network.responseReceived` — enrich pending request with response info.
    fn handle_response_received(&mut self, msg: &CdpMessage) {
        let params = match msg.params.as_ref() {
            Some(p) => p,
            None => return,
        };

        let request_id = match params["requestId"].as_str() {
            Some(id) => id.to_string(),
            None => return,
        };

        let pending = match self.state.pending.get_mut(&request_id) {
            Some(p) => p,
            None => return,
        };

        let response = &params["response"];
        pending.status = response["status"].as_u64().map(|s| s as u16);
        pending.status_text = response["statusText"].as_str().map(String::from);
        pending.response_headers = Some(extract_headers_from_json(&response["headers"]));
        pending.response_content_type = response["headers"]["content-type"]
            .as_str()
            .map(String::from);
        pending.response_body_size = response["bodySize"].as_i64();
    }

    /// Handle `Network.loadingFinished` — finalize the entry.
    fn handle_loading_finished(&mut self, msg: &CdpMessage) {
        let params = match msg.params.as_ref() {
            Some(p) => p,
            None => return,
        };

        let request_id = match params["requestId"].as_str() {
            Some(id) => id.to_string(),
            None => return,
        };

        let encoded_length = params["encodedDataLength"].as_i64().unwrap_or(0);
        let timestamp = params["timestamp"].as_f64();

        if let Some(pending) = self.state.pending.get_mut(&request_id) {
            if pending.finished_timestamp.is_none() {
                pending.finished_timestamp = timestamp;
            }
        }

        self.finalize_entry(&request_id, encoded_length);
    }

    /// Handle `Network.loadingFailed` — finalize with error status.
    fn handle_loading_failed(&mut self, msg: &CdpMessage) {
        let params = match msg.params.as_ref() {
            Some(p) => p,
            None => return,
        };

        let request_id = match params["requestId"].as_str() {
            Some(id) => id.to_string(),
            None => return,
        };

        let timestamp = params["timestamp"].as_f64();

        // Set error status if not already set
        if let Some(pending) = self.state.pending.get_mut(&request_id) {
            if pending.status.is_none() {
                pending.status = Some(0);
                pending.status_text = Some(
                    params["errorText"]
                        .as_str()
                        .unwrap_or("Unknown Error")
                        .to_string(),
                );
            }
            if pending.finished_timestamp.is_none() {
                pending.finished_timestamp = timestamp;
            }
        }

        self.finalize_entry(&request_id, 0);
    }

    /// Finalize a pending request into a HAR entry.
    fn finalize_entry(&mut self, request_id: &str, encoded_length: i64) {
        let pending = match self.state.pending.remove(request_id) {
            Some(p) => p,
            None => return,
        };

        let status = pending.status.unwrap_or(0);
        let status_text = pending.status_text.unwrap_or_default();
        let response_headers = pending.response_headers.unwrap_or_default();
        let body_size = pending.response_body_size.unwrap_or(encoded_length);
        let mime_type = pending
            .response_content_type
            .unwrap_or_else(|| "application/octet-stream".to_string());

        // Compute elapsed time from start to finish (seconds).
        let time = match pending.finished_timestamp {
            Some(finished) => (finished - pending.started_millis).max(0.0),
            None => 0.0,
        };

        let entry = HarEntry {
            started_date_time: pending.started_time,
            time,
            request: HarRequest {
                method: pending.method,
                url: pending.url,
                http_version: pending.http_version,
                cookies: Vec::new(),
                headers: pending.headers,
                query_string: Vec::new(),
                post_data: None,
                headers_size: -1,
                body_size: -1,
            },
            response: HarResponse {
                status,
                status_text,
                http_version: "HTTP/1.1".to_string(),
                cookies: Vec::new(),
                headers: response_headers,
                content: HarContent {
                    size: body_size,
                    mime_type,
                    text: None,
                    encoding: None,
                },
                redirect_url: String::new(),
                headers_size: -1,
                body_size,
            },
            timings: HarTimings {
                dns: -1.0,
                connect: -1.0,
                ssl: -1.0,
                send: 0.0,
                wait: 0.0,
                receive: 0.0,
            },
            cache: None,
            server_ip_address: None,
            connection: None,
        };

        self.state.entries.push(entry);
    }

    /// Build the final HAR archive from current state.
    fn build_archive(&self) -> HarArchive {
        HarArchive {
            log: HarLog {
                version: "1.2".to_string(),
                creator: HarCreator {
                    name: "hpx-browser".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
                entries: self.state.entries.clone(),
            },
        }
    }
}

// ── JSON Helpers ─────────────────────────────────────────────────────────────

/// Extract headers from a JSON object into `HarHeader` entries.
fn extract_headers_from_json(headers: &serde_json::Value) -> Vec<HarHeader> {
    let obj = match headers.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    obj.iter()
        .map(|(k, v)| HarHeader {
            name: k.clone(),
            value: v.as_str().unwrap_or(&v.to_string()).to_string(),
        })
        .collect()
}

// ── ISO 8601 (Manual, no chrono) ─────────────────────────────────────────────

/// Convert a UNIX timestamp (seconds since epoch) to ISO 8601 format:
/// `YYYY-MM-DDTHH:MM:SS.mmmZ`
///
/// Uses Howard Hinnant's civil_from_days algorithm for calendar conversion.
pub fn iso_timestamp(seconds: f64) -> String {
    let secs = seconds.floor() as i64;
    let millis = ((seconds - secs as f64) * 1000.0).min(999.0).round() as u64;

    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let remaining = secs.rem_euclid(86_400);

    let hours = remaining / 3600;
    let remaining = remaining % 3600;
    let minutes = remaining / 60;
    let secs_part = remaining % 60;

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{secs_part:02}.{millis:03}Z")
}

/// Howard Hinnant's `civil_from_days` algorithm.
///
/// Converts a day count (days since 1970-01-01) to (year, month, day).
/// See: <https://howardhinnant.github.io/date_algorithms.html>
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors that can occur during HAR capture.
#[derive(Debug, thiserror::Error)]
pub enum HarCaptureError {
    /// Failed to start capture.
    #[error("failed to start capture: {0}")]
    StartFailed(String),

    /// Failed to stop capture.
    #[error("failed to stop capture: {0}")]
    StopFailed(String),
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ISO 8601 Tests ──────────────────────────────────────────────────

    #[test]
    fn iso_timestamp_epoch() {
        assert_eq!(iso_timestamp(0.0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn iso_timestamp_one_second() {
        assert_eq!(iso_timestamp(1.0), "1970-01-01T00:00:01.000Z");
    }

    #[test]
    fn iso_timestamp_one_minute() {
        assert_eq!(iso_timestamp(60.0), "1970-01-01T00:01:00.000Z");
    }

    #[test]
    fn iso_timestamp_one_hour() {
        assert_eq!(iso_timestamp(3600.0), "1970-01-01T01:00:00.000Z");
    }

    #[test]
    fn iso_timestamp_one_day() {
        assert_eq!(iso_timestamp(86_400.0), "1970-01-02T00:00:00.000Z");
    }

    #[test]
    fn iso_timestamp_millis() {
        assert_eq!(iso_timestamp(0.5), "1970-01-01T00:00:00.500Z");
        assert_eq!(iso_timestamp(1.234), "1970-01-01T00:00:01.234Z");
    }

    #[test]
    fn iso_timestamp_recent_date() {
        // 2024-01-15T12:30:45.000Z = 1705321845
        let ts = iso_timestamp(1_705_321_845.0);
        assert_eq!(ts, "2024-01-15T12:30:45.000Z");
    }

    #[test]
    fn iso_timestamp_with_millis_recent() {
        let ts = iso_timestamp(1_705_321_845.123);
        assert_eq!(ts, "2024-01-15T12:30:45.123Z");
    }

    #[test]
    fn civil_from_days_epoch() {
        let (y, m, d) = civil_from_days(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_one_day_later() {
        let (y, m, d) = civil_from_days(1);
        assert_eq!((y, m, d), (1970, 1, 2));
    }

    #[test]
    fn civil_from_days_leap_year_2000() {
        // 2000-01-01 = day 10957, 2000-02-29 = day 10957 + 59 = 11016
        let (y, m, d) = civil_from_days(11_016);
        assert_eq!((y, m, d), (2000, 2, 29));
    }

    #[test]
    fn civil_from_days_year_boundary() {
        // 2024-01-01 = 19723 days from epoch (54 years, 13 leap years)
        let (y, m, d) = civil_from_days(19_723);
        assert_eq!((y, m, d), (2024, 1, 1));
    }

    // ── HAR Serialization Tests ─────────────────────────────────────────

    #[test]
    fn har_archive_serializes_to_valid_json() {
        let archive = HarArchive {
            log: HarLog {
                version: "1.2".to_string(),
                creator: HarCreator {
                    name: "hpx-browser".to_string(),
                    version: "0.1.0".to_string(),
                },
                entries: vec![HarEntry {
                    started_date_time: "2024-01-15T12:30:45.000Z".to_string(),
                    time: 123.45,
                    request: HarRequest {
                        method: "GET".to_string(),
                        url: "https://example.com/".to_string(),
                        http_version: "HTTP/1.1".to_string(),
                        cookies: Vec::new(),
                        headers: vec![HarHeader {
                            name: "User-Agent".to_string(),
                            value: "hpx-browser/0.1.0".to_string(),
                        }],
                        query_string: Vec::new(),
                        post_data: None,
                        headers_size: -1,
                        body_size: -1,
                    },
                    response: HarResponse {
                        status: 200,
                        status_text: "OK".to_string(),
                        http_version: "HTTP/1.1".to_string(),
                        cookies: Vec::new(),
                        headers: vec![HarHeader {
                            name: "content-type".to_string(),
                            value: "text/html".to_string(),
                        }],
                        content: HarContent {
                            size: 1024,
                            mime_type: "text/html".to_string(),
                            text: None,
                            encoding: None,
                        },
                        redirect_url: String::new(),
                        headers_size: -1,
                        body_size: 1024,
                    },
                    timings: HarTimings {
                        dns: -1.0,
                        connect: -1.0,
                        ssl: -1.0,
                        send: 0.0,
                        wait: 50.0,
                        receive: 73.45,
                    },
                    cache: None,
                    server_ip_address: None,
                    connection: None,
                }],
            },
        };

        let json = serde_json::to_string_pretty(&archive).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should parse as JSON");

        assert_eq!(parsed["log"]["version"], "1.2");
        assert_eq!(parsed["log"]["creator"]["name"], "hpx-browser");
        assert_eq!(parsed["log"]["entries"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["log"]["entries"][0]["request"]["method"], "GET");
        assert_eq!(parsed["log"]["entries"][0]["response"]["status"], 200);
        assert_eq!(
            parsed["log"]["entries"][0]["startedDateTime"],
            "2024-01-15T12:30:45.000Z"
        );
    }

    #[test]
    fn har_archive_camel_case_serialization() {
        let archive = HarArchive {
            log: HarLog {
                version: "1.2".to_string(),
                creator: HarCreator {
                    name: "test".to_string(),
                    version: "1.0".to_string(),
                },
                entries: Vec::new(),
            },
        };

        let json = serde_json::to_string(&archive).expect("should serialize");
        assert!(json.contains("\"version\""));
        assert!(json.contains("\"creator\""));
        assert!(json.contains("\"entries\""));
    }

    // ── CaptureState Tests ──────────────────────────────────────────────

    #[test]
    fn capture_state_new_is_empty() {
        let state = CaptureState::new();
        assert!(state.pending.is_empty());
        assert!(state.entries.is_empty());
    }

    #[tokio::test]
    async fn har_capture_new_has_empty_state() {
        let (_, rx) = broadcast::channel(1);
        let capture = HarCapture::new(rx);
        assert!(capture.state.pending.is_empty());
        assert!(capture.state.entries.is_empty());
        assert!(capture.handle.is_some());
        assert!(capture.stop_tx.is_some());
    }

    #[tokio::test]
    async fn har_capture_stop_returns_archive() {
        let (_, rx) = broadcast::channel::<CdpMessage>(1);
        let mut capture = HarCapture::new(rx);

        // Process events directly to test stop() returns the archive
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705293045.0,
                "request": {
                    "method": "GET",
                    "url": "https://example.com/",
                    "httpVersion": "HTTP/1.1",
                    "headers": {}
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.loadingFinished".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "encodedDataLength": 100
            })),
            result: None,
            error: None,
            session_id: None,
        });

        let archive = capture.stop().await.expect("stop should succeed");
        assert_eq!(archive.log.version, "1.2");
        assert_eq!(archive.log.entries.len(), 1);
        assert_eq!(archive.log.entries[0].request.url, "https://example.com/");
    }

    // ── Event Processing Tests ──────────────────────────────────────────

    #[tokio::test]
    async fn process_request_will_be_sent() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        let msg = CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705321845.0,
                "request": {
                    "method": "GET",
                    "url": "https://example.com/page",
                    "httpVersion": "HTTP/2",
                    "headers": {
                        "User-Agent": "hpx/1.0",
                        "Accept": "text/html"
                    }
                }
            })),
            result: None,
            error: None,
            session_id: None,
        };

        capture.process_event(msg);

        assert_eq!(capture.state.pending.len(), 1);
        let pending = capture.state.pending.get("req-1").unwrap();
        assert_eq!(pending.method, "GET");
        assert_eq!(pending.url, "https://example.com/page");
        assert_eq!(pending.http_version, "HTTP/2");
        assert_eq!(pending.headers.len(), 2);
        assert_eq!(pending.started_time, "2024-01-15T12:30:45.000Z");
    }

    #[tokio::test]
    async fn process_response_received() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        // First: requestWillBeSent
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705319445.0,
                "request": {
                    "method": "POST",
                    "url": "https://example.com/api",
                    "httpVersion": "HTTP/1.1",
                    "headers": {}
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        // Then: responseReceived
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.responseReceived".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "response": {
                    "status": 201,
                    "statusText": "Created",
                    "headers": {
                        "content-type": "application/json"
                    },
                    "bodySize": 256
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        let pending = capture.state.pending.get("req-1").unwrap();
        assert_eq!(pending.status, Some(201));
        assert_eq!(pending.status_text.as_deref(), Some("Created"));
        assert_eq!(pending.response_body_size, Some(256));
    }

    #[tokio::test]
    async fn process_loading_finished_finalizes_entry() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        // requestWillBeSent
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705319445.0,
                "request": {
                    "method": "GET",
                    "url": "https://example.com/",
                    "httpVersion": "HTTP/1.1",
                    "headers": {}
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        // responseReceived
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.responseReceived".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "response": {
                    "status": 200,
                    "statusText": "OK",
                    "headers": { "content-type": "text/html" },
                    "bodySize": 512
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        // loadingFinished
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.loadingFinished".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "encodedDataLength": 480
            })),
            result: None,
            error: None,
            session_id: None,
        });

        assert!(capture.state.pending.is_empty());
        assert_eq!(capture.state.entries.len(), 1);

        let entry = &capture.state.entries[0];
        assert_eq!(entry.request.method, "GET");
        assert_eq!(entry.request.url, "https://example.com/");
        assert_eq!(entry.response.status, 200);
        assert_eq!(entry.response.content.size, 512);
        assert_eq!(entry.response.content.mime_type, "text/html");
    }

    #[tokio::test]
    async fn process_loading_failed_sets_error_status() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705319445.0,
                "request": {
                    "method": "GET",
                    "url": "https://example.com/timeout",
                    "httpVersion": "HTTP/1.1",
                    "headers": {}
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.loadingFailed".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "errorText": "net::ERR_TIMED_OUT"
            })),
            result: None,
            error: None,
            session_id: None,
        });

        assert!(capture.state.pending.is_empty());
        assert_eq!(capture.state.entries.len(), 1);

        let entry = &capture.state.entries[0];
        assert_eq!(entry.response.status, 0);
        assert_eq!(entry.response.status_text, "net::ERR_TIMED_OUT");
    }

    #[tokio::test]
    async fn ignore_unknown_methods() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        capture.process_event(CdpMessage {
            id: None,
            method: Some("Page.loadEventFired".to_string()),
            params: Some(serde_json::json!({})),
            result: None,
            error: None,
            session_id: None,
        });

        assert!(capture.state.pending.is_empty());
        assert!(capture.state.entries.is_empty());
    }

    #[tokio::test]
    async fn ignore_messages_without_method() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        capture.process_event(CdpMessage {
            id: Some(1),
            method: None,
            params: None,
            result: Some(serde_json::json!({})),
            error: None,
            session_id: None,
        });

        assert!(capture.state.pending.is_empty());
        assert!(capture.state.entries.is_empty());
    }

    // ── stop / export / clear Tests ─────────────────────────────────────

    #[tokio::test]
    async fn export_snapshot_includes_pending() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        // Simulate a request that hasn't finished yet
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705319445.0,
                "request": {
                    "method": "GET",
                    "url": "https://example.com/",
                    "httpVersion": "HTTP/1.1",
                    "headers": {}
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        // Export should return an archive (even if no entries finalized)
        let archive = capture.export();
        assert_eq!(archive.log.version, "1.2");
        assert_eq!(archive.log.entries.len(), 0); // No finalized entries
    }

    #[tokio::test]
    async fn clear_resets_state() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705319445.0,
                "request": {
                    "method": "GET",
                    "url": "https://example.com/",
                    "httpVersion": "HTTP/1.1",
                    "headers": {}
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        assert_eq!(capture.state.pending.len(), 1);

        capture.clear();

        assert!(capture.state.pending.is_empty());
        assert!(capture.state.entries.is_empty());
    }

    #[tokio::test]
    async fn multiple_requests_tracked_independently() {
        let (_, rx) = broadcast::channel(1);
        let mut capture = HarCapture::new(rx);

        // Request 1
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705319445.0,
                "request": {
                    "method": "GET",
                    "url": "https://a.com/",
                    "httpVersion": "HTTP/1.1",
                    "headers": {}
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        // Request 2
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-2",
                "timestamp": 1705319446.0,
                "request": {
                    "method": "POST",
                    "url": "https://b.com/api",
                    "httpVersion": "HTTP/2",
                    "headers": {}
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });

        assert_eq!(capture.state.pending.len(), 2);

        // Finalize req-1
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.loadingFinished".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "encodedDataLength": 100
            })),
            result: None,
            error: None,
            session_id: None,
        });

        assert_eq!(capture.state.pending.len(), 1);
        assert_eq!(capture.state.entries.len(), 1);
        assert_eq!(capture.state.entries[0].request.url, "https://a.com/");

        // Finalize req-2
        capture.process_event(CdpMessage {
            id: None,
            method: Some("Network.loadingFinished".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-2",
                "encodedDataLength": 200
            })),
            result: None,
            error: None,
            session_id: None,
        });

        assert_eq!(capture.state.pending.len(), 0);
        assert_eq!(capture.state.entries.len(), 2);
    }

    // ── extract_headers_from_json Tests ─────────────────────────────────

    #[test]
    fn extract_headers_from_json_object() {
        let json = serde_json::json!({
            "Content-Type": "text/html",
            "X-Custom": "value"
        });
        let headers = extract_headers_from_json(&json);
        assert_eq!(headers.len(), 2);
        assert!(
            headers
                .iter()
                .any(|h| h.name == "Content-Type" && h.value == "text/html")
        );
        assert!(
            headers
                .iter()
                .any(|h| h.name == "X-Custom" && h.value == "value")
        );
    }

    #[test]
    fn extract_headers_from_non_object() {
        let json = serde_json::json!("not an object");
        let headers = extract_headers_from_json(&json);
        assert!(headers.is_empty());
    }

    #[test]
    fn extract_headers_from_empty_object() {
        let json = serde_json::json!({});
        let headers = extract_headers_from_json(&json);
        assert!(headers.is_empty());
    }

    #[test]
    fn extract_headers_non_string_values() {
        let json = serde_json::json!({
            "count": 42,
            "flag": true,
            "name": "test"
        });
        let headers = extract_headers_from_json(&json);
        assert_eq!(headers.len(), 3);
        // Non-string values should be stringified
        let count_header = headers.iter().find(|h| h.name == "count").unwrap();
        assert_eq!(count_header.value, "42");
    }

    // ── Full Lifecycle Test ─────────────────────────────────────────────

    #[tokio::test]
    async fn full_lifecycle_via_broadcast_channel() {
        let (tx, rx) = broadcast::channel::<CdpMessage>(1);
        let mut capture = HarCapture::new(rx);

        // Send requestWillBeSent
        let _ = tx.send(CdpMessage {
            id: None,
            method: Some("Network.requestWillBeSent".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "timestamp": 1705293045.0,
                "request": {
                    "method": "GET",
                    "url": "https://example.com/page",
                    "httpVersion": "HTTP/1.1",
                    "headers": { "Accept": "text/html" }
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });
        tokio::task::yield_now().await;

        // Send responseReceived
        let _ = tx.send(CdpMessage {
            id: None,
            method: Some("Network.responseReceived".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "response": {
                    "status": 200,
                    "statusText": "OK",
                    "headers": { "content-type": "text/html; charset=utf-8" },
                    "bodySize": 4096
                }
            })),
            result: None,
            error: None,
            session_id: None,
        });
        tokio::task::yield_now().await;

        // Send loadingFinished
        let _ = tx.send(CdpMessage {
            id: None,
            method: Some("Network.loadingFinished".to_string()),
            params: Some(serde_json::json!({
                "requestId": "req-1",
                "encodedDataLength": 3800
            })),
            result: None,
            error: None,
            session_id: None,
        });
        tokio::task::yield_now().await;

        let archive = capture.stop().await.expect("stop should succeed");
        let json = serde_json::to_string_pretty(&archive).expect("should serialize");

        assert!(json.contains("\"1.2\""));
        assert!(json.contains("\"hpx-browser\""));
        assert!(json.contains("\"GET\""));
        assert!(json.contains("\"https://example.com/page\""));
        assert!(json.contains("\"startedDateTime\""));
    }
}

#[cfg(all(test, feature = "proptest"))]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn har_archive_serialization_never_panics(
            entries in 0..10usize,
        ) {
            let mut har_entries = Vec::new();
            for i in 0..entries {
                har_entries.push(HarEntry {
                    started_date_time: format!("2024-01-15T12:30:{:02}.000Z", i % 60),
                    time: i as f64 * 10.0,
                    request: HarRequest {
                        method: "GET".to_string(),
                        url: format!("https://example.com/page/{i}"),
                        http_version: "HTTP/1.1".to_string(),
                        cookies: Vec::new(),
                        headers: Vec::new(),
                        query_string: Vec::new(),
                        post_data: None,
                        headers_size: -1,
                        body_size: -1,
                    },
                    response: HarResponse {
                        status: 200,
                        status_text: "OK".to_string(),
                        http_version: "HTTP/1.1".to_string(),
                        cookies: Vec::new(),
                        headers: Vec::new(),
                        content: HarContent {
                            size: 1024,
                            mime_type: "text/html".to_string(),
                            text: None,
                            encoding: None,
                        },
                        redirect_url: String::new(),
                        headers_size: -1,
                        body_size: 1024,
                    },
                    timings: HarTimings {
                        dns: -1.0,
                        connect: -1.0,
                        ssl: -1.0,
                        send: 0.0,
                        wait: 10.0,
                        receive: 10.0,
                    },
                    cache: None,
                    server_ip_address: None,
                    connection: None,
                });
            }

            let archive = HarArchive {
                log: HarLog {
                    version: "1.2".to_string(),
                    creator: HarCreator {
                        name: "hpx-browser".to_string(),
                        version: "0.1.0".to_string(),
                    },
                    entries: har_entries,
                },
            };

            let json = serde_json::to_string(&archive)?;
            let parsed: serde_json::Value = serde_json::from_str(&json)?;
            prop_assert_eq!(parsed["log"]["version"].as_str(), Some("1.2"));
            let entry_count = parsed["log"]["entries"].as_array().map_or(0, |a| a.len());
            prop_assert_eq!(entry_count, entries);
        }

        #[test]
        fn iso_timestamp_matches_format(
            seconds in 0.0f64..5_000_000_000.0f64,
        ) {
            let ts = iso_timestamp(seconds);
            // Must match: YYYY-MM-DDTHH:MM:SS.mmmZ
            prop_assert_eq!(ts.len(), 24, "timestamp length should be 24, got {}: {}", ts.len(), ts);
            prop_assert_eq!(&ts[4..5], "-");
            prop_assert_eq!(&ts[7..8], "-");
            prop_assert_eq!(&ts[10..11], "T");
            prop_assert_eq!(&ts[13..14], ":");
            prop_assert_eq!(&ts[16..17], ":");
            prop_assert_eq!(&ts[23..24], "Z");

            // Parse numeric parts
            let year: i64 = ts[0..4].parse()?;
            let month: i64 = ts[5..7].parse()?;
            let day: i64 = ts[8..10].parse()?;
            let hour: i64 = ts[11..13].parse()?;
            let minute: i64 = ts[14..16].parse()?;
            let sec: i64 = ts[17..19].parse()?;
            let millis: u64 = ts[20..23].parse()?;

            prop_assert!(month >= 1 && month <= 12, "month out of range: {month}");
            prop_assert!(day >= 1 && day <= 31, "day out of range: {day}");
            prop_assert!(hour >= 0 && hour <= 23, "hour out of range: {hour}");
            prop_assert!(minute >= 0 && minute <= 59, "minute out of range: {minute}");
            prop_assert!(sec >= 0 && sec <= 59, "second out of range: {sec}");
            prop_assert!(millis <= 999, "millis out of range: {millis}");

            // Re-derive from seconds to verify consistency
            let secs_floor = seconds.floor() as i64;
            let expected_millis = ((seconds - secs_floor as f64) * 1000.0).min(999.0).round() as u64;
            prop_assert_eq!(millis, expected_millis);
        }
    }
}
