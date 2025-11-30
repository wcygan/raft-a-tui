//! Test utilities and mock implementations for unit testing.
//!
//! This module provides mock implementations of traits used in the RaftDriver
//! to enable isolated unit testing of individual components.

use raft::prelude::{Entry, Message};

use crate::entry_applicator::EntryApplicator;
use crate::network::{Transport, TransportError};
use crate::raft_loop::RaftLoopError;
use crate::raft_node::RaftNode;

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
