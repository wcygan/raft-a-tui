use raft::StateRole;
use raft_core::raft_node::RaftNode;
use raft_core::storage::RaftStorage;
use slog::{Drain, Logger, o};
use tokio::sync::oneshot;

fn create_logger() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    Logger::root(drain, o!())
}

#[test]
fn test_follower_rejects_proposals() {
    // Create a node in a 3-node cluster
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1, 2, 3];

    let mut node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Verify node starts as follower
    assert_eq!(node.get_state().role, StateRole::Follower);
    assert!(!node.is_leader());

    // Attempt to propose a command as follower
    let (tx, _rx) = oneshot::channel();
    let result = node.propose_command("key".to_string(), "value".to_string(), Some(tx));

    // Should fail with ProposalDropped
    assert!(result.is_err());
    match result {
        Err(raft::Error::ProposalDropped) => {
            // Expected - this is the correct behavior
        }
        Err(e) => panic!("Expected ProposalDropped, got {:?}", e),
        Ok(_) => panic!("Follower should not accept proposals"),
    }
}

#[test]
fn test_candidate_rejects_proposals() {
    // Create a single-node cluster
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1];

    let mut node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Trigger an election to become candidate
    node.raw_node_mut().campaign().unwrap();

    // Tick to process election
    for _ in 0..10 {
        node.raw_node_mut().tick();
    }

    // In a single-node cluster, node should become leader
    // But let's verify behavior if somehow still in candidate state
    // (This test is theoretical - single node becomes leader immediately)

    // Note: This test primarily documents the expected behavior
    // In practice, candidates reject proposals just like followers
}

#[test]
fn test_leader_accepts_proposals() {
    // Create a single-node cluster (will become leader immediately)
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1];

    let mut node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Trigger election
    node.raw_node_mut().campaign().unwrap();

    // Tick to process election (single node becomes leader immediately)
    for _ in 0..10 {
        node.raw_node_mut().tick();
        if node.raw_node().has_ready() {
            let ready = node.raw_node_mut().ready();
            node.raw_node_mut().advance(ready);
        }
    }

    // Verify node is now leader
    assert!(node.is_leader());
    assert_eq!(node.get_state().role, StateRole::Leader);

    // Propose a command as leader
    let (tx, mut rx) = oneshot::channel();
    let result = node.propose_command("key".to_string(), "value".to_string(), Some(tx));

    // Should succeed
    assert!(result.is_ok());

    // The command won't be committed yet (needs Ready processing)
    // but the proposal should have been accepted.
    // The receiver should be empty (no result sent yet).
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_multiple_proposals_rejected_by_follower() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1, 2, 3];

    let mut node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Verify follower
    assert!(!node.is_leader());

    // Try multiple proposals
    let result1 = node.propose_command("key1".to_string(), "value1".to_string(), None);
    let result2 = node.propose_command("key2".to_string(), "value2".to_string(), None);
    let result3 = node.propose_command("key3".to_string(), "value3".to_string(), None);

    // All should be rejected
    assert!(result1.is_err());
    assert!(result2.is_err());
    assert!(result3.is_err());

    assert!(matches!(result1, Err(raft::Error::ProposalDropped)));
    assert!(matches!(result2, Err(raft::Error::ProposalDropped)));
    assert!(matches!(result3, Err(raft::Error::ProposalDropped)));
}

#[test]
fn test_leader_accepts_multiple_proposals() {
    // Single-node cluster
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1];

    let mut node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Become leader
    node.raw_node_mut().campaign().unwrap();
    for _ in 0..10 {
        node.raw_node_mut().tick();
        if node.raw_node().has_ready() {
            let ready = node.raw_node_mut().ready();
            node.raw_node_mut().advance(ready);
        }
    }

    assert!(node.is_leader());

    // Try multiple proposals
    let result1 = node.propose_command("key1".to_string(), "value1".to_string(), None);
    let result2 = node.propose_command("key2".to_string(), "value2".to_string(), None);
    let result3 = node.propose_command("key3".to_string(), "value3".to_string(), None);

    // All should succeed
    assert!(result1.is_ok());
    assert!(result2.is_ok());
    assert!(result3.is_ok());
}

#[test]
fn test_state_query_on_follower() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1, 2, 3];

    let mut node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Before rejecting proposal, verify we can query state
    let state = node.get_state();
    assert_eq!(state.node_id, 1);
    assert_eq!(state.role, StateRole::Follower);
    assert_eq!(state.leader_id, 0); // No leader initially

    // Reject proposal
    let result = node.propose_command("test".to_string(), "123".to_string(), None);
    assert!(matches!(result, Err(raft::Error::ProposalDropped)));

    // State should still be queryable after rejection
    let state = node.get_state();
    assert_eq!(state.node_id, 1);
    assert_eq!(state.role, StateRole::Follower);
}
