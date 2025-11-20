use std::collections::HashMap;

use crossbeam_channel::{Sender, TrySendError};
use raft::prelude::Message;

/// Error types for transport operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The target peer ID is not registered in the transport.
    PeerNotFound(u64),
    /// The channel to the peer is full (bounded channel limit reached).
    ChannelFull(u64),
    /// The channel to the peer has been closed (peer disconnected).
    ChannelClosed(u64),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::PeerNotFound(id) => write!(f, "Peer {} not found in transport", id),
            TransportError::ChannelFull(id) => {
                write!(f, "Channel to peer {} is full (message dropped)", id)
            }
            TransportError::ChannelClosed(id) => write!(f, "Channel to peer {} is closed", id),
        }
    }
}

impl std::error::Error for TransportError {}

/// Transport abstraction for sending Raft messages between nodes.
///
/// Implementations can use local channels (for testing), TCP, UDP, etc.
pub trait Transport {
    /// Send a Raft message to the specified peer node.
    ///
    /// Returns an error if the peer is not found, the channel is full,
    /// or the connection is closed.
    fn send(&self, to: u64, msg: Message) -> Result<(), TransportError>;
}

/// Local in-memory transport using crossbeam channels.
///
/// Uses bounded channels to prevent memory issues with slow consumers.
/// Messages are dropped (with error) if the channel is full.
pub struct LocalTransport {
    node_id: u64,
    peers: HashMap<u64, Sender<Message>>,
}

impl LocalTransport {
    /// Create a new LocalTransport for the given node.
    ///
    /// # Arguments
    /// * `node_id` - The ID of this node (used for logging/debugging)
    /// * `peers` - Map of peer IDs to their message channels
    ///
    /// # Example
    /// ```
    /// use std::collections::HashMap;
    /// use crossbeam_channel::bounded;
    /// use raft_core::network::LocalTransport;
    /// use raft::prelude::Message;
    ///
    /// let (tx1, _rx1) = bounded(100);
    /// let (tx2, _rx2) = bounded(100);
    ///
    /// let mut peers = HashMap::new();
    /// peers.insert(2, tx1);
    /// peers.insert(3, tx2);
    ///
    /// let transport = LocalTransport::new(1, peers);
    /// ```
    pub fn new(node_id: u64, peers: HashMap<u64, Sender<Message>>) -> Self {
        Self { node_id, peers }
    }

    /// Get this node's ID.
    pub fn node_id(&self) -> u64 {
        self.node_id
    }

    /// Get the number of registered peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Check if a peer is registered.
    pub fn has_peer(&self, peer_id: u64) -> bool {
        self.peers.contains_key(&peer_id)
    }
}

impl Transport for LocalTransport {
    fn send(&self, to: u64, mut msg: Message) -> Result<(), TransportError> {
        // raft-rs generates messages with from=0, transport layer must fill in sender ID
        msg.from = self.node_id;

        let sender = self
            .peers
            .get(&to)
            .ok_or(TransportError::PeerNotFound(to))?;

        sender.try_send(msg).map_err(|e| match e {
            TrySendError::Full(_) => TransportError::ChannelFull(to),
            TrySendError::Disconnected(_) => TransportError::ChannelClosed(to),
        })
    }
}
