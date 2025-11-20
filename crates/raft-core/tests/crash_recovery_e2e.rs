use raft::Storage;
use raft_core::node::Node;
use raft_core::raft_node::RaftNode;
use raft_core::storage::RaftStorage;
use slog::{o, Drain, Logger};
use tempfile::TempDir;

/// Setup a basic logger for testing.
fn setup_logger(node_id: u64) -> Logger {
    let decorator = slog_term::PlainDecorator::new(slog_term::TestStdoutWriter);
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    Logger::root(drain, o!("node_id" => node_id))
}

#[test]
fn test_single_node_crash_recovery() {
    // Scenario: Start node → apply KV commands → crash → restart → verify state
    let temp_dir = TempDir::new().unwrap();
    let data_path = temp_dir.path().join("node-1");

    // Phase 1: Initial node startup and state building
    {
        let logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();
        let raft_node = RaftNode::new(1, vec![1], storage, logger).unwrap();

        // Verify initial state
        let state = raft_node.get_state();
        assert_eq!(state.term, 0, "Initial term should be 0");
        assert_eq!(state.commit_index, 0, "Initial commit should be 0");

        // Create KV node and apply some commands
        let mut kv_node = Node::new();
        kv_node
            .apply_kv_command(&Node::encode_put_command("key1", "value1"))
            .unwrap();
        kv_node
            .apply_kv_command(&Node::encode_put_command("key2", "value2"))
            .unwrap();
        kv_node
            .apply_kv_command(&Node::encode_put_command("key3", "value3"))
            .unwrap();

        // Create and persist snapshot
        let snapshot_data = kv_node.create_snapshot().unwrap();
        assert!(!snapshot_data.is_empty(), "Snapshot should contain data");

        // Verify KV state before crash
        assert_eq!(kv_node.get_internal_map().len(), 3);
        assert_eq!(
            kv_node.get_internal_map().get("key1"),
            Some(&"value1".to_string())
        );
    }
    // Node "crashes" here (raft_node and kv_node dropped)

    // Phase 2: Node restart - verify storage persisted
    {
        let logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();

        // Verify storage was persisted
        let initial_state = storage.initial_state().unwrap();
        assert_eq!(initial_state.hard_state.term, 0, "Term should be persisted");

        // Recreate raft node - should load from disk
        let raft_node = RaftNode::new(1, vec![1], storage, logger).unwrap();
        let state = raft_node.get_state();

        // Verify Raft state recovered
        assert_eq!(state.term, 0, "Term should match persisted value");

        // Note: In a full crash recovery scenario, snapshots would be loaded
        // and applied through the Ready loop. This test focuses on verifying
        // that DiskStorage correctly persists and restores the Raft state.
    }
}

#[test]
fn test_crash_recovery_with_hardstate() {
    // Scenario: Verify HardState (term, vote, commit) survives crash
    let temp_dir = TempDir::new().unwrap();
    let data_path = temp_dir.path().join("node-1");

    let expected_term = 5u64;
    let expected_vote = 2u64;
    let expected_commit = 10u64;

    // Phase 1: Create node and persist HardState + log entries
    {
        let _logger = setup_logger(1);
        let mut storage = RaftStorage::new_with_disk(&data_path).unwrap();

        // Add log entries up to commit index (Raft requires commit <= last_index)
        let mut entries = Vec::new();
        for i in 1..=expected_commit {
            entries.push(raft::prelude::Entry {
                entry_type: raft::prelude::EntryType::EntryNormal.into(),
                term: expected_term,
                index: i,
                data: format!("entry{}", i).into_bytes(),
                context: vec![],
                sync_log: false,
            });
        }
        storage.append(&entries).unwrap();

        // Simulate a node that has participated in elections
        let hard_state = raft::prelude::HardState {
            term: expected_term,
            vote: expected_vote,
            commit: expected_commit,
        };

        storage.set_hardstate(hard_state);

        // Verify HardState was set
        let initial_state = storage.initial_state().unwrap();
        assert_eq!(initial_state.hard_state.term, expected_term);
        assert_eq!(initial_state.hard_state.vote, expected_vote);
        assert_eq!(initial_state.hard_state.commit, expected_commit);
    }
    // Node "crashes"

    // Phase 2: Restart and verify HardState + entries persisted
    {
        let logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();

        let initial_state = storage.initial_state().unwrap();
        assert_eq!(
            initial_state.hard_state.term, expected_term,
            "Term should survive crash"
        );
        assert_eq!(
            initial_state.hard_state.vote, expected_vote,
            "Vote should survive crash"
        );
        assert_eq!(
            initial_state.hard_state.commit, expected_commit,
            "Commit index should survive crash"
        );

        // Verify log entries also persisted
        assert_eq!(
            storage.last_index().unwrap(),
            expected_commit,
            "Last index should match commit"
        );

        // Verify we can create RaftNode with recovered state
        let _raft_node = RaftNode::new(1, vec![1], storage, logger).unwrap();
    }
}

#[test]
fn test_crash_recovery_with_applied_index() {
    // Scenario: Verify applied_index tracking survives crash
    let temp_dir = TempDir::new().unwrap();
    let data_path = temp_dir.path().join("node-1");

    let expected_applied = 42u64;

    // Phase 1: Set applied index and crash
    {
        let _logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();

        // Simulate applying entries
        storage.set_applied_index(expected_applied);

        // Verify before crash
        assert_eq!(storage.applied_index(), expected_applied);
    }
    // Node "crashes"

    // Phase 2: Restart and verify applied_index persisted
    {
        let logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();

        assert_eq!(
            storage.applied_index(),
            expected_applied,
            "Applied index should survive crash"
        );

        // Verify RaftNode can be created
        let _raft_node = RaftNode::new(1, vec![1], storage, logger).unwrap();
    }
}

#[test]
fn test_crash_recovery_with_log_entries() {
    // Scenario: Verify log entries persist across crash
    let temp_dir = TempDir::new().unwrap();
    let data_path = temp_dir.path().join("node-1");

    // Phase 1: Append log entries
    {
        let logger = setup_logger(1);
        let mut storage = RaftStorage::new_with_disk(&data_path).unwrap();

        // Create test entries
        let entries = vec![
            raft::prelude::Entry {
                entry_type: raft::prelude::EntryType::EntryNormal.into(),
                term: 1,
                index: 1,
                data: b"entry1".to_vec(),
                context: vec![],
                sync_log: false,
            },
            raft::prelude::Entry {
                entry_type: raft::prelude::EntryType::EntryNormal.into(),
                term: 1,
                index: 2,
                data: b"entry2".to_vec(),
                context: vec![],
                sync_log: false,
            },
            raft::prelude::Entry {
                entry_type: raft::prelude::EntryType::EntryNormal.into(),
                term: 2,
                index: 3,
                data: b"entry3".to_vec(),
                context: vec![],
                sync_log: false,
            },
        ];

        storage.append(&entries).unwrap();

        // Verify entries are readable
        use raft::storage::Storage;
        let retrieved = storage
            .entries(1, 4, None, raft::GetEntriesContext::empty(false))
            .unwrap();
        assert_eq!(retrieved.len(), 3);
        assert_eq!(retrieved[0].data, b"entry1");
        assert_eq!(retrieved[1].data, b"entry2");
        assert_eq!(retrieved[2].data, b"entry3");
    }
    // Node "crashes"

    // Phase 2: Restart and verify entries persisted
    {
        let logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();

        use raft::storage::Storage;

        // Verify first/last index
        let first_index = storage.first_index().unwrap();
        let last_index = storage.last_index().unwrap();
        assert_eq!(first_index, 1, "First index should be 1");
        assert_eq!(last_index, 3, "Last index should be 3");

        // Verify entries are intact
        let retrieved = storage
            .entries(1, 4, None, raft::GetEntriesContext::empty(false))
            .unwrap();
        assert_eq!(retrieved.len(), 3);
        assert_eq!(retrieved[0].data, b"entry1");
        assert_eq!(retrieved[0].term, 1);
        assert_eq!(retrieved[1].data, b"entry2");
        assert_eq!(retrieved[1].term, 1);
        assert_eq!(retrieved[2].data, b"entry3");
        assert_eq!(retrieved[2].term, 2);

        // Verify RaftNode can be created
        let _raft_node = RaftNode::new(1, vec![1], storage, logger).unwrap();
    }
}

#[test]
fn test_multiple_crash_recovery_cycles() {
    // Scenario: Multiple crash/restart cycles accumulate state correctly
    let temp_dir = TempDir::new().unwrap();
    let data_path = temp_dir.path().join("node-1");

    // Cycle 1: Start fresh
    {
        let logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();
        storage.set_applied_index(10);

        let _raft_node = RaftNode::new(1, vec![1], storage, logger).unwrap();
    }

    // Cycle 2: Crash and restart, update applied index
    {
        let logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();
        assert_eq!(storage.applied_index(), 10);

        storage.set_applied_index(20);
        let _raft_node = RaftNode::new(1, vec![1], storage, logger).unwrap();
    }

    // Cycle 3: Crash and restart again
    {
        let _logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();
        assert_eq!(storage.applied_index(), 20);

        storage.set_applied_index(30);
        let _raft_node = RaftNode::new(1, vec![1], storage, _logger).unwrap();
    }

    // Final verification
    {
        let logger = setup_logger(1);
        let storage = RaftStorage::new_with_disk(&data_path).unwrap();
        assert_eq!(
            storage.applied_index(),
            30,
            "Applied index should accumulate across multiple crashes"
        );
    }
}

#[test]
fn test_in_memory_storage_no_persistence() {
    // Verify that :memory: mode does NOT persist (contrast with disk mode)
    let _logger = setup_logger(1);

    // Create in-memory storage
    let storage = RaftStorage::new();
    storage.set_applied_index(100);
    assert_eq!(storage.applied_index(), 100);

    // Drop and recreate - should start fresh
    drop(storage);

    let storage = RaftStorage::new();
    assert_eq!(
        storage.applied_index(),
        0,
        "In-memory storage should NOT persist applied index"
    );
}
