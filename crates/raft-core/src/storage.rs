use std::sync::{Arc, Mutex};

use raft::prelude::*;
use raft::storage::{MemStorage, Storage};
use raft::{GetEntriesContext, Result as RaftResult};

use crate::disk_storage::DiskStorage;

/// Raft storage abstraction supporting both memory and disk backends.
///
/// - Memory: Fast, non-persistent, useful for testing
/// - Disk: Persistent, crash-safe, production-ready
///
/// Both variants track applied_index to prevent reapplication on restart.
pub enum RaftStorage {
    /// In-memory storage with separate applied_index tracking
    Memory {
        inner: MemStorage,
        applied_index: Arc<Mutex<u64>>,
    },
    /// Disk-based persistent storage (applied_index built-in)
    Disk(DiskStorage),
}

impl RaftStorage {
    /// Create a new in-memory RaftStorage with empty state.
    pub fn new() -> Self {
        Self::Memory {
            inner: MemStorage::new(),
            applied_index: Arc::new(Mutex::new(0)),
        }
    }

    /// Create a new in-memory RaftStorage with the given configuration state.
    pub fn new_with_conf_state(conf_state: ConfState) -> Self {
        Self::Memory {
            inner: MemStorage::new_with_conf_state(conf_state),
            applied_index: Arc::new(Mutex::new(0)),
        }
    }

    /// Create a new disk-based RaftStorage.
    ///
    /// Opens or creates persistent storage at the given path.
    pub fn new_with_disk(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let disk = DiskStorage::open(path)?;
        Ok(Self::Disk(disk))
    }

    /// Initialize storage with the given configuration state.
    ///
    /// For memory storage, this always calls initialize_with_conf_state.
    /// For disk storage, this only initializes if the storage is brand new (no persisted state).
    pub fn initialize_with_conf_state(&mut self, conf_state: ConfState) {
        match self {
            Self::Memory { inner, .. } => {
                inner.initialize_with_conf_state(conf_state);
            }
            Self::Disk(disk) => {
                // Only initialize if this is a brand new DiskStorage with no existing state
                // Check if there's any persisted ConfState
                if let Ok(initial_state) = disk.initial_state() {
                    // If ConfState is empty (no voters), this is a fresh storage - initialize it
                    if initial_state.conf_state.voters.is_empty() {
                        // For disk storage, we need to manually persist the ConfState
                        // Create a dummy snapshot to carry the ConfState
                        use raft::prelude::*;
                        let mut snapshot = Snapshot::default();
                        snapshot.mut_metadata().set_conf_state(conf_state);
                        snapshot.mut_metadata().index = 0;
                        snapshot.mut_metadata().term = 0;

                        // Apply the snapshot to persist ConfState
                        let _ = disk.apply_snapshot(snapshot);
                    }
                }
            }
        }
    }

    /// Get the current applied index.
    pub fn applied_index(&self) -> u64 {
        match self {
            Self::Memory { applied_index, .. } => *applied_index.lock().unwrap(),
            Self::Disk(disk) => disk.applied_index(),
        }
    }

    /// Set the applied index to the given value.
    pub fn set_applied_index(&self, index: u64) -> RaftResult<()> {
        match self {
            Self::Memory { applied_index, .. } => {
                *applied_index.lock().unwrap() = index;
                Ok(())
            }
            Self::Disk(disk) => disk.set_applied_index(index),
        }
    }

    /// Append entries to the log.
    pub fn append(&mut self, entries: &[Entry]) -> RaftResult<()> {
        match self {
            Self::Memory { inner, .. } => inner.wl().append(entries),
            Self::Disk(disk) => disk.append(entries),
        }
    }

    /// Apply a snapshot to the storage.
    pub fn apply_snapshot(&mut self, snapshot: Snapshot) -> RaftResult<()> {
        match self {
            Self::Memory { inner, .. } => inner.wl().apply_snapshot(snapshot),
            Self::Disk(disk) => disk.apply_snapshot(snapshot),
        }
    }

    /// Set the HardState (term, vote, commit).
    pub fn set_hardstate(&mut self, hs: HardState) -> RaftResult<()> {
        match self {
            Self::Memory { inner, .. } => {
                inner.wl().set_hardstate(hs);
                Ok(())
            }
            Self::Disk(disk) => disk.set_hardstate(hs),
        }
    }

    /// Set the ConfState.
    pub fn set_conf_state(&mut self, cs: ConfState) -> RaftResult<()> {
        match self {
            Self::Memory { inner, .. } => {
                inner.wl().set_conf_state(cs);
                Ok(())
            }
            Self::Disk(disk) => disk.set_conf_state(cs),
        }
    }

    /// Compact the log up to the given index.
    pub fn compact(&mut self, compact_index: u64) -> RaftResult<()> {
        match self {
            Self::Memory { inner, .. } => inner.wl().compact(compact_index),
            Self::Disk(disk) => disk.compact(compact_index),
        }
    }
}

impl Storage for RaftStorage {
    fn initial_state(&self) -> RaftResult<RaftState> {
        match self {
            Self::Memory { inner, .. } => inner.initial_state(),
            Self::Disk(disk) => disk.initial_state(),
        }
    }

    fn entries(
        &self,
        low: u64,
        high: u64,
        max_size: impl Into<Option<u64>>,
        context: GetEntriesContext,
    ) -> RaftResult<Vec<Entry>> {
        match self {
            Self::Memory { inner, .. } => inner.entries(low, high, max_size, context),
            Self::Disk(disk) => disk.entries(low, high, max_size, context),
        }
    }

    fn term(&self, idx: u64) -> RaftResult<u64> {
        match self {
            Self::Memory { inner, .. } => inner.term(idx),
            Self::Disk(disk) => disk.term(idx),
        }
    }

    fn first_index(&self) -> RaftResult<u64> {
        match self {
            Self::Memory { inner, .. } => inner.first_index(),
            Self::Disk(disk) => disk.first_index(),
        }
    }

    fn last_index(&self) -> RaftResult<u64> {
        match self {
            Self::Memory { inner, .. } => inner.last_index(),
            Self::Disk(disk) => disk.last_index(),
        }
    }

    fn snapshot(&self, request_index: u64, to: u64) -> RaftResult<Snapshot> {
        match self {
            Self::Memory { inner, .. } => inner.snapshot(request_index, to),
            Self::Disk(disk) => disk.snapshot(request_index, to),
        }
    }
}

impl Clone for RaftStorage {
    fn clone(&self) -> Self {
        match self {
            Self::Memory {
                inner,
                applied_index,
            } => Self::Memory {
                inner: inner.clone(),
                applied_index: Arc::clone(applied_index),
            },
            Self::Disk(disk) => Self::Disk(disk.clone()),
        }
    }
}
