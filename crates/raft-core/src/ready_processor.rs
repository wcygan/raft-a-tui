use crossbeam_channel::Sender;
use raft::prelude::{Entry, Message, Snapshot};
use slog::{debug, error, info, warn, Logger};

use crate::entry_applicator::EntryApplicator;
use crate::network::Transport;
use crate::raft_loop::{RaftLoopError, StateUpdate};
use crate::raft_node::RaftNode;

/// Trait for processing Raft Ready state.
pub trait ReadyProcessor {
    fn process_ready(&mut self) -> Result<(), RaftLoopError>;
}

/// Default Ready processor implementation.
///
/// This coordinates the Ready loop pattern from raft-rs:
/// 1. Send non-persisted messages
/// 2. Apply snapshot if present
/// 3. Append entries to storage
/// 4. Persist HardState
/// 5. Send persisted messages
/// 6. Apply committed entries
/// 7. Update TUI state
/// 8. Advance and process LightReady
pub struct DefaultReadyProcessor<'a, T: Transport, A: EntryApplicator> {
    raft_node: &'a mut RaftNode,
    transport: &'a T,
    entry_applicator: &'a mut A,
    state_tx: &'a Sender<StateUpdate>,
    logger: &'a Logger,
    previous_leader_id: &'a mut Option<u64>,
}

impl<'a, T: Transport, A: EntryApplicator> DefaultReadyProcessor<'a, T, A> {
    pub fn new(
        raft_node: &'a mut RaftNode,
        transport: &'a T,
        entry_applicator: &'a mut A,
        state_tx: &'a Sender<StateUpdate>,
        logger: &'a Logger,
        previous_leader_id: &'a mut Option<u64>,
    ) -> Self {
        Self {
            raft_node,
            transport,
            entry_applicator,
            state_tx,
            logger,
            previous_leader_id,
        }
    }

    fn send_messages(&self, messages: Vec<Message>) {
        if messages.is_empty() {
            return;
        }
        debug!(self.logger, "Sending messages"; "count" => messages.len());

        for msg in messages {
            let to = msg.to;
            if let Err(e) = self.transport.send(to, msg) {
                warn!(self.logger, "Failed to send message"; "peer" => to, "error" => format!("{}", e));
            }
        }
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot) -> Result<(), RaftLoopError> {
        let snapshot_index = snapshot.get_metadata().index;
        let snapshot_term = snapshot.get_metadata().term;

        info!(self.logger, "Applying snapshot"; "index" => snapshot_index, "term" => snapshot_term);

        self.raft_node
            .raw_node_mut()
            .mut_store()
            .apply_snapshot(snapshot.clone())
            .map_err(|e| {
                error!(self.logger, "Failed to apply snapshot"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })?;

        let snapshot_data = snapshot.get_data();
        self.entry_applicator.restore_snapshot(snapshot_data)?;

        self.raft_node
            .raw_node_mut()
            .mut_store()
            .set_applied_index(snapshot_index)
            .map_err(|e| {
                error!(self.logger, "Failed to set applied index"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })?;

        info!(self.logger, "Snapshot applied successfully");
        let _ = self.state_tx.send(StateUpdate::SystemMessage(format!(
            "📸 Applied snapshot at index {} (term {})",
            snapshot_index, snapshot_term
        )));

        Ok(())
    }

    fn append_entries(&mut self, entries: &[Entry]) -> Result<(), RaftLoopError> {
        debug!(self.logger, "Appending entries to storage"; "count" => entries.len());
        self.raft_node
            .raw_node_mut()
            .mut_store()
            .append(entries)
            .map_err(|e| {
                error!(self.logger, "Failed to append entries"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })
    }

    fn persist_hardstate(&mut self, hs: raft::prelude::HardState) -> Result<(), RaftLoopError> {
        debug!(self.logger, "Persisting HardState"; "term" => hs.term, "vote" => hs.vote, "commit" => hs.commit);
        self.raft_node
            .raw_node_mut()
            .mut_store()
            .set_hardstate(hs)
            .map_err(|e| {
                error!(self.logger, "Failed to persist HardState"; "error" => format!("{}", e));
                RaftLoopError::StorageError(e)
            })
    }

    fn update_tui_state(&mut self) {
        let raft_state = self.raft_node.get_state();

        if let Some(prev_leader) = *self.previous_leader_id {
            if prev_leader != raft_state.leader_id && raft_state.leader_id != 0 {
                let msg = if prev_leader == 0 {
                    format!(
                        "New leader elected: {} (term {})",
                        raft_state.leader_id, raft_state.term
                    )
                } else {
                    format!(
                        "Leadership changed: {} → {} (term {})",
                        prev_leader, raft_state.leader_id, raft_state.term
                    )
                };
                info!(self.logger, "Leadership changed"; "new_leader" => raft_state.leader_id);
                let _ = self.state_tx.send(StateUpdate::SystemMessage(msg));
            }
        }
        *self.previous_leader_id = Some(raft_state.leader_id);

        let _ = self.state_tx.send(StateUpdate::RaftState(raft_state));
    }
}

impl<'a, T: Transport, A: EntryApplicator> ReadyProcessor for DefaultReadyProcessor<'a, T, A> {
    fn process_ready(&mut self) -> Result<(), RaftLoopError> {
        let mut ready = self.raft_node.raw_node_mut().ready();

        debug!(self.logger, "Processing Ready state";
               "has_snapshot" => !ready.snapshot().is_empty(),
               "num_entries" => ready.entries().len(),
               "num_messages" => ready.messages().len(),
               "num_persisted_messages" => ready.persisted_messages().len(),
               "num_committed" => ready.committed_entries().len());

        // 1. Send non-persisted messages
        self.send_messages(ready.take_messages());

        // 2. Apply snapshot if present
        if !ready.snapshot().is_empty() {
            self.apply_snapshot(ready.snapshot().clone())?;
        }

        // 3. Append entries to storage
        if !ready.entries().is_empty() {
            self.append_entries(ready.entries())?;
        }

        // 4. Persist HardState
        if let Some(hs) = ready.hs() {
            self.persist_hardstate(hs.clone())?;
        }

        // 5. Send persisted messages
        self.send_messages(ready.take_persisted_messages());

        // 6. Apply committed entries
        let committed_entries = ready.take_committed_entries();
        if !committed_entries.is_empty() {
            self.entry_applicator
                .apply_entries(committed_entries, self.raft_node)?;
        }

        // 7. Update TUI state
        self.update_tui_state();

        // 8. Advance and process LightReady
        let mut light_rd = self.raft_node.raw_node_mut().advance(ready);

        if let Some(commit) = light_rd.commit_index() {
            debug!(self.logger, "LightReady commit index"; "commit" => commit);
        }

        self.send_messages(light_rd.messages().to_vec());

        let light_committed = light_rd.take_committed_entries();
        if !light_committed.is_empty() {
            self.entry_applicator
                .apply_entries(light_committed, self.raft_node)?;
        }

        Ok(())
    }
}
