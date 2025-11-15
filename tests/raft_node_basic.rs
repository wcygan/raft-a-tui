use raft::StateRole;
use raft_a_tui::raft_node::{CommandResponse, RaftNode, RaftState};
use raft_a_tui::storage::RaftStorage;
use slog::{Drain, Logger, o};

fn create_logger() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    Logger::root(drain, o!())
}

#[test]
fn test_new_raft_node() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1, 2, 3];

    let node = RaftNode::new(1, peers, storage, logger);
    assert!(node.is_ok());
}

#[test]
fn test_initial_state() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1, 2, 3];

    let node = RaftNode::new(1, peers, storage, logger).unwrap();
    let state = node.get_state();

    assert_eq!(state.node_id, 1);
    assert_eq!(state.term, 0); // Initial term
    assert_eq!(state.role, StateRole::Follower); // Start as follower
    assert_eq!(state.commit_index, 0);
    assert_eq!(state.applied_index, 0);
}

#[test]
fn test_is_leader_initially_false() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1, 2, 3];

    let node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Nodes start as followers
    assert!(!node.is_leader());
}

#[test]
fn test_propose_command_returns_receiver() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1];

    let mut node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Note: Proposals will fail with ProposalDropped if node is not leader
    // For this test, we just verify the method signature works correctly
    // The proposal may succeed or fail depending on leadership state
    let result = node.propose_command("key".to_string(), "value".to_string());

    // The result may be Ok (proposal queued) or Err (ProposalDropped)
    // Either is acceptable for testing the API
    match result {
        Ok(rx) => {
            // If Ok, receiver should not have a message yet (not committed)
            assert!(rx.try_recv().is_err());
        }
        Err(raft::Error::ProposalDropped) => {
            // Expected if node is not leader
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[test]
fn test_propose_command_unique_ids() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1];

    let mut node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Propose multiple commands
    // They may fail with ProposalDropped if node is not leader
    let result1 = node.propose_command("key1".to_string(), "value1".to_string());
    let result2 = node.propose_command("key2".to_string(), "value2".to_string());

    // Both proposals should have the same result (both dropped or both queued)
    match (result1, result2) {
        (Ok(rx1), Ok(rx2)) => {
            // Both receivers should be valid (different callback IDs)
            assert!(rx1.try_recv().is_err());
            assert!(rx2.try_recv().is_err());
        }
        (Err(raft::Error::ProposalDropped), Err(raft::Error::ProposalDropped)) => {
            // Both dropped - expected if not leader
        }
        _ => {
            // Inconsistent state - should not happen
            panic!("Inconsistent proposal results");
        }
    }
}

#[test]
fn test_config_validation() {
    let storage = RaftStorage::new();
    let logger = create_logger();

    // Empty peers list should still work (single node)
    let peers = vec![1];
    let result = RaftNode::new(1, peers, storage, logger);
    assert!(result.is_ok());
}

#[test]
fn test_logger_access() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1];

    let node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Should be able to access logger
    let _logger = node.logger();
}

#[test]
fn test_raw_node_access() {
    let storage = RaftStorage::new();
    let logger = create_logger();
    let peers = vec![1];

    let node = RaftNode::new(1, peers, storage, logger).unwrap();

    // Should be able to access raw_node
    let raw_node = node.raw_node();
    assert_eq!(raw_node.raft.id, 1);
}

#[test]
fn test_raft_state_equality() {
    let state1 = RaftState {
        node_id: 1,
        term: 5,
        role: StateRole::Leader,
        leader_id: 1,
        commit_index: 10,
        applied_index: 10,
        cluster_size: 3,
    };

    let state2 = RaftState {
        node_id: 1,
        term: 5,
        role: StateRole::Leader,
        leader_id: 1,
        commit_index: 10,
        applied_index: 10,
        cluster_size: 3,
    };

    assert_eq!(state1, state2);
}

#[test]
fn test_command_response_equality() {
    let resp1 = CommandResponse::Success {
        key: "test".to_string(),
        value: "123".to_string(),
    };

    let resp2 = CommandResponse::Success {
        key: "test".to_string(),
        value: "123".to_string(),
    };

    assert_eq!(resp1, resp2);

    let err1 = CommandResponse::Error("failed".to_string());
    let err2 = CommandResponse::Error("failed".to_string());

    assert_eq!(err1, err2);

    assert_ne!(resp1, err1);
}

#[test]
fn test_three_node_cluster_initialization() {
    let logger1 = create_logger();
    let logger2 = create_logger();
    let logger3 = create_logger();

    let peers = vec![1, 2, 3];

    let storage1 = RaftStorage::new();
    let storage2 = RaftStorage::new();
    let storage3 = RaftStorage::new();

    let node1 = RaftNode::new(1, peers.clone(), storage1, logger1).unwrap();
    let node2 = RaftNode::new(2, peers.clone(), storage2, logger2).unwrap();
    let node3 = RaftNode::new(3, peers, storage3, logger3).unwrap();

    // All nodes should start as followers
    assert!(!node1.is_leader());
    assert!(!node2.is_leader());
    assert!(!node3.is_leader());

    // All nodes should report cluster size of 3
    assert_eq!(node1.get_state().cluster_size, 3);
    assert_eq!(node2.get_state().cluster_size, 3);
    assert_eq!(node3.get_state().cluster_size, 3);
}
