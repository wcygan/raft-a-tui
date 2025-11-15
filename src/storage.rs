use std::sync::{Arc, Mutex};

use raft::prelude::*;
use raft::storage::{MemStorage, Storage};
use raft::{GetEntriesContext, Result as RaftResult};

/// Wrapper around MemStorage that tracks applied index for restart safety.
///
/// The applied_index is stored separately because Raft doesn't guarantee
/// that the commit index is persisted before entries are applied to the
/// state machine. Tracking this prevents reapplication on restart.
pub struct RaftStorage {
    inner: MemStorage,
    applied_index: Arc<Mutex<u64>>,
}

impl RaftStorage {
    /// Create a new RaftStorage with empty state.
    pub fn new() -> Self {
        Self {
            inner: MemStorage::new(),
            applied_index: Arc::new(Mutex::new(0)),
        }
    }

    /// Create a new RaftStorage with the given configuration state.
    pub fn new_with_conf_state(conf_state: ConfState) -> Self {
        Self {
            inner: MemStorage::new_with_conf_state(conf_state),
            applied_index: Arc::new(Mutex::new(0)),
        }
    }

    /// Get the current applied index.
    pub fn applied_index(&self) -> u64 {
        *self.applied_index.lock().unwrap()
    }

    /// Set the applied index to the given value.
    pub fn set_applied_index(&self, index: u64) {
        *self.applied_index.lock().unwrap() = index;
    }

    /// Get a reference to the inner MemStorage for advanced operations.
    pub fn inner(&self) -> &MemStorage {
        &self.inner
    }

    /// Append entries to the log.
    pub fn append(&mut self, entries: &[Entry]) -> RaftResult<()> {
        self.inner.wl().append(entries)
    }

    /// Apply a snapshot to the storage.
    pub fn apply_snapshot(&mut self, snapshot: Snapshot) -> RaftResult<()> {
        self.inner.wl().apply_snapshot(snapshot)
    }

    /// Set the HardState (term, vote, commit).
    pub fn set_hardstate(&mut self, hs: HardState) {
        self.inner.wl().set_hardstate(hs);
    }

    /// Compact the log up to the given index.
    pub fn compact(&mut self, compact_index: u64) -> RaftResult<()> {
        self.inner.wl().compact(compact_index)
    }
}

impl Storage for RaftStorage {
    fn initial_state(&self) -> RaftResult<RaftState> {
        self.inner.initial_state()
    }

    fn entries(
        &self,
        low: u64,
        high: u64,
        max_size: impl Into<Option<u64>>,
        context: GetEntriesContext,
    ) -> RaftResult<Vec<Entry>> {
        self.inner.entries(low, high, max_size, context)
    }

    fn term(&self, idx: u64) -> RaftResult<u64> {
        self.inner.term(idx)
    }

    fn first_index(&self) -> RaftResult<u64> {
        self.inner.first_index()
    }

    fn last_index(&self) -> RaftResult<u64> {
        self.inner.last_index()
    }

    fn snapshot(&self, request_index: u64, to: u64) -> RaftResult<Snapshot> {
        self.inner.snapshot(request_index, to)
    }
}

impl Clone for RaftStorage {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            applied_index: Arc::clone(&self.applied_index),
        }
    }
}
