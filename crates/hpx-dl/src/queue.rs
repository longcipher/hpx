//! Priority download queue.

use std::{cmp::Ordering, time::Instant};

use crate::types::{DownloadId, DownloadPriority, DownloadRequest};

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
    tombstones: ahash::AHashSet<DownloadId>,
}

impl PriorityQueue {
    /// Create an empty priority queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            heap: std::collections::BinaryHeap::new(),
            next_seq: 0,
            tombstones: ahash::AHashSet::new(),
        }
    }

    /// Push an entry onto the queue, assigning a sequence number.
    pub fn push(&mut self, mut entry: QueueEntry) {
        entry.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.heap.push(entry);
    }

    /// Pop the highest-priority entry, skipping tombstoned entries.
    pub fn pop(&mut self) -> Option<QueueEntry> {
        loop {
            let entry = self.heap.pop()?;
            if self.tombstones.remove(&entry.id) {
                continue; // skip tombstoned entry
            }
            return Some(entry);
        }
    }

    /// Peek at the highest-priority entry without removing it.
    #[must_use]
    pub fn peek(&self) -> Option<&QueueEntry> {
        self.heap.peek()
    }

    /// Remove an entry by download ID.
    ///
    /// O(1) amortized — marks the entry as tombstoned. The actual memory is
    /// reclaimed when `pop()` encounters the tombstoned entry.
    pub fn remove(&mut self, id: DownloadId) {
        self.tombstones.insert(id);
    }

    /// Number of entries in the queue (may include tombstoned entries not yet popped).
    #[must_use]
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Whether the queue is empty (may return false if only tombstoned entries remain).
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

        q.remove(b_id);

        // Remaining should be a (Low) and c (Normal) -> Normal pops first
        let first = q.pop().expect("pop 1");
        assert_eq!(first.id, c_id);
        let second = q.pop().expect("pop 2");
        assert_eq!(second.id, a_id);
        assert!(q.pop().is_none());
    }

    #[test]
    fn remove_nonexistent_is_harmless() {
        let mut q = PriorityQueue::new();
        q.push(make_entry(DownloadPriority::Normal));
        q.remove(DownloadId::new());
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
}
