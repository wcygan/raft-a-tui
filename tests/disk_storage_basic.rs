use tempfile::TempDir;

use raft::prelude::*;
use raft::storage::Storage;
use raft_a_tui::disk_storage::DiskStorage;

/// Helper to create a temporary directory for tests.
fn temp_dir() -> TempDir {
    tempfile::tempdir().expect("Failed to create temp dir")
}

/// Helper to create test entries.
fn make_entry(index: u64, term: u64, data: &str) -> Entry {
    let mut entry = Entry::default();
    entry.index = index;
    entry.term = term;
    entry.data = data.as_bytes().to_vec();
    entry
}

/// Helper to create a ConfState with given voters.
fn make_conf_state(voters: Vec<u64>) -> ConfState {
    let mut cs = ConfState::default();
    cs.voters = voters;
    cs
}

#[test]
fn test_disk_storage_create_and_reopen() {
    let dir = temp_dir();
    let path = dir.path();

    // Create storage
    {
        let storage = DiskStorage::open(path).expect("Failed to create storage");
        assert_eq!(storage.applied_index(), 0, "New storage should have applied_index=0");
    }

    // Reopen storage - should succeed
    {
        let storage = DiskStorage::open(path).expect("Failed to reopen storage");
        assert_eq!(storage.applied_index(), 0, "Reopened storage should preserve applied_index");
    }
}

#[test]
fn test_applied_index_persistence() {
    let dir = temp_dir();
    let path = dir.path();

    // Create storage and set applied_index
    {
        let storage = DiskStorage::open(path).expect("Failed to create storage");
        storage.set_applied_index(42);
        assert_eq!(storage.applied_index(), 42);
    }

    // Reopen and verify applied_index persisted
    {
        let storage = DiskStorage::open(path).expect("Failed to reopen storage");
        assert_eq!(
            storage.applied_index(),
            42,
            "Applied index should persist across restarts"
        );
    }
}

#[test]
fn test_hardstate_persistence() {
    let dir = temp_dir();
    let path = dir.path();

    let mut hs = HardState::default();
    hs.term = 5;
    hs.vote = 2;
    hs.commit = 10;

    // Create storage and set HardState
    {
        let mut storage = DiskStorage::open(path).expect("Failed to create storage");
        storage.set_hardstate(hs.clone());
    }

    // Reopen and verify HardState persisted
    {
        let storage = DiskStorage::open(path).expect("Failed to reopen storage");
        let state = storage.initial_state().expect("Failed to get initial state");
        assert_eq!(state.hard_state.term, 5);
        assert_eq!(state.hard_state.vote, 2);
        assert_eq!(state.hard_state.commit, 10);
    }
}

#[test]
fn test_hardstate_updates() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Update HardState multiple times
    for i in 1..=5 {
        let mut hs = HardState::default();
        hs.term = i;
        hs.commit = i * 10;
        storage.set_hardstate(hs);
    }

    // Verify latest HardState
    let state = storage.initial_state().expect("Failed to get initial state");
    assert_eq!(state.hard_state.term, 5);
    assert_eq!(state.hard_state.commit, 50);
}

#[test]
fn test_confstate_persistence() {
    let dir = temp_dir();
    let path = dir.path();

    // Create storage with a snapshot containing ConfState
    {
        let mut storage = DiskStorage::open(path).expect("Failed to create storage");

        let mut snapshot = Snapshot::default();
        let mut metadata = SnapshotMetadata::default();
        metadata.index = 10;
        metadata.term = 3;
        metadata.set_conf_state(make_conf_state(vec![1, 2, 3]));
        snapshot.set_metadata(metadata);
        snapshot.data = vec![1, 2, 3]; // Some snapshot data

        storage.apply_snapshot(snapshot).expect("Failed to apply snapshot");
    }

    // Reopen and verify ConfState persisted
    {
        let storage = DiskStorage::open(path).expect("Failed to reopen storage");
        let state = storage.initial_state().expect("Failed to get initial state");
        assert_eq!(state.conf_state.voters, vec![1, 2, 3]);
    }
}

#[test]
fn test_append_and_retrieve_entries() {
    let dir = temp_dir();
    let path = dir.path();

    let entries = vec![
        make_entry(1, 1, "entry1"),
        make_entry(2, 1, "entry2"),
        make_entry(3, 2, "entry3"),
    ];

    // Append entries
    {
        let mut storage = DiskStorage::open(path).expect("Failed to create storage");
        storage.append(&entries).expect("Failed to append entries");
    }

    // Reopen and retrieve entries
    {
        let storage = DiskStorage::open(path).expect("Failed to reopen storage");

        let retrieved = storage
            .entries(1, 4, None, raft::GetEntriesContext::empty(false))
            .expect("Failed to retrieve entries");

        assert_eq!(retrieved.len(), 3);
        assert_eq!(retrieved[0].index, 1);
        assert_eq!(retrieved[0].data, b"entry1");
        assert_eq!(retrieved[1].index, 2);
        assert_eq!(retrieved[2].index, 3);
        assert_eq!(retrieved[2].term, 2);
    }
}

#[test]
fn test_entries_range_queries() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Append 10 entries
    let entries: Vec<_> = (1..=10)
        .map(|i| make_entry(i, 1, &format!("entry{}", i)))
        .collect();
    storage.append(&entries).expect("Failed to append entries");

    // Query range [3, 7)
    let range_entries = storage
        .entries(3, 7, None, raft::GetEntriesContext::empty(false))
        .expect("Failed to get range");
    assert_eq!(range_entries.len(), 4);
    assert_eq!(range_entries[0].index, 3);
    assert_eq!(range_entries[3].index, 6);

    // Query single entry [5, 6)
    let single_entry = storage
        .entries(5, 6, None, raft::GetEntriesContext::empty(false))
        .expect("Failed to get single entry");
    assert_eq!(single_entry.len(), 1);
    assert_eq!(single_entry[0].index, 5);
}

#[test]
fn test_entries_max_size_limit() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Append entries with large data
    let entries: Vec<_> = (1..=10)
        .map(|i| make_entry(i, 1, &"x".repeat(100))) // 100 bytes each
        .collect();
    storage.append(&entries).expect("Failed to append entries");

    // Query with max_size = 250 bytes (should get ~2 entries)
    let limited_entries = storage
        .entries(1, 11, Some(250), raft::GetEntriesContext::empty(false))
        .expect("Failed to get limited entries");

    // Should get at least 1 entry (always returns at least one)
    assert!(limited_entries.len() >= 1);
    // Should not get all 10 entries due to size limit
    assert!(limited_entries.len() < 10);
}

#[test]
fn test_first_and_last_index() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Empty storage
    assert_eq!(storage.first_index().unwrap(), 1); // Default
    assert_eq!(storage.last_index().unwrap(), 0); // No entries

    // Append entries [5, 6, 7]
    let entries = vec![
        make_entry(5, 1, "entry5"),
        make_entry(6, 1, "entry6"),
        make_entry(7, 2, "entry7"),
    ];
    storage.append(&entries).expect("Failed to append");

    assert_eq!(storage.first_index().unwrap(), 5);
    assert_eq!(storage.last_index().unwrap(), 7);
}

#[test]
fn test_term_lookup() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    let entries = vec![
        make_entry(1, 1, "entry1"),
        make_entry(2, 1, "entry2"),
        make_entry(3, 2, "entry3"),
        make_entry(4, 3, "entry4"),
    ];
    storage.append(&entries).expect("Failed to append");

    // Test term lookups
    assert_eq!(storage.term(1).unwrap(), 1);
    assert_eq!(storage.term(2).unwrap(), 1);
    assert_eq!(storage.term(3).unwrap(), 2);
    assert_eq!(storage.term(4).unwrap(), 3);

    // Term for non-existent entry should error
    assert!(storage.term(5).is_err());
}

#[test]
fn test_snapshot_persistence() {
    let dir = temp_dir();
    let path = dir.path();

    let snapshot_data = b"snapshot_state_data";

    // Create storage and apply snapshot
    {
        let mut storage = DiskStorage::open(path).expect("Failed to create storage");

        let mut snapshot = Snapshot::default();
        let mut metadata = SnapshotMetadata::default();
        metadata.index = 100;
        metadata.term = 5;
        metadata.set_conf_state(make_conf_state(vec![1, 2, 3]));
        snapshot.set_metadata(metadata);
        snapshot.data = snapshot_data.to_vec();

        storage.apply_snapshot(snapshot).expect("Failed to apply snapshot");
    }

    // Reopen and verify snapshot persisted
    {
        let storage = DiskStorage::open(path).expect("Failed to reopen storage");

        let retrieved_snapshot = storage
            .snapshot(0, u64::MAX)
            .expect("Failed to get snapshot");

        assert_eq!(retrieved_snapshot.get_metadata().index, 100);
        assert_eq!(retrieved_snapshot.get_metadata().term, 5);
        assert_eq!(retrieved_snapshot.data, snapshot_data);
        assert_eq!(
            retrieved_snapshot.get_metadata().get_conf_state().voters,
            vec![1, 2, 3]
        );
    }
}

#[test]
fn test_snapshot_compacts_log() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Append entries [1..=10]
    let entries: Vec<_> = (1..=10)
        .map(|i| make_entry(i, 1, &format!("entry{}", i)))
        .collect();
    storage.append(&entries).expect("Failed to append");

    // Verify all entries exist
    assert_eq!(storage.first_index().unwrap(), 1);
    assert_eq!(storage.last_index().unwrap(), 10);

    // Apply snapshot at index 7
    let mut snapshot = Snapshot::default();
    let mut metadata = SnapshotMetadata::default();
    metadata.index = 7;
    metadata.term = 1;
    snapshot.set_metadata(metadata);
    snapshot.data = b"snapshot_at_7".to_vec();

    storage.apply_snapshot(snapshot).expect("Failed to apply snapshot");

    // Verify entries [1..=7] are compacted
    // first_index should now be 8 (snapshot_index + 1)
    assert_eq!(storage.first_index().unwrap(), 8);
    assert_eq!(storage.last_index().unwrap(), 10);

    // Trying to get entries before snapshot should return Compacted error
    let result = storage.entries(1, 5, None, raft::GetEntriesContext::empty(false));
    assert!(result.is_err());
    match result {
        Err(raft::Error::Store(raft::StorageError::Compacted)) => {}
        _ => panic!("Expected Compacted error"),
    }

    // Entries after snapshot should still be available
    let remaining = storage
        .entries(8, 11, None, raft::GetEntriesContext::empty(false))
        .expect("Failed to get remaining entries");
    assert_eq!(remaining.len(), 3);
    assert_eq!(remaining[0].index, 8);
}

#[test]
fn test_snapshot_updates_first_index() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Apply snapshot at index 50 (no prior entries)
    let mut snapshot = Snapshot::default();
    let mut metadata = SnapshotMetadata::default();
    metadata.index = 50;
    metadata.term = 3;
    snapshot.set_metadata(metadata);

    storage.apply_snapshot(snapshot).expect("Failed to apply snapshot");

    // first_index should be snapshot_index + 1 = 51
    assert_eq!(storage.first_index().unwrap(), 51);
    assert_eq!(storage.last_index().unwrap(), 50); // No entries after snapshot
}

#[test]
fn test_term_from_snapshot() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Apply snapshot at index 100, term 5
    let mut snapshot = Snapshot::default();
    let mut metadata = SnapshotMetadata::default();
    metadata.index = 100;
    metadata.term = 5;
    snapshot.set_metadata(metadata);

    storage.apply_snapshot(snapshot).expect("Failed to apply snapshot");

    // term(100) should return snapshot's term
    assert_eq!(storage.term(100).unwrap(), 5);

    // term before snapshot should return Compacted error
    assert!(matches!(
        storage.term(99),
        Err(raft::Error::Store(raft::StorageError::Compacted))
    ));
}

#[test]
fn test_multiple_restarts_preserve_state() {
    let dir = temp_dir();
    let path = dir.path();

    // Round 1: Create and add some data
    {
        let mut storage = DiskStorage::open(path).expect("Failed to create storage");
        storage.set_applied_index(10);

        let entries = vec![make_entry(1, 1, "round1")];
        storage.append(&entries).expect("Failed to append");
    }

    // Round 2: Reopen and add more data
    {
        let mut storage = DiskStorage::open(path).expect("Failed to reopen storage");
        assert_eq!(storage.applied_index(), 10);

        let entries = vec![make_entry(2, 1, "round2")];
        storage.append(&entries).expect("Failed to append");
        storage.set_applied_index(20);
    }

    // Round 3: Reopen and verify all data
    {
        let storage = DiskStorage::open(path).expect("Failed to reopen storage");
        assert_eq!(storage.applied_index(), 20);

        let all_entries = storage
            .entries(1, 3, None, raft::GetEntriesContext::empty(false))
            .expect("Failed to get entries");

        assert_eq!(all_entries.len(), 2);
        assert_eq!(all_entries[0].data, b"round1");
        assert_eq!(all_entries[1].data, b"round2");
    }
}

#[test]
fn test_large_entry_storage() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Create a large entry (1 MB)
    let large_data = "x".repeat(1_000_000);
    let entry = make_entry(1, 1, &large_data);

    storage.append(&[entry]).expect("Failed to append large entry");

    // Retrieve and verify
    let retrieved = storage
        .entries(1, 2, None, raft::GetEntriesContext::empty(false))
        .expect("Failed to retrieve large entry");

    assert_eq!(retrieved.len(), 1);
    assert_eq!(retrieved[0].data.len(), 1_000_000);
}

#[test]
fn test_storage_clone() {
    let dir = temp_dir();
    let path = dir.path();

    let storage1 = DiskStorage::open(path).expect("Failed to create storage");
    storage1.set_applied_index(42);

    // Clone storage
    let storage2 = storage1.clone();

    // Both should share the same applied_index (Arc<Mutex<u64>>)
    assert_eq!(storage1.applied_index(), 42);
    assert_eq!(storage2.applied_index(), 42);

    // Update via storage1
    storage1.set_applied_index(100);

    // storage2 should see the update (shared Arc)
    assert_eq!(storage2.applied_index(), 100);
}

#[test]
fn test_initial_state_defaults() {
    let dir = temp_dir();
    let path = dir.path();

    let storage = DiskStorage::open(path).expect("Failed to create storage");
    let state = storage.initial_state().expect("Failed to get initial state");

    // New storage should have default HardState and ConfState
    assert_eq!(state.hard_state.term, 0);
    assert_eq!(state.hard_state.vote, 0);
    assert_eq!(state.hard_state.commit, 0);
    assert!(state.conf_state.voters.is_empty());
}

#[test]
fn test_empty_storage_snapshot_returns_default() {
    let dir = temp_dir();
    let path = dir.path();

    let storage = DiskStorage::open(path).expect("Failed to create storage");

    // No snapshot applied, should return empty/default snapshot
    let snapshot = storage
        .snapshot(0, u64::MAX)
        .expect("Failed to get snapshot");

    // Default snapshot has index=0
    assert_eq!(snapshot.get_metadata().index, 0);
}

#[test]
fn test_compact_removes_entries() {
    let dir = temp_dir();
    let path = dir.path();

    let mut storage = DiskStorage::open(path).expect("Failed to create storage");

    // Append entries [1..=10]
    let entries: Vec<_> = (1..=10)
        .map(|i| make_entry(i, 1, &format!("entry{}", i)))
        .collect();
    storage.append(&entries).expect("Failed to append");

    // Compact up to index 5
    storage.compact(5).expect("Failed to compact");

    // Entries [1..=5] should be gone
    let result = storage.entries(1, 6, None, raft::GetEntriesContext::empty(false));
    assert!(result.is_ok()); // But entries [1..=5] won't be found
    let found = result.unwrap();
    // Only entries after compaction remain
    assert!(found.is_empty() || found[0].index > 5);

    // Entries [6..=10] should still exist
    let remaining = storage
        .entries(6, 11, None, raft::GetEntriesContext::empty(false))
        .expect("Failed to get remaining");
    assert_eq!(remaining.len(), 5);
    assert_eq!(remaining[0].index, 6);
}
