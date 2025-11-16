use std::path::Path;
use std::sync::{Arc, Mutex};

use prost_011::Message as ProstMessage011;
use raft::prelude::*;
use raft::storage::Storage;
use raft::{GetEntriesContext, Result as RaftResult};
use sled::Db;

/// Persistent storage implementation using sled database.
///
/// DiskStorage provides crash-safe persistent storage for Raft state:
/// - HardState (term, vote, commit)
/// - Log entries
/// - Snapshots
/// - Applied index
///
/// ## Data Model
/// The storage uses three sled trees (namespaces):
/// - "meta" → HardState, ConfState, applied_index
/// - "entries" → Log entries (key=index as big-endian u64)
/// - "snapshots" → Latest snapshot only
///
/// ## Crash Recovery
/// On restart, `DiskStorage::open()` loads persisted state from disk.
/// The Raft node can resume from its last persisted state without data loss.
pub struct DiskStorage {
    db: Db,
    meta_tree: sled::Tree,
    entries_tree: sled::Tree,
    snapshots_tree: sled::Tree,
    applied_index: Arc<Mutex<u64>>,
}

impl DiskStorage {
    /// Open or create storage at the given path.
    ///
    /// If the directory doesn't exist, it will be created with default Raft state.
    /// If the directory exists, persisted state will be loaded.
    ///
    /// # Arguments
    /// * `path` - Directory path for sled database
    ///
    /// # Returns
    /// - `Ok(DiskStorage)` - Storage ready to use
    /// - `Err(...)` - Database open error or corruption
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let db = sled::open(path)?;
        let meta_tree = db.open_tree("meta")?;
        let entries_tree = db.open_tree("entries")?;
        let snapshots_tree = db.open_tree("snapshots")?;

        // Load applied_index from disk (or default to 0)
        let applied_index = if let Some(bytes) = meta_tree.get("applied_index")? {
            let (index, _): (u64, usize) =
                bincode::decode_from_slice(&bytes, bincode::config::standard())?;
            index
        } else {
            0
        };

        Ok(Self {
            db,
            meta_tree,
            entries_tree,
            snapshots_tree,
            applied_index: Arc::new(Mutex::new(applied_index)),
        })
    }

    /// Get the current applied index.
    pub fn applied_index(&self) -> u64 {
        *self.applied_index.lock().unwrap()
    }

    /// Set the applied index and persist it to disk.
    pub fn set_applied_index(&self, index: u64) {
        *self.applied_index.lock().unwrap() = index;

        // Persist to disk
        let bytes = bincode::encode_to_vec(&index, bincode::config::standard()).unwrap();
        self.meta_tree
            .insert("applied_index", bytes.as_slice())
            .unwrap();
        self.meta_tree.flush().unwrap(); // Ensure durability
    }

    /// Append entries to the log.
    ///
    /// This persists the entries to disk immediately.
    pub fn append(&mut self, entries: &[Entry]) -> RaftResult<()> {
        for entry in entries {
            let key = index_to_key(entry.index);
            let value = encode_proto(entry);

            self.entries_tree
                .insert(key, value.as_slice())
                .map_err(|e| raft::Error::Store(raft::StorageError::Other(Box::new(e))))?;
        }

        // Flush to ensure durability
        self.entries_tree
            .flush()
            .map_err(|e| raft::Error::Store(raft::StorageError::Other(Box::new(e))))?;

        Ok(())
    }

    /// Apply a snapshot to the storage.
    ///
    /// This persists the snapshot to disk and updates the ConfState.
    pub fn apply_snapshot(&mut self, snapshot: Snapshot) -> RaftResult<()> {
        // Store snapshot data
        let snapshot_index = snapshot.get_metadata().index;
        let snapshot_bytes = encode_proto(&snapshot);

        self.snapshots_tree
            .insert("latest", snapshot_bytes.as_slice())
            .map_err(|e| raft::Error::Store(raft::StorageError::Other(Box::new(e))))?;

        // Store ConfState from snapshot metadata
        let conf_state = snapshot.get_metadata().get_conf_state();
        let conf_bytes = encode_proto(conf_state);

        self.meta_tree
            .insert("confstate", conf_bytes.as_slice())
            .map_err(|e| raft::Error::Store(raft::StorageError::Other(Box::new(e))))?;

        // Compact all entries before snapshot index
        // (we no longer need them since snapshot covers that range)
        let mut to_remove = Vec::new();
        for result in self.entries_tree.iter() {
            let (key, _) = result.map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(Box::new(e)))
            })?;
            let index = key_to_index(&key);
            if index <= snapshot_index {
                to_remove.push(key.to_vec());
            }
        }

        for key in to_remove {
            self.entries_tree.remove(key).map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(Box::new(e)))
            })?;
        }

        // Flush to ensure durability
        self.snapshots_tree
            .flush()
            .map_err(|e| raft::Error::Store(raft::StorageError::Other(Box::new(e))))?;
        self.meta_tree
            .flush()
            .map_err(|e| raft::Error::Store(raft::StorageError::Other(Box::new(e))))?;
        self.entries_tree
            .flush()
            .map_err(|e| raft::Error::Store(raft::StorageError::Other(Box::new(e))))?;

        Ok(())
    }

    /// Set the HardState (term, vote, commit).
    ///
    /// This persists the HardState to disk immediately.
    pub fn set_hardstate(&mut self, hs: HardState) {
        let bytes = encode_proto(&hs);
        self.meta_tree.insert("hardstate", bytes.as_slice()).unwrap();
        self.meta_tree.flush().unwrap(); // Ensure durability
    }

    /// Compact the log up to the given index (inclusive).
    ///
    /// Entries at or before compact_index are removed from storage.
    pub fn compact(&mut self, compact_index: u64) -> RaftResult<()> {
        let mut to_remove = Vec::new();

        for result in self.entries_tree.iter() {
            let (key, _) = result.map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(Box::new(e)))
            })?;
            let index = key_to_index(&key);
            if index <= compact_index {
                to_remove.push(key.to_vec());
            }
        }

        for key in to_remove {
            self.entries_tree.remove(key).map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(Box::new(e)))
            })?;
        }

        self.entries_tree
            .flush()
            .map_err(|e| raft::Error::Store(raft::StorageError::Other(Box::new(e))))?;

        Ok(())
    }

    /// Get the latest snapshot from disk (if any).
    fn get_latest_snapshot(&self) -> RaftResult<Option<Snapshot>> {
        if let Some(bytes) = self.snapshots_tree.get("latest").map_err(|e| {
            raft::Error::Store(raft::StorageError::Other(Box::new(e)))
        })? {
            let snapshot: Snapshot = decode_proto(&bytes).map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(e))
            })?;
            Ok(Some(snapshot))
        } else {
            Ok(None)
        }
    }

    /// Get an entry by index from disk.
    fn get_entry(&self, index: u64) -> RaftResult<Option<Entry>> {
        let key = index_to_key(index);

        if let Some(bytes) = self.entries_tree.get(key).map_err(|e| {
            raft::Error::Store(raft::StorageError::Other(Box::new(e)))
        })? {
            let entry: Entry = decode_proto(&bytes).map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(e))
            })?;
            Ok(Some(entry))
        } else {
            Ok(None)
        }
    }
}

impl Storage for DiskStorage {
    fn initial_state(&self) -> RaftResult<RaftState> {
        // Load HardState from disk (or use default)
        let hard_state = if let Some(bytes) = self.meta_tree.get("hardstate").map_err(|e| {
            raft::Error::Store(raft::StorageError::Other(Box::new(e)))
        })? {
            decode_proto(&bytes).map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(e))
            })?
        } else {
            HardState::default()
        };

        // Load ConfState from disk (or use default)
        let conf_state = if let Some(bytes) = self.meta_tree.get("confstate").map_err(|e| {
            raft::Error::Store(raft::StorageError::Other(Box::new(e)))
        })? {
            decode_proto(&bytes).map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(e))
            })?
        } else {
            ConfState::default()
        };

        Ok(RaftState {
            hard_state,
            conf_state,
        })
    }

    fn entries(
        &self,
        low: u64,
        high: u64,
        max_size: impl Into<Option<u64>>,
        _context: GetEntriesContext,
    ) -> RaftResult<Vec<Entry>> {
        let max_size = max_size.into().unwrap_or(u64::MAX);
        let mut entries = Vec::new();
        let mut total_size = 0u64;

        // Iterate from low to high (exclusive)
        for index in low..high {
            if let Some(entry) = self.get_entry(index)? {
                let entry_size = entry.encoded_len() as u64;

                if total_size + entry_size > max_size && !entries.is_empty() {
                    break; // Stop if we exceed max_size (but always return at least one entry)
                }

                total_size += entry_size;
                entries.push(entry);
            } else {
                // Entry not found - check if it's before snapshot index
                if let Some(snapshot) = self.get_latest_snapshot()? {
                    if index <= snapshot.get_metadata().index {
                        return Err(raft::Error::Store(raft::StorageError::Compacted));
                    }
                }
                // Entry simply doesn't exist yet
                break;
            }
        }

        Ok(entries)
    }

    fn term(&self, idx: u64) -> RaftResult<u64> {
        // Check if index is in a snapshot
        if let Some(snapshot) = self.get_latest_snapshot()? {
            let snapshot_index = snapshot.get_metadata().index;
            if idx == snapshot_index {
                return Ok(snapshot.get_metadata().term);
            } else if idx < snapshot_index {
                return Err(raft::Error::Store(raft::StorageError::Compacted));
            }
        }

        // Otherwise, look up the entry
        if let Some(entry) = self.get_entry(idx)? {
            Ok(entry.term)
        } else {
            Err(raft::Error::Store(raft::StorageError::Unavailable))
        }
    }

    fn first_index(&self) -> RaftResult<u64> {
        // If we have a snapshot, first index is snapshot_index + 1
        if let Some(snapshot) = self.get_latest_snapshot()? {
            return Ok(snapshot.get_metadata().index + 1);
        }

        // Otherwise, find the first entry in the log
        if let Some(result) = self.entries_tree.iter().next() {
            let (key, _) = result.map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(Box::new(e)))
            })?;
            Ok(key_to_index(&key))
        } else {
            // No entries at all, return 1 (default)
            Ok(1)
        }
    }

    fn last_index(&self) -> RaftResult<u64> {
        // Find the last entry in the log
        if let Some(result) = self.entries_tree.iter().next_back() {
            let (key, _) = result.map_err(|e| {
                raft::Error::Store(raft::StorageError::Other(Box::new(e)))
            })?;
            Ok(key_to_index(&key))
        } else if let Some(snapshot) = self.get_latest_snapshot()? {
            // No entries, but we have a snapshot
            Ok(snapshot.get_metadata().index)
        } else {
            // No entries and no snapshot
            Ok(0)
        }
    }

    fn snapshot(&self, _request_index: u64, _to: u64) -> RaftResult<Snapshot> {
        // Return the latest snapshot if we have one
        if let Some(snapshot) = self.get_latest_snapshot()? {
            Ok(snapshot)
        } else {
            // No snapshot available - return default empty snapshot
            Ok(Snapshot::default())
        }
    }
}

impl Clone for DiskStorage {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            meta_tree: self.meta_tree.clone(),
            entries_tree: self.entries_tree.clone(),
            snapshots_tree: self.snapshots_tree.clone(),
            applied_index: Arc::clone(&self.applied_index),
        }
    }
}

/// Convert a log index to a sled key (big-endian u64 bytes).
///
/// This ensures lexicographic ordering matches numerical ordering.
fn index_to_key(index: u64) -> [u8; 8] {
    index.to_be_bytes()
}

/// Convert a sled key back to a log index.
fn key_to_index(key: &[u8]) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(key);
    u64::from_be_bytes(bytes)
}

/// Encode a protobuf message to bytes.
fn encode_proto<M: ProstMessage011>(msg: &M) -> Vec<u8> {
    let mut buf = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut buf).unwrap();
    buf
}

/// Decode a protobuf message from bytes.
fn decode_proto<M: ProstMessage011 + Default>(
    bytes: &[u8],
) -> Result<M, Box<dyn std::error::Error + Send + Sync>> {
    Ok(M::decode(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_key_conversion() {
        assert_eq!(key_to_index(&index_to_key(0)), 0);
        assert_eq!(key_to_index(&index_to_key(1)), 1);
        assert_eq!(key_to_index(&index_to_key(100)), 100);
        assert_eq!(key_to_index(&index_to_key(u64::MAX)), u64::MAX);
    }

    #[test]
    fn test_key_ordering() {
        // Verify lexicographic ordering matches numerical ordering
        let key1 = index_to_key(1);
        let key10 = index_to_key(10);
        let key100 = index_to_key(100);

        assert!(key1 < key10);
        assert!(key10 < key100);
    }
}
