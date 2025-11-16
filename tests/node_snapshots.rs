use raft_a_tui::node::Node;

#[test]
fn test_snapshot_roundtrip_empty() {
    // Empty node → snapshot → restore → verify empty
    let node = Node::new();

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");
    // Note: Empty BTreeMap still has bincode metadata, so snapshot is not 0 bytes
    assert!(snapshot.len() > 0, "Snapshot should contain bincode metadata");

    let mut restored_node = Node::new();
    // Add some data first to verify it gets cleared
    restored_node
        .apply_kv_command(&Node::encode_put_command("temp", "data"))
        .unwrap();

    restored_node
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    assert_eq!(
        restored_node.get_internal_map().len(),
        0,
        "Restored node should be empty"
    );
}

#[test]
fn test_snapshot_roundtrip_single_entry() {
    // Node with 1 entry → snapshot → restore → verify identical
    let mut node = Node::new();
    node.apply_kv_command(&Node::encode_put_command("key1", "value1"))
        .unwrap();

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");
    assert!(!snapshot.is_empty(), "Non-empty node should produce non-empty snapshot");

    let mut restored_node = Node::new();
    restored_node
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    assert_eq!(
        restored_node.get_internal_map(),
        node.get_internal_map(),
        "Restored state should match original"
    );
    assert_eq!(
        restored_node.get_internal_map().get("key1"),
        Some(&"value1".to_string())
    );
}

#[test]
fn test_snapshot_roundtrip_multiple_entries() {
    // Node with 10 entries → snapshot → restore → verify identical
    let mut node = Node::new();
    for i in 0..10 {
        let key = format!("key{}", i);
        let value = format!("value{}", i);
        node.apply_kv_command(&Node::encode_put_command(&key, &value))
            .unwrap();
    }

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");

    let mut restored_node = Node::new();
    restored_node
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    assert_eq!(
        restored_node.get_internal_map(),
        node.get_internal_map(),
        "Restored state should match original"
    );
    assert_eq!(restored_node.get_internal_map().len(), 10);
}

#[test]
fn test_snapshot_preserves_btree_ordering() {
    // BTreeMap has deterministic ordering - verify it's preserved
    let mut node = Node::new();

    // Insert in non-alphabetical order
    node.apply_kv_command(&Node::encode_put_command("zebra", "z"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("apple", "a"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("mango", "m"))
        .unwrap();

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");

    let mut restored_node = Node::new();
    restored_node
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    // Verify ordering is preserved (BTreeMap keeps sorted order)
    let original_keys: Vec<_> = node.get_internal_map().keys().collect();
    let restored_keys: Vec<_> = restored_node.get_internal_map().keys().collect();

    assert_eq!(original_keys, restored_keys);
    assert_eq!(
        original_keys,
        vec![&"apple".to_string(), &"mango".to_string(), &"zebra".to_string()]
    );
}

#[test]
fn test_snapshot_large_dataset() {
    // Test with 1000 entries to verify performance is reasonable
    let mut node = Node::new();
    for i in 0..1000 {
        let key = format!("key{:04}", i);
        let value = format!("value{}", i);
        node.apply_kv_command(&Node::encode_put_command(&key, &value))
            .unwrap();
    }

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");
    assert!(snapshot.len() > 0, "Large dataset should produce non-empty snapshot");

    let mut restored_node = Node::new();
    restored_node
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    assert_eq!(
        restored_node.get_internal_map().len(),
        1000,
        "All entries should be restored"
    );

    // Spot check a few entries
    assert_eq!(
        restored_node.get_internal_map().get("key0000"),
        Some(&"value0".to_string())
    );
    assert_eq!(
        restored_node.get_internal_map().get("key0500"),
        Some(&"value500".to_string())
    );
    assert_eq!(
        restored_node.get_internal_map().get("key0999"),
        Some(&"value999".to_string())
    );
}

#[test]
fn test_restore_from_malformed_snapshot() {
    // Malformed data should return an error, not panic
    let mut node = Node::new();

    let malformed_data = vec![0xFF, 0xFF, 0xFF, 0xFF]; // Invalid bincode data

    let result = node.restore_from_snapshot(&malformed_data);
    assert!(
        result.is_err(),
        "Malformed snapshot should return an error"
    );
}

#[test]
fn test_restore_replaces_existing_state() {
    // Verify that restore REPLACES state, not merges
    let mut node1 = Node::new();
    node1
        .apply_kv_command(&Node::encode_put_command("key1", "value1"))
        .unwrap();
    node1
        .apply_kv_command(&Node::encode_put_command("key2", "value2"))
        .unwrap();

    let mut node2 = Node::new();
    node2
        .apply_kv_command(&Node::encode_put_command("key3", "value3"))
        .unwrap();

    let snapshot1 = node1.create_snapshot().expect("Failed to create snapshot");

    // Restore snapshot1 into node2 - should REPLACE, not merge
    node2
        .restore_from_snapshot(&snapshot1)
        .expect("Failed to restore snapshot");

    assert_eq!(node2.get_internal_map().len(), 2, "Should have exactly 2 keys");
    assert!(
        node2.get_internal_map().contains_key("key1"),
        "Should have key1 from snapshot"
    );
    assert!(
        node2.get_internal_map().contains_key("key2"),
        "Should have key2 from snapshot"
    );
    assert!(
        !node2.get_internal_map().contains_key("key3"),
        "Should NOT have key3 (was replaced)"
    );
}

#[test]
fn test_empty_snapshot_clears_state() {
    // Empty snapshot should clear all existing state
    let mut node = Node::new();
    node.apply_kv_command(&Node::encode_put_command("key1", "value1"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("key2", "value2"))
        .unwrap();

    assert_eq!(node.get_internal_map().len(), 2, "Node should have 2 entries");

    // Restore empty snapshot
    node.restore_from_snapshot(&[])
        .expect("Failed to restore empty snapshot");

    assert_eq!(
        node.get_internal_map().len(),
        0,
        "Empty snapshot should clear all state"
    );
}

#[test]
fn test_snapshot_idempotent() {
    // Creating multiple snapshots of same state should produce identical results
    let mut node = Node::new();
    node.apply_kv_command(&Node::encode_put_command("key1", "value1"))
        .unwrap();

    let snapshot1 = node.create_snapshot().expect("Failed to create snapshot1");
    let snapshot2 = node.create_snapshot().expect("Failed to create snapshot2");

    assert_eq!(
        snapshot1, snapshot2,
        "Multiple snapshots of same state should be identical"
    );
}
