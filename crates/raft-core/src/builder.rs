use crate::{
    commands::ServerCommand,
    network::Transport,
    node::Node,
    raft_loop::{RaftDriver, StateUpdate},
    raft_node::RaftNode,
};
use crossbeam_channel::{Receiver, Sender};
use std::fmt;

/// Error type for RaftDriverBuilder
#[derive(Debug)]
pub enum BuilderError {
    MissingRaftNode,
    MissingKvNode,
    MissingCmdRx,
    MissingMsgRx,
    MissingStateTx,
    MissingTransport,
    MissingShutdownRx,
}

impl fmt::Display for BuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuilderError::MissingRaftNode => write!(f, "RaftNode is required"),
            BuilderError::MissingKvNode => write!(f, "KvNode is required"),
            BuilderError::MissingCmdRx => write!(f, "Command receiver is required"),
            BuilderError::MissingMsgRx => write!(f, "Message receiver is required"),
            BuilderError::MissingStateTx => write!(f, "State sender is required"),
            BuilderError::MissingTransport => write!(f, "Transport is required"),
            BuilderError::MissingShutdownRx => write!(f, "Shutdown receiver is required"),
        }
    }
}

impl std::error::Error for BuilderError {}

/// Builder for constructing RaftDriver instances
pub struct RaftDriverBuilder<T: Transport> {
    raft_node: Option<RaftNode>,
    kv_node: Option<Node>,
    cmd_rx: Option<Receiver<ServerCommand>>,
    msg_rx: Option<Receiver<raft::eraftpb::Message>>,
    state_tx: Option<Sender<StateUpdate>>,
    transport: Option<T>,
    shutdown_rx: Option<Receiver<()>>,
}

impl<T: Transport> Default for RaftDriverBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Transport> RaftDriverBuilder<T> {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            raft_node: None,
            kv_node: None,
            cmd_rx: None,
            msg_rx: None,
            state_tx: None,
            transport: None,
            shutdown_rx: None,
        }
    }

    /// Set the RaftNode
    pub fn raft_node(mut self, node: RaftNode) -> Self {
        self.raft_node = Some(node);
        self
    }

    /// Set the KV Node
    pub fn kv_node(mut self, node: Node) -> Self {
        self.kv_node = Some(node);
        self
    }

    /// Set the command receiver
    pub fn cmd_rx(mut self, rx: Receiver<ServerCommand>) -> Self {
        self.cmd_rx = Some(rx);
        self
    }

    /// Set the message receiver
    pub fn msg_rx(mut self, rx: Receiver<raft::eraftpb::Message>) -> Self {
        self.msg_rx = Some(rx);
        self
    }

    /// Set the state sender
    pub fn state_tx(mut self, tx: Sender<StateUpdate>) -> Self {
        self.state_tx = Some(tx);
        self
    }

    /// Set the transport
    pub fn transport(mut self, transport: T) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Set the shutdown receiver
    pub fn shutdown_rx(mut self, rx: Receiver<()>) -> Self {
        self.shutdown_rx = Some(rx);
        self
    }

    /// Build the RaftDriver
    pub fn build(self) -> Result<RaftDriver<T>, BuilderError> {
        let raft_node = self.raft_node.ok_or(BuilderError::MissingRaftNode)?;
        let kv_node = self.kv_node.ok_or(BuilderError::MissingKvNode)?;
        let cmd_rx = self.cmd_rx.ok_or(BuilderError::MissingCmdRx)?;
        let msg_rx = self.msg_rx.ok_or(BuilderError::MissingMsgRx)?;
        let state_tx = self.state_tx.ok_or(BuilderError::MissingStateTx)?;
        let transport = self.transport.ok_or(BuilderError::MissingTransport)?;
        let shutdown_rx = self.shutdown_rx.ok_or(BuilderError::MissingShutdownRx)?;

        Ok(RaftDriver::from_parts(
            raft_node, kv_node, cmd_rx, msg_rx, state_tx, transport, shutdown_rx,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::LocalTransport;
    use crate::node::Node;
    use crate::raft_node::RaftNode;
    use crate::storage::RaftStorage;
    use crossbeam_channel::unbounded;
    use std::collections::HashMap;

    fn create_test_raft_node(id: u64) -> RaftNode {
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let storage = RaftStorage::new();
        RaftNode::new(id, vec![id], storage, logger).unwrap()
    }

    #[test]
    fn test_builder_missing_raft_node() {
        let builder: RaftDriverBuilder<LocalTransport> = RaftDriverBuilder::new();
        let result = builder.build();
        assert!(matches!(result, Err(BuilderError::MissingRaftNode)));
    }

    #[test]
    fn test_builder_missing_kv_node() {
        let builder: RaftDriverBuilder<LocalTransport> =
            RaftDriverBuilder::new().raft_node(create_test_raft_node(1));
        let result = builder.build();
        assert!(matches!(result, Err(BuilderError::MissingKvNode)));
    }

    #[test]
    fn test_builder_missing_cmd_rx() {
        let builder: RaftDriverBuilder<LocalTransport> = RaftDriverBuilder::new()
            .raft_node(create_test_raft_node(1))
            .kv_node(Node::new());
        let result = builder.build();
        assert!(matches!(result, Err(BuilderError::MissingCmdRx)));
    }

    #[test]
    fn test_builder_missing_msg_rx() {
        let (_, cmd_rx) = unbounded();
        let builder: RaftDriverBuilder<LocalTransport> = RaftDriverBuilder::new()
            .raft_node(create_test_raft_node(1))
            .kv_node(Node::new())
            .cmd_rx(cmd_rx);
        let result = builder.build();
        assert!(matches!(result, Err(BuilderError::MissingMsgRx)));
    }

    #[test]
    fn test_builder_missing_state_tx() {
        let (_, cmd_rx) = unbounded();
        let (_, msg_rx) = unbounded();
        let builder: RaftDriverBuilder<LocalTransport> = RaftDriverBuilder::new()
            .raft_node(create_test_raft_node(1))
            .kv_node(Node::new())
            .cmd_rx(cmd_rx)
            .msg_rx(msg_rx);
        let result = builder.build();
        assert!(matches!(result, Err(BuilderError::MissingStateTx)));
    }

    #[test]
    fn test_builder_missing_transport() {
        let (_, cmd_rx) = unbounded();
        let (_, msg_rx) = unbounded();
        let (state_tx, _) = unbounded();
        let builder: RaftDriverBuilder<LocalTransport> = RaftDriverBuilder::new()
            .raft_node(create_test_raft_node(1))
            .kv_node(Node::new())
            .cmd_rx(cmd_rx)
            .msg_rx(msg_rx)
            .state_tx(state_tx);
        let result = builder.build();
        assert!(matches!(result, Err(BuilderError::MissingTransport)));
    }

    #[test]
    fn test_builder_missing_shutdown_rx() {
        let (_, cmd_rx) = unbounded();
        let (_, msg_rx) = unbounded();
        let (state_tx, _) = unbounded();
        let transport = LocalTransport::new(1, HashMap::new());
        let builder = RaftDriverBuilder::new()
            .raft_node(create_test_raft_node(1))
            .kv_node(Node::new())
            .cmd_rx(cmd_rx)
            .msg_rx(msg_rx)
            .state_tx(state_tx)
            .transport(transport);
        let result = builder.build();
        assert!(matches!(result, Err(BuilderError::MissingShutdownRx)));
    }

    #[test]
    fn test_builder_success() {
        let (_, cmd_rx) = unbounded();
        let (_, msg_rx) = unbounded();
        let (state_tx, _) = unbounded();
        let (_, shutdown_rx) = unbounded();
        let transport = LocalTransport::new(1, HashMap::new());

        let builder = RaftDriverBuilder::new()
            .raft_node(create_test_raft_node(1))
            .kv_node(Node::new())
            .cmd_rx(cmd_rx)
            .msg_rx(msg_rx)
            .state_tx(state_tx)
            .transport(transport)
            .shutdown_rx(shutdown_rx);

        let result = builder.build();
        assert!(result.is_ok(), "Builder should succeed with all fields set");
    }

    #[test]
    fn test_builder_fluent_api() {
        let (_, cmd_rx) = unbounded();
        let (_, msg_rx) = unbounded();
        let (state_tx, _) = unbounded();
        let (_, shutdown_rx) = unbounded();
        let transport = LocalTransport::new(1, HashMap::new());

        // Test that methods return self for chaining
        let driver = RaftDriverBuilder::new()
            .raft_node(create_test_raft_node(1))
            .kv_node(Node::new())
            .cmd_rx(cmd_rx)
            .msg_rx(msg_rx)
            .state_tx(state_tx)
            .transport(transport)
            .shutdown_rx(shutdown_rx)
            .build()
            .expect("Builder should succeed");

        // Just verify it built successfully
        drop(driver);
    }

    #[test]
    fn test_builder_error_display() {
        assert_eq!(
            BuilderError::MissingRaftNode.to_string(),
            "RaftNode is required"
        );
        assert_eq!(
            BuilderError::MissingKvNode.to_string(),
            "KvNode is required"
        );
        assert_eq!(
            BuilderError::MissingCmdRx.to_string(),
            "Command receiver is required"
        );
        assert_eq!(
            BuilderError::MissingMsgRx.to_string(),
            "Message receiver is required"
        );
        assert_eq!(
            BuilderError::MissingStateTx.to_string(),
            "State sender is required"
        );
        assert_eq!(
            BuilderError::MissingTransport.to_string(),
            "Transport is required"
        );
        assert_eq!(
            BuilderError::MissingShutdownRx.to_string(),
            "Shutdown receiver is required"
        );
    }

    #[test]
    fn test_builder_default() {
        let builder: RaftDriverBuilder<LocalTransport> = RaftDriverBuilder::default();
        let result = builder.build();
        assert!(result.is_err(), "Default builder should fail without fields");
    }
}
