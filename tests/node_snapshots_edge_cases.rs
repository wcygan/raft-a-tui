use raft_a_tui::node::Node;

#[test]
fn test_snapshot_unicode_data() {
    // Test with Unicode characters (emoji, Chinese, etc.)
    let mut node = Node::new();

    node.apply_kv_command(&Node::encode_put_command("emoji", "🚀🎉💾"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("chinese", "你好世界"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("arabic", "مرحبا"))
        .unwrap();

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");

    let mut restored = Node::new();
    restored
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    assert_eq!(
        restored.get_internal_map().get("emoji"),
        Some(&"🚀🎉💾".to_string())
    );
    assert_eq!(
        restored.get_internal_map().get("chinese"),
        Some(&"你好世界".to_string())
    );
    assert_eq!(
        restored.get_internal_map().get("arabic"),
        Some(&"مرحبا".to_string())
    );
}

#[test]
fn test_snapshot_special_characters() {
    // Test with special characters that might break serialization
    let mut node = Node::new();

    node.apply_kv_command(&Node::encode_put_command("newlines", "line1\nline2\nline3"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("tabs", "col1\tcol2\tcol3"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("quotes", r#"He said "hello""#))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("backslashes", r"C:\path\to\file"))
        .unwrap();

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");

    let mut restored = Node::new();
    restored
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    assert_eq!(
        restored.get_internal_map().get("newlines"),
        Some(&"line1\nline2\nline3".to_string())
    );
    assert_eq!(
        restored.get_internal_map().get("tabs"),
        Some(&"col1\tcol2\tcol3".to_string())
    );
}

#[test]
fn test_snapshot_very_long_keys_and_values() {
    let mut node = Node::new();

    // Very long key and value (10KB each)
    let long_key = "key_".to_string() + &"x".repeat(10_000);
    let long_value = "value_".to_string() + &"y".repeat(10_000);

    node.apply_kv_command(&Node::encode_put_command(&long_key, &long_value))
        .unwrap();

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");

    let mut restored = Node::new();
    restored
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    assert_eq!(restored.get_internal_map().get(&long_key), Some(&long_value));
    assert_eq!(restored.get_internal_map().len(), 1);
}

#[test]
fn test_snapshot_empty_keys_and_values() {
    let mut node = Node::new();

    // Empty value
    node.apply_kv_command(&Node::encode_put_command("empty_value", ""))
        .unwrap();

    let snapshot = node.create_snapshot().expect("Failed to create snapshot");

    let mut restored = Node::new();
    restored
        .restore_from_snapshot(&snapshot)
        .expect("Failed to restore snapshot");

    assert_eq!(
        restored.get_internal_map().get("empty_value"),
        Some(&"".to_string())
    );
}

#[test]
fn test_multiple_snapshots_same_node() {
    // Creating multiple snapshots should be deterministic
    let mut node = Node::new();

    node.apply_kv_command(&Node::encode_put_command("a", "1"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("b", "2"))
        .unwrap();

    let snap1 = node.create_snapshot().expect("Failed to create snapshot1");
    let snap2 = node.create_snapshot().expect("Failed to create snapshot2");
    let snap3 = node.create_snapshot().expect("Failed to create snapshot3");

    assert_eq!(snap1, snap2);
    assert_eq!(snap2, snap3);
}

#[test]
fn test_restore_multiple_times() {
    // Restoring multiple snapshots in sequence
    let mut node1 = Node::new();
    node1
        .apply_kv_command(&Node::encode_put_command("state", "v1"))
        .unwrap();
    let snap1 = node1.create_snapshot().unwrap();

    let mut node2 = Node::new();
    node2
        .apply_kv_command(&Node::encode_put_command("state", "v2"))
        .unwrap();
    node2
        .apply_kv_command(&Node::encode_put_command("extra", "data"))
        .unwrap();
    let snap2 = node2.create_snapshot().unwrap();

    let mut node3 = Node::new();
    node3
        .apply_kv_command(&Node::encode_put_command("state", "v3"))
        .unwrap();
    let snap3 = node3.create_snapshot().unwrap();

    // Restore in different order
    let mut target = Node::new();

    target.restore_from_snapshot(&snap1).unwrap();
    assert_eq!(target.get_internal_map().get("state"), Some(&"v1".to_string()));
    assert_eq!(target.get_internal_map().len(), 1);

    target.restore_from_snapshot(&snap2).unwrap();
    assert_eq!(target.get_internal_map().get("state"), Some(&"v2".to_string()));
    assert_eq!(target.get_internal_map().len(), 2);

    target.restore_from_snapshot(&snap3).unwrap();
    assert_eq!(target.get_internal_map().get("state"), Some(&"v3".to_string()));
    assert_eq!(target.get_internal_map().len(), 1);
}

#[test]
fn test_snapshot_after_clear() {
    let mut node = Node::new();

    // Add data
    node.apply_kv_command(&Node::encode_put_command("key1", "value1"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("key2", "value2"))
        .unwrap();

    // Clear via empty snapshot
    node.restore_from_snapshot(&[]).unwrap();
    assert_eq!(node.get_internal_map().len(), 0);

    // Create snapshot of empty state
    let snapshot = node.create_snapshot().expect("Failed to create snapshot");

    // Restore to another node
    let mut node2 = Node::new();
    node2
        .apply_kv_command(&Node::encode_put_command("temp", "data"))
        .unwrap();

    node2.restore_from_snapshot(&snapshot).unwrap();
    assert_eq!(node2.get_internal_map().len(), 0);
}

#[test]
fn test_snapshot_preserves_exact_state() {
    // Ensure snapshot captures exact state, not more, not less
    let mut node = Node::new();

    for i in 0..100 {
        let key = format!("k{:03}", i);
        let value = format!("v{}", i);
        node.apply_kv_command(&Node::encode_put_command(&key, &value))
            .unwrap();
    }

    let snapshot = node.create_snapshot().unwrap();

    let mut restored = Node::new();
    restored.restore_from_snapshot(&snapshot).unwrap();

    // Verify exact same keys
    assert_eq!(
        node.get_internal_map().keys().collect::<Vec<_>>(),
        restored.get_internal_map().keys().collect::<Vec<_>>()
    );

    // Verify exact same values
    for (k, v) in node.get_internal_map() {
        assert_eq!(restored.get_internal_map().get(k), Some(v));
    }
}

#[test]
fn test_snapshot_binary_data() {
    // Test with binary data (not valid UTF-8)
    let mut node = Node::new();

    // Note: Our KV store uses String, so we encode binary as hex
    let binary_data = vec![0xFF, 0xFE, 0xFD, 0x00, 0x01, 0x02];
    let hex_encoded = hex::encode(&binary_data);

    node.apply_kv_command(&Node::encode_put_command("binary", &hex_encoded))
        .unwrap();

    let snapshot = node.create_snapshot().unwrap();

    let mut restored = Node::new();
    restored.restore_from_snapshot(&snapshot).unwrap();

    assert_eq!(
        restored.get_internal_map().get("binary"),
        Some(&hex_encoded)
    );

    // Verify we can decode it back
    let decoded = hex::decode(restored.get_internal_map().get("binary").unwrap()).unwrap();
    assert_eq!(decoded, binary_data);
}

#[test]
fn test_snapshot_size_growth() {
    let mut node = Node::new();
    let mut previous_size = 0;

    // Verify snapshot size grows as we add data
    for i in 1..=5 {
        node.apply_kv_command(&Node::encode_put_command(
            &format!("key{}", i),
            &"x".repeat(100),
        ))
        .unwrap();

        let snapshot = node.create_snapshot().unwrap();
        let current_size = snapshot.len();

        assert!(
            current_size > previous_size,
            "Snapshot size should grow as data is added"
        );
        previous_size = current_size;
    }
}

#[test]
fn test_snapshot_with_duplicate_keys() {
    // Verify that updates (duplicate keys) are handled correctly
    let mut node = Node::new();

    // Insert key1 multiple times with different values
    node.apply_kv_command(&Node::encode_put_command("key1", "value1"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("key1", "value2"))
        .unwrap();
    node.apply_kv_command(&Node::encode_put_command("key1", "value3"))
        .unwrap();

    let snapshot = node.create_snapshot().unwrap();

    let mut restored = Node::new();
    restored.restore_from_snapshot(&snapshot).unwrap();

    // Should only have latest value
    assert_eq!(restored.get_internal_map().len(), 1);
    assert_eq!(
        restored.get_internal_map().get("key1"),
        Some(&"value3".to_string())
    );
}

// Helper module for hex encoding test
mod hex {
    pub fn encode(data: &[u8]) -> String {
        data.iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, std::num::ParseIntError> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
            .collect()
    }
}
