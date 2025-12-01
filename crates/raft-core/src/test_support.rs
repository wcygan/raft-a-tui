//! Test utilities and mock implementations for unit testing.
//!
//! This module provides mock implementations of traits used in the RaftDriver
//! to enable isolated unit testing of individual components.

use raft::prelude::{Entry, Message};

use crate::builder::RaftDriverBuilder;
use crate::commands::ServerCommand;
use crate::entry_applicator::EntryApplicator;
use crate::network::{LocalTransport, Transport, TransportError};
use crate::node::Node;
use crate::raft_loop::{RaftDriver, RaftLoopError, StateUpdate};
use crate::raft_node::RaftNode;
use crate::storage::RaftStorage;
use crossbeam_channel::{unbounded, Receiver, SendError, Sender};
use std::collections::HashMap;

/// Mock transport that always fails with ChannelFull error.
///
/// Useful for testing error handling and resilience to transport failures.
#[derive(Debug, Clone)]
pub struct FailingTransport;

impl Transport for FailingTransport {
    fn send(&self, to: u64, _msg: Message) -> Result<(), TransportError> {
        Err(TransportError::ChannelFull(to))
    }
}

/// Mock entry applicator for testing ReadyProcessor.
///
/// Records all entries passed to `apply_entries()` and allows
/// simulating failures via `fail_on_apply`.
#[derive(Debug, Default)]
pub struct MockEntryApplicator {
    /// All entries that have been applied
    pub applied_entries: Vec<Entry>,
    /// If true, `apply_entries()` will return an error
    pub fail_on_apply: bool,
    /// If true, `restore_snapshot()` will return an error
    pub fail_on_snapshot: bool,
}

impl MockEntryApplicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a mock applicator that fails on apply_entries
    pub fn failing() -> Self {
        Self {
            applied_entries: Vec::new(),
            fail_on_apply: true,
            fail_on_snapshot: false,
        }
    }

    /// Returns the number of entries applied
    pub fn num_applied(&self) -> usize {
        self.applied_entries.len()
    }

    /// Checks if a specific entry index was applied
    pub fn has_entry_at_index(&self, index: u64) -> bool {
        self.applied_entries.iter().any(|e| e.index == index)
    }

    /// Clears all applied entries (useful for multi-step tests)
    pub fn clear(&mut self) {
        self.applied_entries.clear();
    }
}

impl EntryApplicator for MockEntryApplicator {
    fn apply_entries(
        &mut self,
        entries: Vec<Entry>,
        _raft_node: &mut RaftNode,
    ) -> Result<(), RaftLoopError> {
        if self.fail_on_apply {
            return Err(RaftLoopError::Other("Mock failure".into()));
        }
        self.applied_entries.extend(entries);
        Ok(())
    }

    fn restore_snapshot(&mut self, _snapshot_data: &[u8]) -> Result<(), RaftLoopError> {
        if self.fail_on_snapshot {
            return Err(RaftLoopError::Other("Mock snapshot failure".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failing_transport_always_fails() {
        let transport = FailingTransport;
        let msg = Message::default();
        let result = transport.send(1, msg);
        assert!(result.is_err());
        assert!(matches!(result, Err(TransportError::ChannelFull(1))));
    }

    #[test]
    fn test_mock_applicator_records_entries() {
        let mut applicator = MockEntryApplicator::new();
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let mut raft_node = crate::raft_node::RaftNode::new(
            1,
            vec![1],
            crate::storage::RaftStorage::new(),
            logger,
        )
        .unwrap();

        let entry1 = Entry {
            index: 1,
            term: 1,
            ..Default::default()
        };
        let entry2 = Entry {
            index: 2,
            term: 1,
            ..Default::default()
        };

        applicator
            .apply_entries(vec![entry1, entry2], &mut raft_node)
            .unwrap();

        assert_eq!(applicator.num_applied(), 2);
        assert!(applicator.has_entry_at_index(1));
        assert!(applicator.has_entry_at_index(2));
        assert!(!applicator.has_entry_at_index(3));
    }

    #[test]
    fn test_mock_applicator_can_fail() {
        let mut applicator = MockEntryApplicator::failing();
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let mut raft_node = crate::raft_node::RaftNode::new(
            1,
            vec![1],
            crate::storage::RaftStorage::new(),
            logger,
        )
        .unwrap();

        let result = applicator.apply_entries(vec![], &mut raft_node);
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_applicator_clear() {
        let mut applicator = MockEntryApplicator::new();
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let mut raft_node = crate::raft_node::RaftNode::new(
            1,
            vec![1],
            crate::storage::RaftStorage::new(),
            logger,
        )
        .unwrap();

        let entry = Entry {
            index: 1,
            term: 1,
            ..Default::default()
        };
        applicator.apply_entries(vec![entry], &mut raft_node).unwrap();
        assert_eq!(applicator.num_applied(), 1);

        applicator.clear();
        assert_eq!(applicator.num_applied(), 0);
    }
}

/// Test harness that wraps RaftDriver with all necessary channel handles.
///
/// Provides convenient access to send commands and receive state updates
/// without needing TCP connections.
pub struct RaftDriverTestHarness<T: Transport> {
    pub driver: RaftDriver<T>,
    pub cmd_tx: Sender<ServerCommand>,
    pub msg_tx: Sender<Message>,
    pub state_rx: Receiver<StateUpdate>,
    pub shutdown_tx: Sender<()>,
}

impl RaftDriverTestHarness<LocalTransport> {
    /// Creates a single-node test harness with LocalTransport.
    ///
    /// # Arguments
    /// * `node_id` - The ID for the single node
    ///
    /// # Example
    /// ```
    /// use raft_core::test_support::RaftDriverTestHarness;
    ///
    /// let harness = RaftDriverTestHarness::single_node(1);
    /// // Now you can send commands and check state updates
    /// ```
    pub fn single_node(node_id: u64) -> Self {
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let storage = RaftStorage::new();
        let raft_node = RaftNode::new(node_id, vec![node_id], storage, logger).unwrap();
        let kv_node = Node::new();

        let (cmd_tx, cmd_rx) = unbounded();
        let (msg_tx, msg_rx) = unbounded();
        let (state_tx, state_rx) = unbounded();
        let (shutdown_tx, shutdown_rx) = unbounded();

        // LocalTransport with no peers (single node)
        let transport = LocalTransport::new(node_id, HashMap::new());

        let driver = RaftDriverBuilder::new()
            .raft_node(raft_node)
            .kv_node(kv_node)
            .cmd_rx(cmd_rx)
            .msg_rx(msg_rx)
            .state_tx(state_tx)
            .transport(transport)
            .shutdown_rx(shutdown_rx)
            .build()
            .expect("Failed to build RaftDriver in test harness");

        Self {
            driver,
            cmd_tx,
            msg_tx,
            state_rx,
            shutdown_tx,
        }
    }
}

impl<T: Transport> RaftDriverTestHarness<T> {
    /// Creates a test harness with a custom transport.
    ///
    /// # Arguments
    /// * `node_id` - The ID for the node
    /// * `transport` - Custom transport implementation
    ///
    /// # Example
    /// ```
    /// use raft_core::test_support::{RaftDriverTestHarness, FailingTransport};
    ///
    /// let harness = RaftDriverTestHarness::with_transport(1, FailingTransport);
    /// ```
    pub fn with_transport(node_id: u64, transport: T) -> Self {
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let storage = RaftStorage::new();
        let raft_node = RaftNode::new(node_id, vec![node_id], storage, logger).unwrap();
        let kv_node = Node::new();

        let (cmd_tx, cmd_rx) = unbounded();
        let (msg_tx, msg_rx) = unbounded();
        let (state_tx, state_rx) = unbounded();
        let (shutdown_tx, shutdown_rx) = unbounded();

        let driver = RaftDriverBuilder::new()
            .raft_node(raft_node)
            .kv_node(kv_node)
            .cmd_rx(cmd_rx)
            .msg_rx(msg_rx)
            .state_tx(state_tx)
            .transport(transport)
            .shutdown_rx(shutdown_rx)
            .build()
            .expect("Failed to build RaftDriver in test harness");

        Self {
            driver,
            cmd_tx,
            msg_tx,
            state_rx,
            shutdown_tx,
        }
    }

    /// Send a command to the driver.
    pub fn send_command(&self, cmd: ServerCommand) -> Result<(), SendError<ServerCommand>> {
        self.cmd_tx.send(cmd)
    }

    /// Trigger shutdown of the driver.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}
