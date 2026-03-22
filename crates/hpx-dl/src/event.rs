//! Download events broadcast via tokio broadcast channels.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

// Re-export from canonical location.
pub use crate::types::DownloadEvent;
use crate::types::{DownloadId, DownloadState};

/// Error returned when emitting a download event.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    /// No active receivers for the broadcast channel.
    #[error("no active receivers")]
    NoReceivers,
    /// The channel has been closed (all receivers dropped).
    #[error("broadcast channel closed")]
    Closed,
}

impl From<tokio::sync::broadcast::error::SendError<DownloadEvent>> for EventError {
    fn from(_: tokio::sync::broadcast::error::SendError<DownloadEvent>) -> Self {
        Self::Closed
    }
}

/// Wrapper around `broadcast::Sender<DownloadEvent>` that provides a
/// convenient emit interface with graceful handling of empty receivers.
pub struct EventBroadcaster {
    sender: tokio::sync::broadcast::Sender<DownloadEvent>,
}

impl std::fmt::Debug for EventBroadcaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBroadcaster")
            .field("receiver_count", &self.receiver_count())
            .finish()
    }
}

impl EventBroadcaster {
    /// Create a new broadcaster with the given channel capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity);
        Self { sender }
    }

    /// Emit a download event to all active subscribers.
    ///
    /// If there are no receivers, a warning is logged but no error is returned.
    /// Returns the number of active receivers on success.
    pub fn emit(&self, event: DownloadEvent) -> Result<usize, EventError> {
        if self.sender.receiver_count() == 0 {
            tracing::warn!("emit called with no active receivers");
            return Ok(0);
        }
        let count = self.sender.send(event)?;
        Ok(count)
    }

    /// Subscribe to download events, returning a new receiver.
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<DownloadEvent> {
        self.sender.subscribe()
    }

    /// Return the number of active receivers.
    #[must_use]
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// Tracks download progress and emits events at configurable intervals.
pub struct ProgressTracker {
    download_id: DownloadId,
    total_bytes: Option<u64>,
    bytes_downloaded: Arc<AtomicU64>,
    broadcaster: Arc<EventBroadcaster>,
    last_emit: Arc<Mutex<Instant>>,
    emit_interval: Duration,
}

impl std::fmt::Debug for ProgressTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressTracker")
            .field("download_id", &self.download_id)
            .field("total_bytes", &self.total_bytes)
            .field(
                "bytes_downloaded",
                &self.bytes_downloaded.load(Ordering::Relaxed),
            )
            .field("emit_interval", &self.emit_interval)
            .finish_non_exhaustive()
    }
}

impl ProgressTracker {
    /// Create a new progress tracker.
    #[must_use]
    pub fn new(
        download_id: DownloadId,
        total_bytes: Option<u64>,
        broadcaster: Arc<EventBroadcaster>,
        emit_interval: Duration,
    ) -> Self {
        Self {
            download_id,
            total_bytes,
            bytes_downloaded: Arc::new(AtomicU64::new(0)),
            broadcaster,
            last_emit: Arc::new(Mutex::new(Instant::now())),
            emit_interval,
        }
    }

    /// Report bytes downloaded. Emits progress event if interval has elapsed.
    pub fn report_progress(&self, additional_bytes: u64) {
        let total = self
            .bytes_downloaded
            .fetch_add(additional_bytes, Ordering::Relaxed)
            + additional_bytes;
        let should_emit = {
            match self.last_emit.lock() {
                Ok(mut last) => {
                    let now = Instant::now();
                    if now.duration_since(*last) >= self.emit_interval {
                        *last = now;
                        true
                    } else {
                        false
                    }
                }
                Err(_) => false,
            }
        };
        if should_emit {
            let _ = self.broadcaster.emit(DownloadEvent::Progress {
                id: self.download_id,
                downloaded: total,
                total: self.total_bytes,
            });
        }
    }

    /// Force emit current progress (e.g., on completion).
    pub fn flush(&self) {
        let downloaded = self.bytes_downloaded.load(Ordering::Relaxed);
        let _ = self.broadcaster.emit(DownloadEvent::Progress {
            id: self.download_id,
            downloaded,
            total: self.total_bytes,
        });
        if let Ok(mut last) = self.last_emit.lock() {
            *last = Instant::now();
        }
    }

    /// Emit a state change event.
    pub fn emit_state_change(&self, state: DownloadState) {
        let _ = self.broadcaster.emit(DownloadEvent::StateChanged {
            id: self.download_id,
            state,
        });
    }

    /// Emit started event.
    pub fn emit_started(&self) {
        let _ = self.broadcaster.emit(DownloadEvent::Started {
            id: self.download_id,
        });
    }

    /// Emit completed event.
    pub fn emit_completed(&self) {
        let _ = self.broadcaster.emit(DownloadEvent::Completed {
            id: self.download_id,
        });
    }

    /// Emit failed event.
    pub fn emit_failed(&self, error: &str) {
        let _ = self.broadcaster.emit(DownloadEvent::Failed {
            id: self.download_id,
            error: error.to_string(),
        });
    }

    /// Get current bytes downloaded.
    #[must_use]
    pub fn bytes_downloaded(&self) -> u64 {
        self.bytes_downloaded.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DownloadId;

    #[tokio::test]
    async fn subscribe_and_emit() {
        let broadcaster = EventBroadcaster::new(16);
        let mut rx = broadcaster.subscribe();

        let id = DownloadId::new();
        let event = DownloadEvent::Started { id };

        broadcaster.emit(event.clone()).unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, event);
    }

    #[tokio::test]
    async fn emit_multiple_events() {
        let broadcaster = EventBroadcaster::new(16);
        let mut rx = broadcaster.subscribe();

        let id = DownloadId::new();
        let events = vec![
            DownloadEvent::Added { id },
            DownloadEvent::Started { id },
            DownloadEvent::Progress {
                id,
                downloaded: 500,
                total: Some(1000),
            },
            DownloadEvent::Completed { id },
        ];

        for event in &events {
            broadcaster.emit(event.clone()).unwrap();
        }

        for expected in &events {
            let received = rx.recv().await.unwrap();
            assert_eq!(&received, expected);
        }
    }

    #[tokio::test]
    async fn emit_no_receivers_warns_but_ok() {
        let broadcaster = EventBroadcaster::new(16);
        let id = DownloadId::new();
        let result = broadcaster.emit(DownloadEvent::Added { id });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_events() {
        let broadcaster = EventBroadcaster::new(16);
        let mut rx1 = broadcaster.subscribe();
        let mut rx2 = broadcaster.subscribe();

        assert_eq!(broadcaster.receiver_count(), 2);

        let id = DownloadId::new();
        let event = DownloadEvent::Completed { id };

        broadcaster.emit(event.clone()).unwrap();

        assert_eq!(rx1.recv().await.unwrap(), event);
        assert_eq!(rx2.recv().await.unwrap(), event);
    }

    #[tokio::test]
    async fn subscriber_drop_reduces_count() {
        let broadcaster = EventBroadcaster::new(16);
        let rx = broadcaster.subscribe();
        assert_eq!(broadcaster.receiver_count(), 1);
        drop(rx);
        assert_eq!(broadcaster.receiver_count(), 0);
    }

    #[tokio::test]
    async fn emit_after_all_receivers_dropped() {
        let broadcaster = EventBroadcaster::new(16);
        let rx = broadcaster.subscribe();
        drop(rx);

        let id = DownloadId::new();
        let result = broadcaster.emit(DownloadEvent::Added { id });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    // --- ProgressTracker tests ---

    #[tokio::test]
    async fn progress_tracker_report_progress_emits_after_interval() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        let tracker = ProgressTracker::new(id, Some(1000), broadcaster, Duration::from_millis(0));

        tracker.report_progress(100);

        let event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(event.is_ok(), "should receive progress event");
        let event = event.unwrap().unwrap();
        assert_eq!(
            event,
            DownloadEvent::Progress {
                id,
                downloaded: 100,
                total: Some(1000)
            }
        );
    }

    #[tokio::test]
    async fn progress_tracker_accumulates_bytes() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        let tracker = ProgressTracker::new(id, Some(1000), broadcaster, Duration::from_millis(0));

        tracker.report_progress(100);
        let _ = rx.recv().await; // consume first event

        tracker.report_progress(200);
        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            DownloadEvent::Progress {
                id,
                downloaded: 300,
                total: Some(1000)
            }
        );
    }

    #[tokio::test]
    async fn progress_tracker_bytes_downloaded_accessor() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let id = DownloadId::new();
        let tracker = ProgressTracker::new(id, None, broadcaster, Duration::from_millis(0));

        tracker.report_progress(50);
        tracker.report_progress(75);
        assert_eq!(tracker.bytes_downloaded(), 125);
    }

    #[tokio::test]
    async fn progress_tracker_flush_forces_emit() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        // Long interval so normal report_progress won't emit
        let tracker = ProgressTracker::new(id, Some(2000), broadcaster, Duration::from_secs(60));

        tracker.report_progress(42);
        // The first report_progress may or may not emit (interval from creation).
        // Flush should always emit.
        tracker.flush();

        // Drain until we get a flush event with downloaded=42
        let mut found = false;
        for _ in 0..5 {
            if let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await
            {
                let expected = DownloadEvent::Progress {
                    id,
                    downloaded: 42,
                    total: Some(2000),
                };
                if event == expected {
                    found = true;
                    break;
                }
            }
        }
        assert!(found, "flush should emit progress event with downloaded=42");
    }

    #[tokio::test]
    async fn progress_tracker_emit_state_change() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        let tracker = ProgressTracker::new(id, None, broadcaster, Duration::from_millis(0));

        tracker.emit_state_change(DownloadState::Downloading);

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            DownloadEvent::StateChanged {
                id,
                state: DownloadState::Downloading
            }
        );
    }

    #[tokio::test]
    async fn progress_tracker_emit_started() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        let tracker = ProgressTracker::new(id, None, broadcaster, Duration::from_millis(0));

        tracker.emit_started();

        let event = rx.recv().await.unwrap();
        assert_eq!(event, DownloadEvent::Started { id });
    }

    #[tokio::test]
    async fn progress_tracker_emit_completed() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        let tracker = ProgressTracker::new(id, Some(500), broadcaster, Duration::from_millis(0));

        tracker.report_progress(500);
        let _ = rx.recv().await;
        tracker.flush();
        let _ = rx.recv().await;
        tracker.emit_completed();

        let event = rx.recv().await.unwrap();
        assert_eq!(event, DownloadEvent::Completed { id });
    }

    #[tokio::test]
    async fn progress_tracker_emit_failed() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        let tracker = ProgressTracker::new(id, None, broadcaster, Duration::from_millis(0));

        tracker.emit_failed("connection reset");

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            DownloadEvent::Failed {
                id,
                error: "connection reset".to_string()
            }
        );
    }

    #[tokio::test]
    async fn progress_tracker_rapid_reports_batch() {
        let broadcaster = Arc::new(EventBroadcaster::new(64));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        // 100ms interval
        let tracker =
            ProgressTracker::new(id, Some(10_000), broadcaster, Duration::from_millis(100));

        // Rapidly report 10 times within the interval window
        for _ in 0..10 {
            tracker.report_progress(100);
        }

        // Should have 0 or 1 events (the first call since creation may trigger)
        let mut count = 0u32;
        while let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(20), rx.recv()).await {
            count += 1;
        }
        assert!(count <= 1, "rapid reports should batch, got {count} events");

        // After interval elapses, next report should emit
        tokio::time::sleep(Duration::from_millis(110)).await;
        tracker.report_progress(100);

        let event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(event.is_ok(), "should receive progress after interval");
        let event = event.unwrap().unwrap();
        assert_eq!(
            event,
            DownloadEvent::Progress {
                id,
                downloaded: 1100,
                total: Some(10_000)
            }
        );
    }

    #[tokio::test]
    async fn progress_tracker_flush_also_updates_last_emit() {
        let broadcaster = Arc::new(EventBroadcaster::new(16));
        let mut rx = broadcaster.subscribe();
        let id = DownloadId::new();
        let tracker = ProgressTracker::new(id, Some(1000), broadcaster, Duration::from_millis(0));

        tracker.report_progress(10);
        let _ = rx.recv().await;

        tracker.flush();
        let _ = rx.recv().await;

        // After flush, another report_progress should still respect the interval
        // With 0ms interval, it should emit immediately
        tracker.report_progress(20);
        let event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(event.is_ok());
    }
}
