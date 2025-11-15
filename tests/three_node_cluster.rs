use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use raft::prelude::Message;
use raft_a_tui::commands::UserCommand;
use raft_a_tui::network::LocalTransport;
use raft_a_tui::node::Node;
use raft_a_tui::raft_loop::{raft_ready_loop, StateUpdate};
use raft_a_tui::raft_node::{RaftNode, RaftState};
use raft_a_tui::storage::RaftStorage;
use slog::{o, Drain};

/// Helper to create a test logger that writes to stdout with minimal formatting
fn test_logger(node_id: u64) -> slog::Logger {
    let decorator = slog_term::PlainDecorator::new(std::io::stdout());
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    slog::Logger::root(drain, o!("node_id" => node_id))
}

/// A running Raft node with all its channels.
struct TestNode {
    node_id: u64,
    cmd_tx: Sender<UserCommand>,
    state_rx: Receiver<StateUpdate>,
    shutdown_tx: Sender<()>,
    handle: thread::JoinHandle<Result<(), raft_a_tui::raft_loop::RaftLoopError>>,
}

impl TestNode {
    /// Send a command to this node.
    fn send_command(&self, cmd: UserCommand) {
        self.cmd_tx.send(cmd).unwrap();
    }

    /// Get the latest Raft state (non-blocking).
    fn get_raft_state(&self) -> Option<RaftState> {
        // Drain all state updates and return the latest RaftState
        let mut latest_state = None;

        while let Ok(update) = self.state_rx.try_recv() {
            if let StateUpdate::RaftState(state) = update {
                latest_state = Some(state);
            }
        }

        latest_state
    }

    /// Wait for this node to have a specific key-value pair.
    fn has_kv_update(&self, expected_key: &str, expected_value: &str) -> bool {
        // Drain all state updates and check for matching KvUpdate
        while let Ok(update) = self.state_rx.try_recv() {
            if let StateUpdate::KvUpdate { key, value } = update {
                if key == expected_key && value == expected_value {
                    return true;
                }
            }
        }

        false
    }

    /// Shutdown this node gracefully.
    fn shutdown(self) {
        self.shutdown_tx.send(()).ok();
        self.handle.join().ok();
    }
}

/// Setup a 3-node Raft cluster using LocalTransport (in-memory).
fn setup_three_node_cluster() -> (TestNode, TestNode, TestNode) {
    let node_ids = vec![1u64, 2u64, 3u64];

    // Create message channels for each node
    let (msg_tx1, msg_rx1) = unbounded();
    let (msg_tx2, msg_rx2) = unbounded();
    let (msg_tx3, msg_rx3) = unbounded();

    // Create a full mesh of message senders
    // Each node can send to every other node (including itself for testing)
    let mut senders1 = HashMap::new();
    senders1.insert(1, msg_tx1.clone());
    senders1.insert(2, msg_tx2.clone());
    senders1.insert(3, msg_tx3.clone());

    let mut senders2 = HashMap::new();
    senders2.insert(1, msg_tx1.clone());
    senders2.insert(2, msg_tx2.clone());
    senders2.insert(3, msg_tx3.clone());

    let mut senders3 = HashMap::new();
    senders3.insert(1, msg_tx1);
    senders3.insert(2, msg_tx2);
    senders3.insert(3, msg_tx3);

    // Create nodes
    let node1 = create_node(1, node_ids.clone(), senders1, msg_rx1);
    let node2 = create_node(2, node_ids.clone(), senders2, msg_rx2);
    let node3 = create_node(3, node_ids, senders3, msg_rx3);

    (node1, node2, node3)
}

/// Create a single Raft node with all necessary components.
fn create_node(
    node_id: u64,
    peer_ids: Vec<u64>,
    senders: HashMap<u64, Sender<Message>>,
    msg_rx: Receiver<Message>,
) -> TestNode {
    // Create components
    let storage = RaftStorage::new();
    let logger = test_logger(node_id);
    let raft_node = RaftNode::new(node_id, peer_ids, storage, logger.clone()).unwrap();
    let kv_node = Node::new();

    // Create channels
    let (cmd_tx, cmd_rx) = unbounded();
    let (state_tx, state_rx) = unbounded();
    let (shutdown_tx, shutdown_rx) = unbounded();

    // Create transport
    let transport = LocalTransport::new(node_id, senders);

    // Spawn ready loop
    let handle = thread::Builder::new()
        .name(format!("raft-node-{}", node_id))
        .spawn(move || {
            raft_ready_loop(
                raft_node,
                kv_node,
                cmd_rx,
                msg_rx,
                state_tx,
                transport,
                shutdown_rx,
            )
        })
        .unwrap();

    TestNode {
        node_id,
        cmd_tx,
        state_rx,
        shutdown_tx,
        handle,
    }
}

/// Wait for a leader to be elected (with timeout).
fn wait_for_leader(nodes: &[&TestNode], timeout: Duration) -> Option<u64> {
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        // Check each node's state
        for node in nodes {
            if let Some(state) = node.get_raft_state() {
                if state.leader_id != 0 && state.leader_id != raft::INVALID_ID {
                    println!(
                        "Leader elected: node {} (term {})",
                        state.leader_id, state.term
                    );
                    return Some(state.leader_id);
                }
            }
        }

        thread::sleep(Duration::from_millis(50));
    }

    None
}

/// Wait for all nodes to have a specific key-value pair (with timeout).
fn wait_for_replication(
    nodes: &[&TestNode],
    key: &str,
    value: &str,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        let mut all_have_it = true;

        for node in nodes {
            if !node.has_kv_update(key, value) {
                all_have_it = false;
                break;
            }
        }

        if all_have_it {
            println!("All nodes have {} = {}", key, value);
            return true;
        }

        thread::sleep(Duration::from_millis(50));
    }

    false
}

#[test]
#[ignore] // This test takes a while and involves threading - run with --ignored
fn test_three_node_consensus_full() {
    println!("\n=== Starting 3-Node Consensus Test ===\n");

    // 1. Setup 3-node cluster
    println!("Setting up 3-node cluster...");
    let (node1, node2, node3) = setup_three_node_cluster();

    // Give nodes time to initialize
    thread::sleep(Duration::from_millis(200));

    // 2. Wait for leader election
    println!("Waiting for leader election...");
    let leader_id = wait_for_leader(&[&node1, &node2, &node3], Duration::from_secs(5));

    assert!(
        leader_id.is_some(),
        "A leader should be elected within 5 seconds"
    );

    let leader_id = leader_id.unwrap();
    println!("Leader elected: node {}", leader_id);

    // 3. Find the leader node
    let leader_node = match leader_id {
        1 => &node1,
        2 => &node2,
        3 => &node3,
        _ => panic!("Invalid leader ID: {}", leader_id),
    };

    // 4. Submit PUT command to leader
    println!("\nSubmitting PUT command to leader (node {})...", leader_id);
    leader_node.send_command(UserCommand::Put {
        key: "key1".to_string(),
        value: "value1".to_string(),
    });

    // 5. Wait for replication to all nodes
    println!("Waiting for replication...");
    let replicated = wait_for_replication(
        &[&node1, &node2, &node3],
        "key1",
        "value1",
        Duration::from_secs(3),
    );

    assert!(
        replicated,
        "key1=value1 should replicate to all nodes within 3 seconds"
    );

    // 6. Submit multiple commands
    println!("\nSubmitting multiple PUT commands...");
    for i in 2..=5 {
        leader_node.send_command(UserCommand::Put {
            key: format!("key{}", i),
            value: format!("value{}", i),
        });
        thread::sleep(Duration::from_millis(100));
    }

    // 7. Wait for all to replicate
    println!("Waiting for all commands to replicate...");
    for i in 2..=5 {
        let replicated = wait_for_replication(
            &[&node1, &node2, &node3],
            &format!("key{}", i),
            &format!("value{}", i),
            Duration::from_secs(3),
        );

        assert!(
            replicated,
            "key{}=value{} should replicate to all nodes",
            i, i
        );
    }

    println!("\n=== All commands replicated successfully! ===\n");

    // 8. Shutdown all nodes
    println!("Shutting down nodes...");
    node1.shutdown();
    node2.shutdown();
    node3.shutdown();

    println!("=== Test complete ===\n");
}

#[test]
fn test_three_node_cluster_basic_setup() {
    // This is a simpler test that just verifies nodes start up
    println!("\n=== Basic 3-Node Setup Test ===\n");

    println!("Setting up 3-node cluster...");
    let (node1, node2, node3) = setup_three_node_cluster();

    // Give nodes time to initialize
    thread::sleep(Duration::from_millis(500));

    println!("Checking node states...");

    // All nodes should have some state
    let state1 = node1.get_raft_state();
    let state2 = node2.get_raft_state();
    let state3 = node3.get_raft_state();

    // We may or may not have states yet depending on timing
    println!("Node 1 state: {:?}", state1);
    println!("Node 2 state: {:?}", state2);
    println!("Node 3 state: {:?}", state3);

    // Shutdown all nodes
    println!("Shutting down nodes...");
    node1.shutdown();
    node2.shutdown();
    node3.shutdown();

    println!("=== Basic setup test complete ===\n");
}

#[test]
fn test_follower_campaign_becomes_leader() {
    println!("\n=== Follower Campaign Test ===\n");

    // 1. Setup 3-node cluster
    println!("Setting up 3-node cluster...");
    let (node1, node2, node3) = setup_three_node_cluster();

    // Give nodes time to initialize
    thread::sleep(Duration::from_millis(200));

    // 2. Wait for initial leader election
    println!("Waiting for initial leader election...");
    let initial_leader_id =
        wait_for_leader(&[&node1, &node2, &node3], Duration::from_secs(5));

    assert!(
        initial_leader_id.is_some(),
        "Initial leader should be elected within 5 seconds"
    );

    let initial_leader_id = initial_leader_id.unwrap();
    println!("Initial leader: node {}", initial_leader_id);

    // 3. Find a follower node to campaign
    let (follower_node, follower_id) = match initial_leader_id {
        1 => (&node2, 2u64), // If node 1 is leader, use node 2 as follower
        2 => (&node1, 1u64), // If node 2 is leader, use node 1 as follower
        3 => (&node1, 1u64), // If node 3 is leader, use node 1 as follower
        _ => panic!("Invalid leader ID: {}", initial_leader_id),
    };

    println!(
        "\nCampaigning with follower node {}...",
        follower_id
    );

    // 4. Call CAMPAIGN on the follower
    follower_node.send_command(UserCommand::Campaign);

    // Give time for election to complete
    thread::sleep(Duration::from_millis(500));

    // 5. Wait for leadership to change
    println!("Waiting for new leader election...");
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(5);
    let mut new_leader_id = None;

    while start.elapsed() < timeout {
        // Check if follower became leader
        if let Some(state) = follower_node.get_raft_state() {
            if state.leader_id == follower_id {
                new_leader_id = Some(follower_id);
                println!(
                    "Node {} successfully became leader at term {}!",
                    follower_id, state.term
                );
                break;
            }
        }

        thread::sleep(Duration::from_millis(50));
    }

    assert!(
        new_leader_id.is_some(),
        "Follower node {} should become leader after campaigning",
        follower_id
    );

    assert_eq!(
        new_leader_id.unwrap(),
        follower_id,
        "The campaigning follower should be the new leader"
    );

    // 6. Verify all nodes agree on the new leader
    thread::sleep(Duration::from_millis(200));

    let mut all_agree = true;
    for (i, node) in [&node1, &node2, &node3].iter().enumerate() {
        if let Some(state) = node.get_raft_state() {
            println!(
                "Node {} sees leader as node {} at term {}",
                i + 1,
                state.leader_id,
                state.term
            );

            if state.leader_id != follower_id {
                all_agree = false;
            }
        }
    }

    assert!(
        all_agree,
        "All nodes should agree on the new leader (node {})",
        follower_id
    );

    println!("\n=== Campaign test complete - follower successfully became leader! ===\n");

    // Shutdown all nodes
    println!("Shutting down nodes...");
    node1.shutdown();
    node2.shutdown();
    node3.shutdown();
}
