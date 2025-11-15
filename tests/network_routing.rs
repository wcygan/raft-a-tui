use std::collections::HashMap;

use crossbeam_channel::bounded;
use raft::prelude::Message;
use raft_a_tui::network::{LocalTransport, Transport, TransportError};

#[test]
fn test_send_message_to_peer() {
    let (tx, rx) = bounded(100);
    let mut peers = HashMap::new();
    peers.insert(2, tx);

    let transport = LocalTransport::new(1, peers);

    // Create a minimal Raft message
    let msg = Message::default();

    // Send should succeed
    assert!(transport.send(2, msg.clone()).is_ok());

    // Receiver should get the message
    let received = rx.recv().unwrap();
    assert_eq!(received, msg);
}

#[test]
fn test_peer_not_found() {
    let peers = HashMap::new();
    let transport = LocalTransport::new(1, peers);

    let msg = Message::default();

    // Sending to non-existent peer should fail
    let result = transport.send(999, msg);
    assert_eq!(result, Err(TransportError::PeerNotFound(999)));
}

#[test]
fn test_channel_full() {
    let (tx, _rx) = bounded(2); // Small buffer
    let mut peers = HashMap::new();
    peers.insert(2, tx);

    let transport = LocalTransport::new(1, peers);
    let msg = Message::default();

    // Fill the channel
    assert!(transport.send(2, msg.clone()).is_ok());
    assert!(transport.send(2, msg.clone()).is_ok());

    // Third message should fail with ChannelFull
    let result = transport.send(2, msg);
    assert_eq!(result, Err(TransportError::ChannelFull(2)));
}

#[test]
fn test_channel_closed() {
    let (tx, rx) = bounded(100);
    let mut peers = HashMap::new();
    peers.insert(2, tx);

    let transport = LocalTransport::new(1, peers);

    // Drop the receiver to close the channel
    drop(rx);

    let msg = Message::default();

    // Sending should fail with ChannelClosed
    let result = transport.send(2, msg);
    assert_eq!(result, Err(TransportError::ChannelClosed(2)));
}

#[test]
fn test_three_node_routing() {
    // Create channels for 3 nodes
    let (tx1, rx1) = bounded(100);
    let (tx2, rx2) = bounded(100);
    let (tx3, rx3) = bounded(100);

    // Node 1 can send to nodes 2 and 3
    let mut peers1 = HashMap::new();
    peers1.insert(2, tx2.clone());
    peers1.insert(3, tx3.clone());
    let transport1 = LocalTransport::new(1, peers1);

    // Node 2 can send to nodes 1 and 3
    let mut peers2 = HashMap::new();
    peers2.insert(1, tx1.clone());
    peers2.insert(3, tx3.clone());
    let transport2 = LocalTransport::new(2, peers2);

    // Node 3 can send to nodes 1 and 2
    let mut peers3 = HashMap::new();
    peers3.insert(1, tx1.clone());
    peers3.insert(2, tx2.clone());
    let _transport3 = LocalTransport::new(3, peers3);

    // Node 1 sends to nodes 2 and 3
    let mut msg = Message::default();
    msg.from = 1;
    msg.to = 2;
    assert!(transport1.send(2, msg.clone()).is_ok());

    msg.to = 3;
    assert!(transport1.send(3, msg).is_ok());

    // Node 2 receives message from node 1
    let received = rx2.recv().unwrap();
    assert_eq!(received.from, 1);
    assert_eq!(received.to, 2);

    // Node 3 receives message from node 1
    let received = rx3.recv().unwrap();
    assert_eq!(received.from, 1);
    assert_eq!(received.to, 3);

    // Node 2 sends back to node 1
    let mut msg = Message::default();
    msg.from = 2;
    msg.to = 1;
    assert!(transport2.send(1, msg).is_ok());

    let received = rx1.recv().unwrap();
    assert_eq!(received.from, 2);
    assert_eq!(received.to, 1);
}

#[test]
fn test_node_id() {
    let transport = LocalTransport::new(42, HashMap::new());
    assert_eq!(transport.node_id(), 42);
}

#[test]
fn test_peer_count() {
    let (tx1, _rx1) = bounded(100);
    let (tx2, _rx2) = bounded(100);

    let mut peers = HashMap::new();
    peers.insert(2, tx1);
    peers.insert(3, tx2);

    let transport = LocalTransport::new(1, peers);
    assert_eq!(transport.peer_count(), 2);
}

#[test]
fn test_has_peer() {
    let (tx, _rx) = bounded(100);
    let mut peers = HashMap::new();
    peers.insert(2, tx);

    let transport = LocalTransport::new(1, peers);

    assert!(transport.has_peer(2));
    assert!(!transport.has_peer(3));
    assert!(!transport.has_peer(1)); // Can't send to self
}

#[test]
fn test_transport_error_display() {
    let err1 = TransportError::PeerNotFound(5);
    assert_eq!(err1.to_string(), "Peer 5 not found in transport");

    let err2 = TransportError::ChannelFull(10);
    assert_eq!(
        err2.to_string(),
        "Channel to peer 10 is full (message dropped)"
    );

    let err3 = TransportError::ChannelClosed(15);
    assert_eq!(err3.to_string(), "Channel to peer 15 is closed");
}
