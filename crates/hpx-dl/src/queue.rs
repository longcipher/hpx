//! Priority download queue and concurrency limiter.

use std::{cmp::Ordering, sync::Arc, time::Instant};

use tokio::sync::{Semaphore, SemaphorePermit};

use crate::{
    error::DownloadError,
    types::{DownloadId, DownloadPriority, DownloadRequest},
};

/// Entry in the priority queue.
#[derive(Debug, Clone)]
pub struct QueueEntry {
    /// Unique download identifier.
    pub id: DownloadId,
    /// Scheduling priority.
    pub priority: DownloadPriority,
    /// The download request.
    pub request: DownloadRequest,
    /// When this entry was inserted.
    pub inserted_at: Instant,
    /// Monotonic sequence number for stable FIFO within same priority.
    seq: u64,
}

impl QueueEntry {
    /// Create a new queue entry.
    #[must_use]
    pub fn new(
        id: DownloadId,
        priority: DownloadPriority,
        request: DownloadRequest,
        seq: u64,
    ) -> Self {
        Self {
            id,
            priority,
            request,
            inserted_at: Instant::now(),
            seq,
        }
    }
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.seq == other.seq
    }
}

impl Eq for QueueEntry {}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueueEntry {
    /// `BinaryHeap` is a max-heap, so higher priority dequeues first.
    /// Within same priority, earlier sequence number dequeues first (FIFO).
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// Priority-based download queue using a binary heap.
#[derive(Debug)]
pub struct PriorityQueue {
    heap: std::collections::BinaryHeap<QueueEntry>,
    next_seq: u64,
}

impl PriorityQueue {
    /// Create an empty priority queue.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            heap: std::collections::BinaryHeap::new(),
            next_seq: 0,
        }
    }

    /// Push an entry onto the queue, assigning a sequence number.
    pub fn push(&mut self, mut entry: QueueEntry) {
        entry.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.heap.push(entry);
    }

    /// Pop the highest-priority entry.
    pub fn pop(&mut self) -> Option<QueueEntry> {
        self.heap.pop()
    }

    /// Peek at the highest-priority entry without removing it.
    #[must_use]
    pub fn peek(&self) -> Option<&QueueEntry> {
        self.heap.peek()
    }

    /// Remove an entry by download ID.
    ///
    /// This is O(n) in the queue size. For typical download queues this is acceptable.
    pub fn remove(&mut self, id: DownloadId) -> Option<QueueEntry> {
        let entries: Vec<QueueEntry> = self.heap.drain().collect();
        let pos = entries.iter().position(|e| e.id == id);
        if let Some(idx) = pos {
            let mut entries = entries;
            let removed = entries.swap_remove(idx);
            for entry in entries {
                self.heap.push(entry);
            }
            Some(removed)
        } else {
            // Restore all entries
            for entry in entries {
                self.heap.push(entry);
            }
            None
        }
    }

    /// Number of entries in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Whether the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Iterate over entries (arbitrary heap order).
    pub fn iter(&self) -> impl Iterator<Item = &QueueEntry> {
        self.heap.iter()
    }
}

impl Default for PriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Limits the number of concurrent downloads.
#[derive(Clone)]
pub struct ConcurrencyLimiter {
    semaphore: Arc<Semaphore>,
    max_concurrent: usize,
}

impl std::fmt::Debug for ConcurrencyLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcurrencyLimiter")
            .field("max_concurrent", &self.max_concurrent)
            .field("available_permits", &self.semaphore.available_permits())
            .finish()
    }
}

impl ConcurrencyLimiter {
    /// Create a new limiter allowing up to `max_concurrent` concurrent downloads.
    #[must_use]
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }

    /// Acquire a permit, blocking until one is available.
    ///
    /// # Errors
    ///
    /// Returns `DownloadError::RateLimited` if the semaphore is closed.
    pub async fn acquire(&self) -> Result<SemaphorePermit<'_>, DownloadError> {
        self.semaphore
            .acquire()
            .await
            .map_err(|_| DownloadError::RateLimited)
    }

    /// Number of currently available permits.
    #[must_use]
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Maximum concurrent downloads.
    #[must_use]
    pub const fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use super::*;

    fn dummy_request() -> DownloadRequest {
        DownloadRequest {
            url: "https://example.com/file".to_string(),
            destination: PathBuf::from("/tmp/file"),
            priority: DownloadPriority::Normal,
            checksum: None,
            headers: HashMap::new(),
            max_connections: None,
            speed_limit: None,
            mirrors: Vec::new(),
            proxy: None,
        }
    }

    fn make_entry(priority: DownloadPriority) -> QueueEntry {
        let id = DownloadId::new();
        QueueEntry::new(id, priority, dummy_request(), 0)
    }

    // --- QueueEntry ordering tests ---

    #[test]
    fn critical_is_higher_than_high() {
        let a = make_entry(DownloadPriority::Critical);
        let b = make_entry(DownloadPriority::High);
        assert!(a > b, "Critical should be greater than High");
    }

    #[test]
    fn high_is_higher_than_normal() {
        let a = make_entry(DownloadPriority::High);
        let b = make_entry(DownloadPriority::Normal);
        assert!(a > b, "High should be greater than Normal");
    }

    #[test]
    fn normal_is_higher_than_low() {
        let a = make_entry(DownloadPriority::Normal);
        let b = make_entry(DownloadPriority::Low);
        assert!(a > b, "Normal should be greater than Low");
    }

    #[test]
    fn same_priority_fifo_ordering() {
        let mut a = make_entry(DownloadPriority::Normal);
        let mut b = make_entry(DownloadPriority::Normal);
        // Earlier seq = higher priority in max-heap
        a.seq = 0;
        b.seq = 1;
        assert!(
            a > b,
            "Earlier seq should dequeue first within same priority"
        );
    }

    // --- PriorityQueue tests ---

    #[test]
    fn new_queue_is_empty() {
        let mut q = PriorityQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert!(q.peek().is_none());
        assert!(q.pop().is_none());
    }

    #[test]
    fn push_pop_single() {
        let mut q = PriorityQueue::new();
        let entry = make_entry(DownloadPriority::Normal);
        let id = entry.id;
        q.push(entry);
        assert!(!q.is_empty());
        assert_eq!(q.len(), 1);
        let popped = q.pop().expect("should have one entry");
        assert_eq!(popped.id, id);
        assert!(q.is_empty());
    }

    #[test]
    fn pop_returns_highest_priority_first() {
        let mut q = PriorityQueue::new();
        let low = make_entry(DownloadPriority::Low);
        let high = make_entry(DownloadPriority::High);
        let critical = make_entry(DownloadPriority::Critical);
        let normal = make_entry(DownloadPriority::Normal);

        let low_id = low.id;
        let normal_id = normal.id;
        let high_id = high.id;
        let critical_id = critical.id;

        // Push in random order
        q.push(low);
        q.push(high);
        q.push(critical);
        q.push(normal);

        assert_eq!(q.pop().expect("pop 1").id, critical_id);
        assert_eq!(q.pop().expect("pop 2").id, high_id);
        assert_eq!(q.pop().expect("pop 3").id, normal_id);
        assert_eq!(q.pop().expect("pop 4").id, low_id);
        assert!(q.pop().is_none());
    }

    #[test]
    fn fifo_within_same_priority() {
        let mut q = PriorityQueue::new();
        let a = make_entry(DownloadPriority::Normal);
        let b = make_entry(DownloadPriority::Normal);
        let c = make_entry(DownloadPriority::Normal);

        let a_id = a.id;
        let b_id = b.id;
        let c_id = c.id;

        q.push(a);
        q.push(b);
        q.push(c);

        assert_eq!(q.pop().expect("pop 1").id, a_id);
        assert_eq!(q.pop().expect("pop 2").id, b_id);
        assert_eq!(q.pop().expect("pop 3").id, c_id);
    }

    #[test]
    fn peek_does_not_remove() {
        let mut q = PriorityQueue::new();
        let entry = make_entry(DownloadPriority::High);
        q.push(entry.clone());
        let peeked = q.peek().expect("peek");
        assert_eq!(peeked.id, entry.id);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn remove_by_id() {
        let mut q = PriorityQueue::new();
        let a = make_entry(DownloadPriority::Low);
        let b = make_entry(DownloadPriority::High);
        let c = make_entry(DownloadPriority::Normal);

        let a_id = a.id;
        let b_id = b.id;
        let c_id = c.id;

        q.push(a);
        q.push(b);
        q.push(c);

        let removed = q.remove(b_id);
        assert!(removed.is_some());
        assert_eq!(removed.expect("removed").id, b_id);
        assert_eq!(q.len(), 2);

        // Remaining should be a (Low) and c (Normal) -> Normal pops first
        let first = q.pop().expect("pop 1");
        assert_eq!(first.id, c_id);
        let second = q.pop().expect("pop 2");
        assert_eq!(second.id, a_id);
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut q = PriorityQueue::new();
        q.push(make_entry(DownloadPriority::Normal));
        let missing = q.remove(DownloadId::new());
        assert!(missing.is_none());
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn iter_yields_all_entries() {
        let mut q = PriorityQueue::new();
        q.push(make_entry(DownloadPriority::Low));
        q.push(make_entry(DownloadPriority::High));
        q.push(make_entry(DownloadPriority::Normal));
        assert_eq!(q.iter().count(), 3);
    }

    #[test]
    fn default_is_empty() {
        let q = PriorityQueue::default();
        assert!(q.is_empty());
    }

    // --- ConcurrencyLimiter tests ---

    #[tokio::test]
    async fn limiter_limits_concurrency() {
        let limiter = ConcurrencyLimiter::new(2);
        assert_eq!(limiter.max_concurrent(), 2);
        assert_eq!(limiter.available_permits(), 2);

        let p1 = limiter.acquire().await.expect("acquire 1");
        assert_eq!(limiter.available_permits(), 1);

        let p2 = limiter.acquire().await.expect("acquire 2");
        assert_eq!(limiter.available_permits(), 0);

        drop(p1);
        assert_eq!(limiter.available_permits(), 1);

        drop(p2);
        assert_eq!(limiter.available_permits(), 2);
    }

    #[tokio::test]
    async fn limiter_permit_release_allows_next() {
        let limiter = Arc::new(ConcurrencyLimiter::new(1));
        let p = limiter.acquire().await.expect("acquire");
        assert_eq!(limiter.available_permits(), 0);

        let limiter2 = Arc::clone(&limiter);
        let handle = tokio::spawn(async move {
            let permit = limiter2.acquire().await.expect("acquire after release");
            // permit is dropped here; verify it was obtained
            assert!(true);
            drop(permit);
        });

        // The spawned task should be waiting
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!handle.is_finished());

        // Release the permit
        drop(p);

        // Wait for the spawned task to complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(handle.is_finished());
    }
}
