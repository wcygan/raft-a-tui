use crossbeam_channel::Sender;
use raft::prelude::{ConfChange, ConfChangeV2, Entry, EntryType};
use slog::{debug, error, info, warn, Logger};

use crate::node::Node;
use crate::raft_loop::{RaftLoopError, StateUpdate};
use crate::raft_node::RaftNode;

/// Trait for applying committed Raft entries to the state machine.
pub trait EntryApplicator {
    /// Apply committed entries to the state machine.
    ///
    /// RaftNode is passed as a parameter (not held as a field) to avoid borrow checker
    /// conflicts with ReadyProcessor which also needs mutable access to RaftNode.
    fn apply_entries(
        &mut self,
        entries: Vec<Entry>,
        raft_node: &mut RaftNode,
    ) -> Result<(), RaftLoopError>;

    /// Restore state machine from snapshot data.
    fn restore_snapshot(&mut self, snapshot_data: &[u8]) -> Result<(), RaftLoopError>;
}

/// Default implementation for Key-Value state machine.
pub struct KvEntryApplicator<'a> {
    kv_node: &'a mut Node,
    state_tx: &'a Sender<StateUpdate>,
    logger: &'a Logger,
}

impl<'a> KvEntryApplicator<'a> {
    pub fn new(
        kv_node: &'a mut Node,
        state_tx: &'a Sender<StateUpdate>,
        logger: &'a Logger,
    ) -> Self {
        Self {
            kv_node,
            state_tx,
            logger,
        }
    }

    fn apply_normal_entry(
        &mut self,
        entry: &Entry,
        raft_node: &mut RaftNode,
    ) -> Result<(), RaftLoopError> {
        match self.kv_node.apply_kv_command(&entry.data) {
            Ok(_) => {
                debug!(self.logger, "Applied KV command"; "index" => entry.index);
                self.notify_kv_update(entry, raft_node);
            }
            Err(e) => {
                warn!(self.logger, "Failed to apply KV command"; "error" => format!("{}", e));
                if let Some(callback) = raft_node.take_callback(&entry.context) {
                    let _ = callback.send(Err(format!("Apply failed: {}", e)));
                }
            }
        }

        raft_node
            .raw_node_mut()
            .mut_store()
            .set_applied_index(entry.index)
            .map_err(|e| {
                error!(self.logger, "Failed to set applied index"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })
    }

    fn apply_conf_change(
        &mut self,
        entry: &Entry,
        raft_node: &mut RaftNode,
    ) -> Result<(), RaftLoopError> {
        info!(self.logger, "Applying configuration change"; "index" => entry.index);

        let mut cc = ConfChange::default();
        if let Err(e) = prost_011::Message::merge(&mut cc, &entry.data[..]) {
            error!(self.logger, "Failed to decode ConfChange"; "error" => format!("{}", e));
            return Ok(()); // Continue processing other entries
        }

        let cs = match raft_node.raw_node_mut().apply_conf_change(&cc) {
            Ok(cs) => cs,
            Err(e) => {
                error!(self.logger, "Failed to apply ConfChange"; "error" => format!("{}", e));
                return Ok(()); // Continue processing other entries
            }
        };

        raft_node
            .raw_node_mut()
            .mut_store()
            .set_conf_state(cs)
            .map_err(|e| {
                error!(self.logger, "Failed to persist ConfState"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })?;

        raft_node
            .raw_node_mut()
            .mut_store()
            .set_applied_index(entry.index)
            .map_err(|e| {
                error!(self.logger, "Failed to set applied index"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })
    }

    fn apply_conf_change_v2(
        &mut self,
        entry: &Entry,
        raft_node: &mut RaftNode,
    ) -> Result<(), RaftLoopError> {
        info!(self.logger, "Applying configuration change V2"; "index" => entry.index);

        let mut cc = ConfChangeV2::default();
        if let Err(e) = prost_011::Message::merge(&mut cc, &entry.data[..]) {
            error!(self.logger, "Failed to decode ConfChangeV2"; "error" => format!("{}", e));
            return Ok(()); // Continue processing other entries
        }

        let cs = match raft_node.raw_node_mut().apply_conf_change(&cc) {
            Ok(cs) => cs,
            Err(e) => {
                error!(self.logger, "Failed to apply ConfChangeV2"; "error" => format!("{}", e));
                return Ok(()); // Continue processing other entries
            }
        };

        raft_node
            .raw_node_mut()
            .mut_store()
            .set_conf_state(cs)
            .map_err(|e| {
                error!(self.logger, "Failed to persist ConfState"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })?;

        raft_node
            .raw_node_mut()
            .mut_store()
            .set_applied_index(entry.index)
            .map_err(|e| {
                error!(self.logger, "Failed to set applied index"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })
    }

    fn notify_kv_update(&mut self, entry: &Entry, raft_node: &mut RaftNode) {
        if let Ok(cmd) = crate::codec::decode::<raft_proto::kvraft::KvCommand>(&entry.data) {
            if let Some(raft_proto::kvraft::kv_command::Cmd::Put(put)) = cmd.cmd {
                let _ = self.state_tx.send(StateUpdate::KvUpdate {
                    key: put.key.clone(),
                    value: put.value.clone(),
                });
                let _ = self.state_tx.send(StateUpdate::LogEntry {
                    index: entry.index,
                    term: entry.term,
                    data: format!("PUT {} = {}", put.key, put.value),
                });

                if let Some(callback) = raft_node.take_callback(&entry.context) {
                    let _ = callback.send(Ok(()));
                }
            }
        }
    }
}

impl<'a> EntryApplicator for KvEntryApplicator<'a> {
    fn apply_entries(
        &mut self,
        entries: Vec<Entry>,
        raft_node: &mut RaftNode,
    ) -> Result<(), RaftLoopError> {
        for entry in entries {
            if entry.data.is_empty() {
                continue;
            }

            match entry.get_entry_type() {
                EntryType::EntryNormal => self.apply_normal_entry(&entry, raft_node)?,
                EntryType::EntryConfChange => self.apply_conf_change(&entry, raft_node)?,
                EntryType::EntryConfChangeV2 => self.apply_conf_change_v2(&entry, raft_node)?,
            }
        }
        Ok(())
    }

    fn restore_snapshot(&mut self, snapshot_data: &[u8]) -> Result<(), RaftLoopError> {
        self.kv_node
            .restore_from_snapshot(snapshot_data)
            .map_err(|e| RaftLoopError::Other(format!("Failed to restore snapshot: {}", e)))
    }
}
