//! Snapshot + buffered-delta order-book alignment.
//!
//! Exchanges publish a full depth snapshot over REST and an incremental delta
//! stream over WebSocket. On the initial subscription and on **every** data
//! WebSocket reconnect the two must be stitched back together by sequence id:
//!
//! 1. start buffering incoming deltas;
//! 2. request and await a REST snapshot with `last_update_id = U`;
//! 3. drop every buffered delta whose `final_update_id <= U`;
//! 4. replay the remaining deltas, requiring `first_update_id <= U + 1`;
//! 5. stop buffering and apply deltas directly.
//!
//! While synchronized, a delta whose `first_update_id` skips ahead of
//! `last_applied + 1` is a gap and triggers a new resync. This is the same
//! sequence used by the Binance/Nautilus adapters, expressed as a pure state
//! machine with no I/O so it can be exhaustively unit tested.
//!
//! The pending buffer is bounded by [`DEFAULT_MAX_BUFFERED_DELTAS`] (4096
//! deltas); raise it with [`BookSync::with_max_buffered`] only if the adapter
//! expects longer snapshot outages.

use std::{collections::VecDeque, fmt};

/// Default maximum number of deltas held while a snapshot is pending.
///
/// At a 100 ms depth-update interval this covers roughly seven minutes of
/// stream, which is well beyond a normal snapshot request.
pub const DEFAULT_MAX_BUFFERED_DELTAS: usize = 4_096;

/// Phases of the order-book alignment state machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BookPhase {
    /// Waiting for a snapshot while deltas accumulate.
    Buffering,
    /// Snapshot installed; deltas may be applied directly.
    #[default]
    Synchronized,
    /// A gap was detected; a fresh snapshot is required.
    Resync,
}

/// A delta frame's identity in the exchange sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeltaId {
    /// First update id covered by the frame (Binance `U`).
    pub first_update_id: u64,
    /// Final update id covered by the frame (Binance `u`).
    pub final_update_id: u64,
}

impl DeltaId {
    /// Create a delta identity.
    #[inline]
    #[must_use]
    pub const fn new(first_update_id: u64, final_update_id: u64) -> Self {
        Self {
            first_update_id,
            final_update_id,
        }
    }

    /// Whether the ids are ordered correctly.
    #[inline]
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.first_update_id <= self.final_update_id
    }
}

/// What a caller must do with a delta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaDecision {
    /// Hold the delta until a snapshot arrives.
    Buffer,
    /// Apply the delta immediately.
    Apply,
    /// Ignore the delta (already covered by the snapshot).
    Drop,
    /// A sequence gap was detected; request a fresh snapshot.
    Gap,
}

/// Failure when installing a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookError {
    /// A snapshot arrived while the machine was already synchronized.
    UnexpectedSnapshot,
    /// The first replayable delta does not connect to the snapshot.
    SequenceGap {
        /// Expected first update id.
        expected: u64,
        /// Actual first update id of the next delta.
        actual: u64,
    },
    /// A delta had `first_update_id > final_update_id`.
    InvalidDelta,
    /// The buffered-delta queue reached its configured limit.
    BufferLimitReached {
        /// Configured maximum number of buffered deltas.
        limit: usize,
    },
}

impl fmt::Display for BookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedSnapshot => f.write_str("snapshot arrived while already synchronized"),
            Self::SequenceGap { expected, actual } => {
                write!(f, "sequence gap: expected {expected}, got {actual}")
            }
            Self::InvalidDelta => f.write_str("delta has first_update_id > final_update_id"),
            Self::BufferLimitReached { limit } => {
                write!(f, "order-book buffer limit of {limit} deltas reached")
            }
        }
    }
}

impl std::error::Error for BookError {}

/// Snapshot + buffered-delta alignment state machine.
///
/// `D` is the caller's delta type; it is stored as-is and returned in wire
/// order, so this type performs no allocation per field beyond the buffer.
#[derive(Debug)]
pub struct BookSync<D> {
    phase: BookPhase,
    buffered: VecDeque<(DeltaId, D)>,
    last_applied_id: Option<u64>,
    last_snapshot_id: Option<u64>,
    max_buffered: usize,
}

impl<D> Default for BookSync<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D> BookSync<D> {
    /// Create a state machine that requires an initial snapshot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: BookPhase::Buffering,
            buffered: VecDeque::new(),
            last_applied_id: None,
            last_snapshot_id: None,
            max_buffered: DEFAULT_MAX_BUFFERED_DELTAS,
        }
    }

    /// Create a state machine with an explicit buffer limit.
    ///
    /// A limit of zero is treated as one so the state machine can always hold
    /// the delta that exposed a sequence gap.
    #[must_use]
    pub fn with_max_buffered(max_buffered: usize) -> Self {
        Self {
            max_buffered: max_buffered.max(1),
            ..Self::new()
        }
    }

    /// Configured maximum number of buffered deltas.
    #[inline]
    #[must_use]
    pub const fn max_buffered(&self) -> usize {
        self.max_buffered
    }

    /// Current phase.
    #[inline]
    #[must_use]
    pub const fn phase(&self) -> BookPhase {
        self.phase
    }

    /// Number of deltas currently buffered.
    #[inline]
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffered.len()
    }

    /// The last update id that has been applied, if any.
    #[inline]
    #[must_use]
    pub const fn last_applied_id(&self) -> Option<u64> {
        self.last_applied_id
    }

    /// The installed snapshot's `last_update_id`, if any.
    #[inline]
    #[must_use]
    pub const fn last_snapshot_id(&self) -> Option<u64> {
        self.last_snapshot_id
    }

    /// Force a resync and discard everything buffered so far.
    ///
    /// Call on initial subscription and on every reconnect. A sequence gap
    /// detected while synchronized already moves the machine to
    /// [`BookPhase::Resync`] and keeps the delta that exposed the gap, so do
    /// **not** call this for gap recovery: it would discard a delta that a
    /// snapshot older than the gap can still replay.
    pub fn start_resync(&mut self) {
        self.phase = BookPhase::Buffering;
        self.buffered.clear();
        self.last_applied_id = None;
        self.last_snapshot_id = None;
    }

    /// Append a delta to the pending buffer, enforcing the configured limit.
    fn buffer_delta(&mut self, id: DeltaId, delta: D) -> Result<(), BookError> {
        if self.buffered.len() >= self.max_buffered {
            return Err(BookError::BufferLimitReached {
                limit: self.max_buffered,
            });
        }
        self.buffered.push_back((id, delta));
        Ok(())
    }

    /// Feed an incoming delta.
    ///
    /// Returns the decision for this delta. In [`BookPhase::Buffering`] and
    /// [`BookPhase::Resync`] the delta is stored; in
    /// [`BookPhase::Synchronized`] it is applied, dropped as a duplicate, or
    /// reported as a gap (which moves the machine to [`BookPhase::Resync`]).
    ///
    /// # Errors
    ///
    /// Returns [`BookError::InvalidDelta`] for a malformed id pair, or
    /// [`BookError::BufferLimitReached`] when the pending buffer is full.
    pub fn on_delta(&mut self, id: DeltaId, delta: D) -> Result<DeltaDecision, BookError> {
        if !id.is_valid() {
            return Err(BookError::InvalidDelta);
        }

        match self.phase {
            BookPhase::Buffering | BookPhase::Resync => {
                self.buffer_delta(id, delta)?;
                Ok(DeltaDecision::Buffer)
            }
            BookPhase::Synchronized => {
                let last = self.last_applied_id.unwrap_or(0);
                if id.final_update_id <= last {
                    return Ok(DeltaDecision::Drop);
                }
                if id.first_update_id > last.saturating_add(1) {
                    self.phase = BookPhase::Resync;
                    self.buffered.clear();
                    // Keep the delta that exposed the gap: a snapshot older
                    // than this delta can still be stitched to it, and a
                    // newer one simply drops it during the snapshot prune.
                    self.buffer_delta(id, delta)?;
                    return Ok(DeltaDecision::Gap);
                }
                self.last_applied_id = Some(id.final_update_id);
                Ok(DeltaDecision::Apply)
            }
        }
    }

    /// Consume the delta without storing it (used for `Apply`/`Drop`).
    ///
    /// This is a convenience helper for adapters whose delta type is `()`:
    /// it lets tests and callers drive the state machine without allocating.
    #[inline]
    pub fn on_delta_id(&mut self, id: DeltaId) -> Result<DeltaDecision, BookError>
    where
        D: Default,
    {
        self.on_delta(id, D::default())
    }

    /// Install a REST snapshot and return the deltas that must be replayed.
    ///
    /// Drops every buffered delta with `final_update_id <= last_update_id`,
    /// then verifies that the remaining deltas connect to
    /// `last_update_id + 1` and to each other. The machine becomes
    /// [`BookPhase::Synchronized`] and its last applied id is advanced past
    /// the replayed deltas.
    ///
    /// A snapshot may arrive while the machine is buffering or while it is
    /// waiting for a new snapshot after a gap; a snapshot while already
    /// synchronized is rejected.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnexpectedSnapshot`] when the machine is already
    /// synchronized, or [`BookError::SequenceGap`] when the buffered deltas
    /// do not form a continuous chain from the snapshot. On a gap the buffer
    /// is preserved (deltas covered by the snapshot are dropped) and the
    /// machine moves to [`BookPhase::Resync`], so a newer snapshot can still
    /// be applied.
    pub fn on_snapshot(&mut self, last_update_id: u64) -> Result<Vec<(DeltaId, D)>, BookError> {
        if !matches!(self.phase, BookPhase::Buffering | BookPhase::Resync) {
            return Err(BookError::UnexpectedSnapshot);
        }

        while self
            .buffered
            .front()
            .is_some_and(|(id, _)| id.final_update_id <= last_update_id)
        {
            self.buffered.pop_front();
        }

        // Validate the whole replay chain before draining, so a gap leaves
        // the usable deltas in the buffer for the next snapshot attempt.
        let mut applied = last_update_id;
        for (id, _) in &self.buffered {
            let expected = applied.saturating_add(1);
            if id.first_update_id > expected {
                self.phase = BookPhase::Resync;
                return Err(BookError::SequenceGap {
                    expected,
                    actual: id.first_update_id,
                });
            }
            applied = applied.max(id.final_update_id);
        }

        let replay: Vec<_> = self.buffered.drain(..).collect();
        self.last_applied_id = Some(applied);
        self.last_snapshot_id = Some(last_update_id);
        self.phase = BookPhase::Synchronized;
        Ok(replay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_machine_waits_for_a_snapshot() {
        let book = BookSync::<()>::new();
        assert_eq!(book.phase(), BookPhase::Buffering);
        assert_eq!(book.buffered_len(), 0);
        assert_eq!(book.last_applied_id(), None);
    }

    #[test]
    fn buffers_deltas_until_snapshot_then_replays() {
        let mut book = BookSync::<u8>::new();
        assert_eq!(
            book.on_delta(DeltaId::new(98, 100), 1).expect("d1"),
            DeltaDecision::Buffer
        );
        assert_eq!(
            book.on_delta(DeltaId::new(101, 102), 2).expect("d2"),
            DeltaDecision::Buffer
        );
        assert_eq!(
            book.on_delta(DeltaId::new(103, 104), 3).expect("d3"),
            DeltaDecision::Buffer
        );
        assert_eq!(book.buffered_len(), 3);

        let replay = book.on_snapshot(100).expect("snapshot");
        assert_eq!(book.phase(), BookPhase::Synchronized);
        assert_eq!(book.last_snapshot_id(), Some(100));
        assert_eq!(book.last_applied_id(), Some(104));
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].0, DeltaId::new(101, 102));
        assert_eq!(replay[1].1, 3);
        assert_eq!(book.buffered_len(), 0);
    }

    #[test]
    fn synchronized_deltas_are_applied_in_order() {
        let mut book = BookSync::<()>::new();
        assert!(book.on_snapshot(100).expect("snapshot").is_empty());
        assert_eq!(
            book.on_delta_id(DeltaId::new(101, 101)).expect("apply"),
            DeltaDecision::Apply
        );
        assert_eq!(book.last_applied_id(), Some(101));
        assert_eq!(
            book.on_delta_id(DeltaId::new(100, 100)).expect("dup"),
            DeltaDecision::Drop
        );
    }

    #[test]
    fn a_sequence_gap_forces_a_resync() {
        let mut book = BookSync::<()>::new();
        assert!(book.on_snapshot(100).expect("snapshot").is_empty());
        assert_eq!(
            book.on_delta_id(DeltaId::new(105, 105)).expect("gap"),
            DeltaDecision::Gap
        );
        assert_eq!(book.phase(), BookPhase::Resync);

        // Deltas arriving after the gap are held for the next snapshot.
        assert_eq!(
            book.on_delta_id(DeltaId::new(106, 106)).expect("buffer"),
            DeltaDecision::Buffer
        );
        book.start_resync();
        assert_eq!(book.phase(), BookPhase::Buffering);
        assert_eq!(book.buffered_len(), 0);
    }

    #[test]
    fn the_delta_that_exposed_a_gap_is_replayed_after_the_snapshot() {
        let mut book = BookSync::<u8>::new();
        assert!(book.on_snapshot(100).expect("snapshot").is_empty());
        assert_eq!(
            book.on_delta(DeltaId::new(105, 105), 7).expect("gap delta"),
            DeltaDecision::Gap
        );
        assert_eq!(book.buffered_len(), 1);

        let replay = book.on_snapshot(104).expect("snapshot before the gap");
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0], (DeltaId::new(105, 105), 7));
        assert_eq!(book.last_applied_id(), Some(105));
    }

    #[test]
    fn snapshot_gap_is_reported() {
        let mut book = BookSync::<()>::new();
        assert_eq!(
            book.on_delta_id(DeltaId::new(110, 112)).expect("delta"),
            DeltaDecision::Buffer
        );
        assert_eq!(
            book.on_snapshot(100),
            Err(BookError::SequenceGap {
                expected: 101,
                actual: 110
            })
        );
        assert_eq!(book.phase(), BookPhase::Resync);

        // A newer snapshot that covers the gap recovers without a manual reset.
        assert!(book.on_snapshot(112).expect("recovered").is_empty());
        assert_eq!(book.phase(), BookPhase::Synchronized);
        assert_eq!(book.last_applied_id(), Some(112));
    }

    #[test]
    fn a_gap_inside_the_buffered_replay_is_reported() {
        let mut book = BookSync::<()>::new();
        assert_eq!(
            book.on_delta_id(DeltaId::new(101, 102)).expect("first"),
            DeltaDecision::Buffer
        );
        assert_eq!(
            book.on_delta_id(DeltaId::new(105, 106)).expect("second"),
            DeltaDecision::Buffer
        );
        assert_eq!(
            book.on_snapshot(100),
            Err(BookError::SequenceGap {
                expected: 103,
                actual: 105
            })
        );
        assert_eq!(book.phase(), BookPhase::Resync);
        assert_eq!(book.buffered_len(), 2, "a failed snapshot keeps the buffer");
    }

    #[test]
    fn a_snapshot_is_accepted_while_waiting_for_a_resync() {
        let mut book = BookSync::<()>::new();
        assert!(book.on_snapshot(100).expect("snapshot").is_empty());
        assert_eq!(
            book.on_delta_id(DeltaId::new(105, 105)).expect("gap"),
            DeltaDecision::Gap
        );
        assert_eq!(book.phase(), BookPhase::Resync);
        assert_eq!(
            book.on_delta_id(DeltaId::new(106, 106)).expect("buffer"),
            DeltaDecision::Buffer
        );

        let replay = book.on_snapshot(105).expect("snapshot during resync");
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].0, DeltaId::new(106, 106));
        assert_eq!(book.phase(), BookPhase::Synchronized);
        assert_eq!(book.last_applied_id(), Some(106));
    }

    #[test]
    fn unexpected_snapshot_is_rejected() {
        let mut book = BookSync::<()>::new();
        assert!(book.on_snapshot(10).expect("first").is_empty());
        assert_eq!(book.on_snapshot(11), Err(BookError::UnexpectedSnapshot));
    }

    #[test]
    fn malformed_delta_is_rejected() {
        let mut book = BookSync::<()>::new();
        assert_eq!(
            book.on_delta_id(DeltaId::new(9, 8)),
            Err(BookError::InvalidDelta)
        );
    }

    #[test]
    fn the_resync_buffer_is_bounded() {
        let mut book = BookSync::<()>::with_max_buffered(2);
        assert_eq!(book.max_buffered(), 2);
        assert_eq!(
            book.on_delta_id(DeltaId::new(1, 1)).expect("first"),
            DeltaDecision::Buffer
        );
        assert_eq!(
            book.on_delta_id(DeltaId::new(2, 2)).expect("second"),
            DeltaDecision::Buffer
        );
        assert_eq!(
            book.on_delta_id(DeltaId::new(3, 3)),
            Err(BookError::BufferLimitReached { limit: 2 })
        );
        assert_eq!(book.buffered_len(), 2);
    }

    #[test]
    fn the_default_buffer_limit_is_positive_and_configurable() {
        assert_eq!(
            BookSync::<()>::new().max_buffered(),
            DEFAULT_MAX_BUFFERED_DELTAS
        );
        assert_eq!(BookSync::<()>::with_max_buffered(0).max_buffered(), 1);
    }

    #[test]
    fn reconnect_restarts_the_whole_alignment() {
        let mut book = BookSync::<()>::new();
        assert!(book.on_snapshot(100).expect("snapshot").is_empty());
        assert_eq!(
            book.on_delta_id(DeltaId::new(101, 101)).expect("apply"),
            DeltaDecision::Apply
        );
        book.start_resync();
        assert_eq!(book.phase(), BookPhase::Buffering);
        assert_eq!(book.last_applied_id(), None);
        assert_eq!(
            book.on_delta_id(DeltaId::new(102, 102)).expect("buffer"),
            DeltaDecision::Buffer
        );
        assert!(book.on_snapshot(102).expect("resync").is_empty());
        assert_eq!(book.last_applied_id(), Some(102));
    }

    #[test]
    fn delta_id_validity() {
        assert!(DeltaId::new(1, 1).is_valid());
        assert!(DeltaId::new(1, 9).is_valid());
        assert!(!DeltaId::new(9, 1).is_valid());
    }

    #[test]
    fn book_errors_have_messages() {
        assert_eq!(
            format!("{}", BookError::UnexpectedSnapshot),
            "snapshot arrived while already synchronized"
        );
        assert_eq!(
            format!("{}", BookError::InvalidDelta),
            "delta has first_update_id > final_update_id"
        );
        assert_eq!(
            format!("{}", BookError::BufferLimitReached { limit: 2 }),
            "order-book buffer limit of 2 deltas reached"
        );
    }
}
