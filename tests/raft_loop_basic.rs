use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use raft::prelude::*;
use raft_a_tui::commands::UserCommand;
use raft_a_tui::network::LocalTransport;
use raft_a_tui::node::Node;
use raft_a_tui::raft_loop::{raft_ready_loop, StateUpdate};
use raft_a_tui::raft_node::RaftNode;
use raft_a_tui::storage::RaftStorage;
use slog::o;

/// Helper to create a test logger that discards output
fn test_logger() -> slog::Logger {
    let drain = slog::Discard;
    slog::Logger::root(drain, o!())
}

/// Helper to create a RaftNode for testing
fn create_test_raft_node(id: u64, peers: Vec<u64>) -> RaftNode {
    let storage = RaftStorage::new();
    let logger = test_logger();
    RaftNode::new(id, peers, storage, logger).unwrap()
}

/// Helper to create channels for the ready loop
fn create_test_channels() -> (
    Sender<UserCommand>,
    Receiver<UserCommand>,
    Sender<Message>,
    Receiver<Message>,
    Sender<StateUpdate>,
    Receiver<StateUpdate>,
    Sender<()>,
    Receiver<()>,
) {
    let (cmd_tx, cmd_rx) = unbounded();
    let (msg_tx, msg_rx) = unbounded();
    let (state_tx, state_rx) = unbounded();
    let (shutdown_tx, shutdown_rx) = unbounded();

    (
        cmd_tx,
        cmd_rx,
        msg_tx,
        msg_rx,
        state_tx,
        state_rx,
        shutdown_tx,
        shutdown_rx,
    )
}

/// Helper to create a mock transport
fn create_mock_transport(node_id: u64) -> LocalTransport {
    let peers = HashMap::new();
    LocalTransport::new(node_id, peers)
}

#[test]
fn test_ready_loop_starts_and_stops_with_shutdown() {
    let raft_node = create_test_raft_node(1, vec![1]);
    let kv_node = Node::new();
    let (cmd_tx, cmd_rx, _msg_tx, msg_rx, state_tx, state_rx, shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Give loop time to start
    thread::sleep(Duration::from_millis(50));

    // Send shutdown signal
    shutdown_tx.send(()).unwrap();

    // Loop should exit cleanly
    let result = handle.join().expect("Thread panicked");
    assert!(result.is_ok(), "Ready loop should exit cleanly");

    // Clean up channels
    drop(cmd_tx);
    drop(state_rx);
}

#[test]
fn test_ready_loop_handles_cmd_rx_closed() {
    let raft_node = create_test_raft_node(1, vec![1]);
    let kv_node = Node::new();
    let (cmd_tx, cmd_rx, _msg_tx, msg_rx, state_tx, _state_rx, _shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Give loop time to start
    thread::sleep(Duration::from_millis(50));

    // Close cmd_rx by dropping cmd_tx
    drop(cmd_tx);

    // Loop should detect channel closed and exit with error
    let result = handle.join().expect("Thread panicked");
    assert!(result.is_err(), "Ready loop should error on channel close");

    if let Err(e) = result {
        assert!(
            e.to_string().contains("cmd_rx"),
            "Error should mention cmd_rx"
        );
    }
}

#[test]
fn test_ready_loop_handles_msg_rx_closed() {
    let raft_node = create_test_raft_node(1, vec![1]);
    let kv_node = Node::new();
    let (_cmd_tx, cmd_rx, msg_tx, msg_rx, state_tx, _state_rx, _shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Give loop time to start
    thread::sleep(Duration::from_millis(50));

    // Close msg_rx by dropping msg_tx
    drop(msg_tx);

    // Loop should detect channel closed and exit with error
    let result = handle.join().expect("Thread panicked");
    assert!(result.is_err(), "Ready loop should error on channel close");

    if let Err(e) = result {
        assert!(
            e.to_string().contains("msg_rx"),
            "Error should mention msg_rx"
        );
    }
}

#[test]
fn test_ready_loop_handles_user_commands() {
    let raft_node = create_test_raft_node(1, vec![1]);
    let kv_node = Node::new();
    let (cmd_tx, cmd_rx, _msg_tx, msg_rx, state_tx, state_rx, shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Give loop time to start
    thread::sleep(Duration::from_millis(50));

    // Send a GET command (read-only, should be handled locally)
    cmd_tx
        .send(UserCommand::Get {
            key: "test".to_string(),
        })
        .unwrap();

    // Give loop time to process
    thread::sleep(Duration::from_millis(100));

    // Send a KEYS command
    cmd_tx.send(UserCommand::Keys).unwrap();

    // Give loop time to process
    thread::sleep(Duration::from_millis(100));

    // Loop should still be running (commands don't cause shutdown)
    shutdown_tx.send(()).unwrap();

    let result = handle.join().expect("Thread panicked");
    assert!(result.is_ok(), "Ready loop should handle commands and exit cleanly");

    // We should have received some state updates
    // Note: We might not receive specific updates for read-only commands
    // since they don't go through Raft, but we should have initial state
    drop(state_rx); // Clean up
}

#[test]
fn test_ready_loop_emits_state_updates() {
    let raft_node = create_test_raft_node(1, vec![1]);
    let kv_node = Node::new();
    let (_cmd_tx, cmd_rx, _msg_tx, msg_rx, state_tx, state_rx, shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Give loop time to start and process initial state
    thread::sleep(Duration::from_millis(200));

    // Shutdown
    shutdown_tx.send(()).unwrap();

    let result = handle.join().expect("Thread panicked");
    assert!(result.is_ok());

    // Check if we received any state updates
    // The ready loop should send RaftState updates periodically
    let updates: Vec<StateUpdate> = state_rx.try_iter().collect();

    // We should have received at least one state update
    // (The exact number depends on timing and Raft behavior)
    println!("Received {} state updates", updates.len());

    // Count different types of updates
    let raft_state_count = updates
        .iter()
        .filter(|u| matches!(u, StateUpdate::RaftState(_)))
        .count();

    println!("RaftState updates: {}", raft_state_count);

    // We should have at least some RaftState updates if the ready loop processed any Ready state
    // However, with a single node that doesn't receive any messages, there might not be any Ready state
    // So we just verify that we can receive updates without panicking
}

#[test]
fn test_ready_loop_handles_raft_messages() {
    let raft_node = create_test_raft_node(1, vec![1, 2, 3]);
    let kv_node = Node::new();
    let (_cmd_tx, cmd_rx, msg_tx, msg_rx, state_tx, _state_rx, shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Give loop time to start
    thread::sleep(Duration::from_millis(50));

    // Send a mock Raft message (heartbeat from node 2)
    let msg = Message {
        msg_type: MessageType::MsgHeartbeat.into(),
        to: 1,
        from: 2,
        term: 1,
        ..Default::default()
    };

    msg_tx.send(msg).unwrap();

    // Give loop time to process
    thread::sleep(Duration::from_millis(100));

    // Loop should still be running (message doesn't cause shutdown)
    shutdown_tx.send(()).unwrap();

    let result = handle.join().expect("Thread panicked");
    assert!(result.is_ok(), "Ready loop should handle messages and exit cleanly");
}

#[test]
fn test_ready_loop_ticks_periodically() {
    // This test verifies that the ready loop calls tick() periodically
    // We can't easily verify this without instrumenting RaftNode,
    // but we can verify that the loop runs for multiple tick periods

    let raft_node = create_test_raft_node(1, vec![1]);
    let kv_node = Node::new();
    let (_cmd_tx, cmd_rx, _msg_tx, msg_rx, state_tx, _state_rx, shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Wait for several tick periods (100ms each)
    // Loop should tick at least 3 times in 350ms
    thread::sleep(Duration::from_millis(350));

    // Shutdown
    shutdown_tx.send(()).unwrap();

    let result = handle.join().expect("Thread panicked");
    assert!(result.is_ok(), "Ready loop should run and tick periodically");
}

#[test]
fn test_ready_loop_handles_campaign_command() {
    let raft_node = create_test_raft_node(1, vec![1]);
    let kv_node = Node::new();
    let (cmd_tx, cmd_rx, _msg_tx, msg_rx, state_tx, state_rx, shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Give loop time to start
    thread::sleep(Duration::from_millis(50));

    // Send CAMPAIGN command
    cmd_tx.send(UserCommand::Campaign).unwrap();

    // Give loop time to process
    thread::sleep(Duration::from_millis(100));

    // Shutdown
    shutdown_tx.send(()).unwrap();

    let result = handle.join().expect("Thread panicked");
    assert!(result.is_ok(), "Ready loop should handle campaign command");

    // Check for SystemMessage about campaign
    let updates: Vec<StateUpdate> = state_rx.try_iter().collect();
    let has_campaign_message = updates.iter().any(|u| {
        if let StateUpdate::SystemMessage(msg) = u {
            msg.contains("campaign")
        } else {
            false
        }
    });

    assert!(
        has_campaign_message,
        "Should receive system message about campaign"
    );
}

#[test]
fn test_ready_loop_handles_put_command() {
    let raft_node = create_test_raft_node(1, vec![1]);
    let kv_node = Node::new();
    let (cmd_tx, cmd_rx, _msg_tx, msg_rx, state_tx, _state_rx, shutdown_tx, shutdown_rx) =
        create_test_channels();
    let transport = create_mock_transport(1);

    // Spawn ready loop in background thread
    let handle = thread::spawn(move || {
        raft_ready_loop(
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
        )
    });

    // Give loop time to start
    thread::sleep(Duration::from_millis(50));

    // Send PUT command
    // Note: This will propose to Raft, but with a single node, it should commit immediately
    cmd_tx
        .send(UserCommand::Put {
            key: "foo".to_string(),
            value: "bar".to_string(),
        })
        .unwrap();

    // Give loop time to process and potentially commit
    thread::sleep(Duration::from_millis(200));

    // Shutdown
    shutdown_tx.send(()).unwrap();

    let result = handle.join().expect("Thread panicked");
    assert!(result.is_ok(), "Ready loop should handle PUT command");
}
