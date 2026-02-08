//! Lock-free pending request management using `scc::HashMap`.
//!
//! This store tracks outgoing requests awaiting responses, with automatic
//! timeout cleanup and capacity management.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::oneshot;

use super::{config::WsConfig, types::RequestId};
use crate::error::{TransportError, TransportResult};

/// A pending request awaiting a response.
pub struct PendingRequest {
    /// Channel to send the response.
    pub response_tx: oneshot::Sender<TransportResult<String>>,
    /// When this request was created.
    pub created_at: Instant,
    /// Timeout for this specific request (overrides default).
    pub timeout: Duration,
}

/// Lock-free store for pending requests.
///
/// Uses `scc::HashMap` for wait-free reads and lock-free writes,
/// providing excellent performance under high concurrency.
pub struct PendingRequestStore {
    requests: scc::HashMap<RequestId, PendingRequest>,
    config: Arc<WsConfig>,
}

impl PendingRequestStore {
    /// Create a new pending request store.
    pub fn new(config: Arc<WsConfig>) -> Self {
        Self {
            requests: scc::HashMap::new(),
            config,
        }
    }

    /// Add a new pending request.
    ///
    /// Returns a receiver that will receive the response when it arrives.
    /// Returns `None` if capacity is exceeded.
    pub fn add(
        &self,
        id: RequestId,
        timeout: Option<Duration>,
    ) -> Option<oneshot::Receiver<TransportResult<String>>> {
        // Check capacity first
        if self.requests.len() >= self.config.max_pending_requests {
            return None;
        }

        let (tx, rx) = oneshot::channel();
        let timeout = timeout.unwrap_or(self.config.request_timeout);

        let pending = PendingRequest {
            response_tx: tx,
            created_at: Instant::now(),
            timeout,
        };

        // Insert returns Err if key already exists
        if self.requests.insert_sync(id, pending).is_err() {
            return None;
        }

        Some(rx)
    }

    /// Resolve a pending request with a response.
    ///
    /// Returns `true` if the request was found and resolved, `false` otherwise.
    pub fn resolve(&self, id: &RequestId, response: TransportResult<String>) -> bool {
        if let Some((_, pending)) = self.requests.remove_sync(id) {
            // Send the response; ignore error (receiver may have dropped)
            let _ = pending.response_tx.send(response);
            return true;
        }
        false
    }

    /// Remove a pending request without notifying the receiver.
    ///
    /// Returns `true` if the request was present, `false` otherwise.
    pub fn remove(&self, id: &RequestId) -> bool {
        self.requests.remove_sync(id).is_some()
    }

    /// Clean up stale (timed out) requests without notification.
    ///
    /// Use this for periodic cleanup during normal operation.
    pub fn cleanup_stale(&self) {
        let now = Instant::now();
        self.requests
            .retain_sync(|_, pending| now.duration_since(pending.created_at) < pending.timeout);
    }

    /// Clean up stale requests and notify them of timeout.
    ///
    /// This sends timeout errors to all expired requests.
    pub fn cleanup_stale_with_notify(&self) {
        let now = Instant::now();
        let mut expired = Vec::new();

        // First, collect expired IDs
        self.requests.retain_sync(|id, pending| {
            if now.duration_since(pending.created_at) >= pending.timeout {
                expired.push((id.clone(), pending.timeout));
            }
            true
        });

        // Then remove and notify each one
        for (id, timeout) in expired {
            if let Some((_, pending)) = self.requests.remove_sync(&id) {
                let _ = pending
                    .response_tx
                    .send(Err(TransportError::request_timeout(
                        timeout,
                        id.to_string(),
                    )));
            }
        }
    }

    /// Check if there's capacity for more requests.
    pub fn has_capacity(&self) -> bool {
        self.requests.len() < self.config.max_pending_requests
    }

    /// Get the current number of pending requests.
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    /// Check if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Clear all pending requests.
    ///
    /// This should be called on connection close to notify all waiters.
    pub fn clear_with_error(&self, error_message: &str) {
        let mut ids = Vec::new();
        self.requests.retain_sync(|id, _| {
            ids.push(id.clone());
            true
        });

        for id in ids {
            if let Some((_, pending)) = self.requests.remove_sync(&id) {
                let _ = pending
                    .response_tx
                    .send(Err(TransportError::connection_closed(Some(
                        error_message.to_string(),
                    ))));
            }
        }
    }

    /// Clear all pending requests without notification.
    pub fn clear(&self) {
        self.requests.clear_sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Arc<WsConfig> {
        Arc::new(WsConfig::new("wss://test.com").max_pending_requests(10))
    }

    #[test]
    fn test_add_and_resolve() {
        let store = PendingRequestStore::new(test_config());
        let id = RequestId::new();

        let rx = store.add(id.clone(), None);
        assert!(rx.is_some());
        assert_eq!(store.len(), 1);

        let resolved = store.resolve(&id, Ok("response".to_string()));
        assert!(resolved);
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_capacity_limit() {
        let store = PendingRequestStore::new(test_config());

        // Fill to capacity
        for _ in 0..10 {
            let rx = store.add(RequestId::new(), None);
            assert!(rx.is_some());
        }

        // Should fail at capacity
        let rx = store.add(RequestId::new(), None);
        assert!(rx.is_none());
        assert!(!store.has_capacity());
    }

    #[test]
    fn test_resolve_nonexistent() {
        let store = PendingRequestStore::new(test_config());
        let id = RequestId::new();

        let resolved = store.resolve(&id, Ok("response".to_string()));
        assert!(!resolved);
    }

    #[test]
    fn test_cleanup_stale() {
        let config =
            Arc::new(WsConfig::new("wss://test.com").request_timeout(Duration::from_millis(1)));
        let store = PendingRequestStore::new(config);

        let _rx = store.add(RequestId::new(), None);
        assert_eq!(store.len(), 1);

        // Wait for timeout
        std::thread::sleep(Duration::from_millis(10));
        store.cleanup_stale();

        assert_eq!(store.len(), 0);
    }
}
