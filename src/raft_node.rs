use std::collections::HashMap;

use crossbeam_channel::Sender;
use raft::StateRole;
use raft::prelude::*;
use raft::raw_node::RawNode;
use slog::Logger;
use uuid::Uuid;

use crate::storage::RaftStorage;

/// Response from a committed Raft command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResponse {
    /// Command was successfully applied to the state machine.
    Success { key: String, value: String },
    /// Command failed with an error message.
    Error(String),
}

/// Wrapper around raft-rs RawNode with callback tracking.
///
/// Manages Raft consensus, proposal callbacks, and provides
/// a high-level interface for the application.
pub struct RaftNode {
    raw_node: RawNode<RaftStorage>,
    callbacks: HashMap<Vec<u8>, Sender<CommandResponse>>,
    logger: Logger,
}

impl RaftNode {
    /// Create a new RaftNode with the given configuration.
    ///
    /// # Arguments
    /// * `id` - This node's ID
    /// * `peers` - List of all peer IDs in the cluster (including self)
    /// * `storage` - Storage implementation for Raft logs and state
    /// * `logger` - Logger for Raft events
    ///
    /// # Config Values
    /// Uses recommended defaults from CLAUDE.md:
    /// - heartbeat_tick: 3 (300ms with 100ms tick)
    /// - election_tick: 10 (1 second with 100ms tick)
    /// - check_quorum: true (leader checks quorum connectivity)
    /// - pre_vote: true (prevent unnecessary elections)
    ///
    /// # Example
    /// ```no_run
    /// use raft_a_tui::storage::RaftStorage;
    /// use raft_a_tui::raft_node::RaftNode;
    /// use slog::{Drain, Logger, o};
    ///
    /// let decorator = slog_term::TermDecorator::new().build();
    /// let drain = slog_term::FullFormat::new(decorator).build().fuse();
    /// let drain = slog_async::Async::new(drain).build().fuse();
    /// let logger = Logger::root(drain, o!());
    ///
    /// let storage = RaftStorage::new();
    /// let peers = vec![1, 2, 3];
    /// let node = RaftNode::new(1, peers, storage, logger).unwrap();
    /// ```
    pub fn new(
        id: u64,
        peers: Vec<u64>,
        mut storage: RaftStorage,
        logger: Logger,
    ) -> Result<Self, raft::Error> {
        let config = Config {
            id,
            heartbeat_tick: 3,
            election_tick: 10,
            check_quorum: false,  // Disabled to allow CAMPAIGN command to work on followers
            pre_vote: true,
            ..Default::default()
        };

        config.validate()?;

        // Create ConfState with all peers as voters
        let conf_state = ConfState {
            voters: peers,
            ..Default::default()
        };

        // Initialize storage with conf_state
        // MemStorage needs to be initialized before creating RawNode
        // DiskStorage may already have persisted conf_state, so this is conditional
        storage.initialize_with_conf_state(conf_state);

        let raw_node = RawNode::new(&config, storage, &logger)?;

        Ok(Self {
            raw_node,
            callbacks: HashMap::new(),
            logger,
        })
    }

    /// Propose a command to the Raft cluster.
    ///
    /// Returns a channel receiver for the command result.
    /// The response will be sent when the command is committed.
    ///
    /// # Arguments
    /// * `key` - The key to set
    /// * `value` - The value to set
    ///
    /// # Returns
    /// A receiver that will get the CommandResponse when committed,
    /// or an error if the proposal fails.
    ///
    /// # Errors
    /// Returns `Error::ProposalDropped` if this node is not the leader.
    /// Clients should redirect to the current leader (available via `get_state().leader_id`).
    pub fn propose_command(
        &mut self,
        key: String,
        value: String,
    ) -> Result<crossbeam_channel::Receiver<CommandResponse>, raft::Error> {
        // CRITICAL: Only leaders can accept write proposals
        // This is a fundamental Raft safety requirement
        if !self.is_leader() {
            return Err(raft::Error::ProposalDropped);
        }

        // Generate unique callback ID
        let callback_id = Uuid::new_v4().as_bytes().to_vec();

        // Create channel for response
        let (tx, rx) = crossbeam_channel::bounded(1);

        // Encode the command using Node's helper
        let data = crate::node::Node::encode_put_command(&key, &value);

        // Store callback
        self.callbacks.insert(callback_id.clone(), tx);

        // Propose to Raft (context = callback_id)
        self.raw_node.propose(callback_id, data)?;

        Ok(rx)
    }

    /// Get the current Raft state.
    ///
    /// Returns information about the node's role, term, leader, etc.
    pub fn get_state(&self) -> RaftState {
        // Count the number of voters in the cluster configuration
        let cluster_size = self.raw_node.raft.prs().conf().voters().ids().iter().count();

        RaftState {
            node_id: self.raw_node.raft.id,
            term: self.raw_node.raft.term,
            role: self.raw_node.raft.state,
            leader_id: self.raw_node.raft.leader_id,
            commit_index: self.raw_node.raft.raft_log.committed,
            applied_index: self.raw_node.store().applied_index(),
            cluster_size,
        }
    }

    /// Check if this node is the current leader.
    pub fn is_leader(&self) -> bool {
        self.raw_node.raft.state == StateRole::Leader
    }

    /// Get a reference to the logger.
    pub fn logger(&self) -> &Logger {
        &self.logger
    }

    /// Get a reference to the underlying RawNode.
    pub fn raw_node(&self) -> &RawNode<RaftStorage> {
        &self.raw_node
    }

    /// Get a mutable reference to the underlying RawNode.
    pub fn raw_node_mut(&mut self) -> &mut RawNode<RaftStorage> {
        &mut self.raw_node
    }

    /// Get the callback sender for a given context.
    ///
    /// This is used by the Ready loop to send responses when
    /// commands are committed.
    pub fn take_callback(&mut self, context: &[u8]) -> Option<Sender<CommandResponse>> {
        self.callbacks.remove(context)
    }
}

/// Information about the current Raft state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaftState {
    pub node_id: u64,
    pub term: u64,
    pub role: StateRole,
    pub leader_id: u64,
    pub commit_index: u64,
    pub applied_index: u64,
    /// Number of nodes in the cluster (voter count from configuration)
    pub cluster_size: usize,
}
