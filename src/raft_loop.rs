use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, select};
use raft::prelude::*;
use slog::{debug, error, info, warn};

use crate::commands::UserCommand;
use crate::network::{Transport, TransportError};
use crate::node::Node;
use crate::raft_node::{CommandResponse, RaftNode, RaftState};

/// State updates sent to the TUI for visualization.
///
/// The Ready loop sends these updates whenever significant
/// events occur in the Raft cluster or KV store.
#[derive(Debug, Clone)]
pub enum StateUpdate {
    /// Current Raft state (term, role, leader, indices).
    RaftState(RaftState),
    /// A key-value pair was updated in the store.
    KvUpdate { key: String, value: String },
    /// A log entry was committed and applied.
    LogEntry { index: u64, term: u64, data: String },
    /// Human-readable system event message.
    SystemMessage(String),
}

/// Errors that can occur in the Raft ready loop.
#[derive(Debug)]
pub enum RaftLoopError {
    /// A critical channel was closed unexpectedly.
    ChannelClosed(String),
    /// Storage operation failed.
    StorageError(raft::Error),
    /// Transport failed (non-fatal, logged and continued).
    TransportError(TransportError),
    /// Other errors.
    Other(String),
}

impl std::fmt::Display for RaftLoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RaftLoopError::ChannelClosed(msg) => write!(f, "Channel closed: {}", msg),
            RaftLoopError::StorageError(e) => write!(f, "Storage error: {}", e),
            RaftLoopError::TransportError(e) => write!(f, "Transport error: {}", e),
            RaftLoopError::Other(msg) => write!(f, "Error: {}", msg),
        }
    }
}

impl std::error::Error for RaftLoopError {}

impl From<raft::Error> for RaftLoopError {
    fn from(err: raft::Error) -> Self {
        RaftLoopError::StorageError(err)
    }
}

/// The main Raft ready loop.
///
/// This function drives the Raft consensus algorithm by:
/// 1. Receiving commands from CLI/TUI and network messages from peers
/// 2. Ticking the Raft state machine at regular intervals (100ms)
/// 3. Processing Ready state changes (persist, send, apply)
/// 4. Applying committed entries to the KV store
/// 5. Sending state updates to the TUI for visualization
///
/// # Arguments
/// * `raft_node` - The Raft node wrapper (contains RawNode and callbacks)
/// * `kv_node` - The KV state machine
/// * `cmd_rx` - Channel for receiving user commands from CLI/TUI
/// * `msg_rx` - Channel for receiving Raft messages from other nodes
/// * `state_tx` - Channel for sending state updates to TUI
/// * `transport` - Network transport for sending messages to peers
/// * `shutdown_rx` - Channel for graceful shutdown signal
///
/// # Returns
/// Returns Ok(()) on graceful shutdown, or an error if a critical failure occurs.
///
/// # Loop Structure
/// The loop follows the standard raft-rs Ready pattern:
/// - Phase 1: Input reception (commands, messages, tick)
/// - Phase 2: Ready check (has_ready())
/// - Phase 3: Ready processing (snapshot, entries, hardstate, messages, apply, advance)
///
/// # Error Handling
/// - step() errors: Logged and continued (resilience)
/// - apply_kv_command errors: Logged and continued
/// - Transport errors: Logged and continued (network may be flaky)
/// - Channel closed: Returns error (fatal)
/// - Storage errors: Returns error (fatal)
const TICK_INTERVAL: Duration = Duration::from_millis(100);

/// The driver for the Raft consensus algorithm.
///
/// Encapsulates the state and logic for driving the Raft node, including:
/// - Handling input (commands, messages, ticks)
/// - Processing Ready states
/// - Applying entries to the state machine
/// - Managing the transport and storage
pub struct RaftDriver<T: Transport> {
    raft_node: RaftNode,
    kv_node: Node,
    cmd_rx: Receiver<UserCommand>,
    msg_rx: Receiver<Message>,
    state_tx: Sender<StateUpdate>,
    transport: T,
    shutdown_rx: Receiver<()>,
    logger: slog::Logger,
    previous_leader_id: Option<u64>,
}

impl<T: Transport> RaftDriver<T> {
    pub fn new(
        raft_node: RaftNode,
        kv_node: Node,
        cmd_rx: Receiver<UserCommand>,
        msg_rx: Receiver<Message>,
        state_tx: Sender<StateUpdate>,
        transport: T,
        shutdown_rx: Receiver<()>,
    ) -> Self {
        let logger = raft_node.logger().clone();
        Self {
            raft_node,
            kv_node,
            cmd_rx,
            msg_rx,
            state_tx,
            transport,
            shutdown_rx,
            logger,
            previous_leader_id: None,
        }
    }

    pub fn run(mut self) -> Result<(), RaftLoopError> {
        info!(self.logger, "Raft ready loop starting"; "node_id" => self.raft_node.get_state().node_id);

        let mut tick_timer = Instant::now();

        loop {
            // Calculate timeout until next tick
            let timeout = TICK_INTERVAL.saturating_sub(tick_timer.elapsed());

            // Phase 1: Input Reception
            select! {
                recv(self.cmd_rx) -> result => {
                    match result {
                        Ok(cmd) => self.handle_user_command(cmd)?,
                        Err(_) => {
                            info!(self.logger, "Command channel closed, shutting down");
                            return Err(RaftLoopError::ChannelClosed("cmd_rx".to_string()));
                        }
                    }
                },
                recv(self.msg_rx) -> result => {
                    match result {
                        Ok(msg) => self.handle_raft_message(msg)?,
                        Err(_) => {
                            info!(self.logger, "Message channel closed, shutting down");
                            return Err(RaftLoopError::ChannelClosed("msg_rx".to_string()));
                        }
                    }
                },
                recv(self.shutdown_rx) -> _ => {
                    info!(self.logger, "Shutdown signal received, exiting ready loop");
                    return Ok(());
                },
                default(timeout) => {},
            }

            // Tick the Raft state machine if enough time has elapsed
            if tick_timer.elapsed() >= TICK_INTERVAL {
                debug!(self.logger, "Ticking Raft state machine");
                self.raft_node.raw_node_mut().tick();
                tick_timer = Instant::now();
            }

            // Phase 2: Ready Check
            if !self.raft_node.raw_node().has_ready() {
                continue;
            }

            // Phase 3: Ready Processing
            self.process_ready()?;
        }
    }

    fn handle_user_command(&mut self, cmd: UserCommand) -> Result<(), RaftLoopError> {
        debug!(self.logger, "Received user command"; "command" => format!("{:?}", cmd));

        match cmd {
            UserCommand::Put { key, value } => {
                match self.raft_node.propose_command(key.clone(), value.clone()) {
                    Ok(rx) => {
                        debug!(self.logger, "Proposed PUT command"; "key" => &key, "value" => &value);
                        let _ = self.state_tx.send(StateUpdate::SystemMessage(
                            format!("Proposed: PUT {} = {}", key, value)
                        ));
                        drop(rx); // Fire and forget for now
                    }
                    Err(raft::Error::ProposalDropped) => {
                        let state = self.raft_node.get_state();
                        let leader_hint = if state.leader_id == 0 {
                            "no leader elected yet".to_string()
                        } else {
                            format!("leader is node {}", state.leader_id)
                        };

                        warn!(self.logger, "Rejected PUT - not leader";
                              "key" => &key,
                              "role" => format!("{:?}", state.role),
                              "leader_id" => state.leader_id);

                        let _ = self.state_tx.send(StateUpdate::SystemMessage(
                            format!("ERROR: Not leader ({}) - writes must go to leader", leader_hint)
                        ));
                    }
                    Err(e) => {
                        warn!(self.logger, "Failed to propose command"; "error" => format!("{}", e));
                        let _ = self.state_tx.send(StateUpdate::SystemMessage(
                            format!("Error: Failed to propose PUT ({})", e)
                        ));
                    }
                }
            }
            UserCommand::Campaign => {
                if let Err(e) = self.raft_node.raw_node_mut().campaign() {
                    warn!(self.logger, "Failed to campaign"; "error" => format!("{}", e));
                } else {
                    info!(self.logger, "Starting election campaign");
                    let _ = self.state_tx.send(StateUpdate::SystemMessage(
                        "Starting election campaign".to_string()
                    ));
                }
            }
            UserCommand::Status => {
                let state = self.raft_node.get_state();
                let status_msg = format!(
                    "STATUS:\n  Node ID: {}\n  Term: {}\n  Role: {:?}\n  Leader ID: {}\n  Cluster Size: {} nodes\n  Commit Index: {}\n  Applied Index: {}",
                    state.node_id,
                    state.term,
                    state.role,
                    state.leader_id,
                    state.cluster_size,
                    state.commit_index,
                    state.applied_index
                );
                let _ = self.state_tx.send(StateUpdate::SystemMessage(status_msg));
            }
            UserCommand::Get { .. } | UserCommand::Keys => {
                debug!(self.logger, "Read-only command, applying locally");
                let output = self.kv_node.apply_user_command(cmd);
                if let crate::node::NodeOutput::Text(text) = output {
                    let _ = self.state_tx.send(StateUpdate::SystemMessage(text));
                }
            }
        }
        Ok(())
    }

    fn handle_raft_message(&mut self, msg: Message) -> Result<(), RaftLoopError> {
        debug!(self.logger, "Received Raft message";
               "from" => msg.from,
               "to" => msg.to,
               "msg_type" => format!("{:?}", msg.msg_type()));

        if let Err(e) = self.raft_node.raw_node_mut().step(msg) {
            warn!(self.logger, "Failed to step Raft message"; "error" => format!("{}", e));
        }
        Ok(())
    }

    fn process_ready(&mut self) -> Result<(), RaftLoopError> {
        let mut ready = self.raft_node.raw_node_mut().ready();

        debug!(self.logger, "Processing Ready state";
               "has_snapshot" => !ready.snapshot().is_empty(),
               "num_entries" => ready.entries().len(),
               "num_messages" => ready.messages().len(),
               "num_persisted_messages" => ready.persisted_messages().len(),
               "num_committed" => ready.committed_entries().len());

        if !ready.snapshot().is_empty() {
            self.apply_snapshot(ready.snapshot().clone())?;
        }

        if !ready.entries().is_empty() {
            debug!(self.logger, "Appending entries to storage"; "count" => ready.entries().len());
            if let Err(e) = self.raft_node.raw_node_mut().mut_store().append(ready.entries()) {
                error!(self.logger, "Failed to append entries"; "error" => format!("{}", e));
                return Err(RaftLoopError::StorageError(e));
            }
        }

        if let Some(hs) = ready.hs() {
            debug!(self.logger, "Persisting HardState"; "term" => hs.term, "vote" => hs.vote, "commit" => hs.commit);
            if let Err(e) = self.raft_node.raw_node_mut().mut_store().set_hardstate(hs.clone()) {
                error!(self.logger, "Failed to persist HardState"; "error" => format!("{}", e));
                return Err(RaftLoopError::StorageError(e));
            }
        }

        self.send_messages(ready.take_messages());
        self.send_messages(ready.take_persisted_messages());

        let committed_entries = ready.take_committed_entries();
        if !committed_entries.is_empty() {
            self.apply_committed_entries(committed_entries)?;
        }

        self.update_tui_state();

        let mut light_rd = self.raft_node.raw_node_mut().advance(ready);

        if let Some(commit) = light_rd.commit_index() {
            debug!(self.logger, "LightReady commit index"; "commit" => commit);
        }

        self.send_messages(light_rd.messages().to_vec());

        let light_committed = light_rd.take_committed_entries();
        if !light_committed.is_empty() {
            self.apply_committed_entries(light_committed)?;
        }

        Ok(())
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot) -> Result<(), RaftLoopError> {
        let snapshot_index = snapshot.get_metadata().index;
        let snapshot_term = snapshot.get_metadata().term;

        info!(self.logger, "Applying snapshot"; "index" => snapshot_index, "term" => snapshot_term);

        if let Err(e) = self.raft_node.raw_node_mut().mut_store().apply_snapshot(snapshot.clone()) {
            error!(self.logger, "Failed to apply snapshot"; "error" => format!("{}", e));
            return Err(RaftLoopError::StorageError(e));
        }

        let snapshot_data = snapshot.get_data();
        if let Err(e) = self.kv_node.restore_from_snapshot(snapshot_data) {
            error!(self.logger, "Failed to restore KV state from snapshot"; "error" => format!("{}", e));
            return Err(RaftLoopError::Other(format!("Snapshot restore failed: {}", e)));
        }

        if let Err(e) = self.raft_node.raw_node_mut().mut_store().set_applied_index(snapshot_index) {
            error!(self.logger, "Failed to set applied index"; "error" => format!("{}", e));
            return Err(RaftLoopError::StorageError(e));
        }

        info!(self.logger, "Snapshot applied successfully");
        let _ = self.state_tx.send(StateUpdate::SystemMessage(format!(
            "📸 Applied snapshot at index {} (term {})",
            snapshot_index, snapshot_term
        )));

        Ok(())
    }

    fn send_messages(&self, messages: Vec<Message>) {
        if messages.is_empty() { return; }
        debug!(self.logger, "Sending messages"; "count" => messages.len());

        for msg in messages {
            let to = msg.to;
            if let Err(e) = self.transport.send(to, msg) {
                warn!(self.logger, "Failed to send message"; "peer" => to, "error" => format!("{}", e));
            }
        }
    }

    fn apply_committed_entries(&mut self, entries: Vec<Entry>) -> Result<(), RaftLoopError> {
        for entry in entries {
            if entry.data.is_empty() { continue; }

            match entry.get_entry_type() {
                EntryType::EntryNormal => {
                    match self.kv_node.apply_kv_command(&entry.data) {
                        Ok(_) => {
                            debug!(self.logger, "Applied KV command"; "index" => entry.index);
                            self.notify_kv_update(&entry);
                        }
                        Err(e) => {
                            warn!(self.logger, "Failed to apply KV command"; "error" => format!("{}", e));
                            if let Some(callback) = self.raft_node.take_callback(&entry.context) {
                                let _ = callback.send(CommandResponse::Error(format!("Apply failed: {}", e)));
                            }
                        }
                    }
                    if let Err(e) = self.raft_node.raw_node_mut().mut_store().set_applied_index(entry.index) {
                        error!(self.logger, "Failed to set applied index"; "error" => format!("{}", e));
                        return Err(RaftLoopError::StorageError(e));
                    }
                }
                EntryType::EntryConfChange | EntryType::EntryConfChangeV2 => {
                    info!(self.logger, "Applying configuration change"; "index" => entry.index);
                    if let Err(e) = self.raft_node.raw_node_mut().mut_store().set_applied_index(entry.index) {
                        error!(self.logger, "Failed to set applied index"; "error" => format!("{}", e));
                        return Err(RaftLoopError::StorageError(e));
                    }
                }
            }
        }
        Ok(())
    }

    fn notify_kv_update(&mut self, entry: &Entry) {
        if let Ok(cmd) = crate::codec::decode::<crate::kvproto::KvCommand>(&entry.data) {
            if let Some(crate::kvproto::kv_command::Cmd::Put(put)) = cmd.cmd {
                let _ = self.state_tx.send(StateUpdate::KvUpdate {
                    key: put.key.clone(),
                    value: put.value.clone(),
                });
                let _ = self.state_tx.send(StateUpdate::LogEntry {
                    index: entry.index,
                    term: entry.term,
                    data: format!("PUT {} = {}", put.key, put.value),
                });

                if let Some(callback) = self.raft_node.take_callback(&entry.context) {
                    let _ = callback.send(CommandResponse::Success {
                        key: put.key,
                        value: put.value,
                    });
                }
            }
        }
    }

    fn update_tui_state(&mut self) {
        let raft_state = self.raft_node.get_state();

        if let Some(prev_leader) = self.previous_leader_id {
            if prev_leader != raft_state.leader_id && raft_state.leader_id != 0 {
                let msg = if prev_leader == 0 {
                    format!("New leader elected: {} (term {})", raft_state.leader_id, raft_state.term)
                } else {
                    format!("Leadership changed: {} → {} (term {})", prev_leader, raft_state.leader_id, raft_state.term)
                };
                info!(self.logger, "Leadership changed"; "new_leader" => raft_state.leader_id);
                let _ = self.state_tx.send(StateUpdate::SystemMessage(msg));
            }
        }
        self.previous_leader_id = Some(raft_state.leader_id);

        let _ = self.state_tx.send(StateUpdate::RaftState(raft_state));
    }
}


