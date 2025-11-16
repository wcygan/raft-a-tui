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
pub fn raft_ready_loop<T: Transport>(
    mut raft_node: RaftNode,
    mut kv_node: Node,
    cmd_rx: Receiver<UserCommand>,
    msg_rx: Receiver<Message>,
    state_tx: Sender<StateUpdate>,
    transport: T,
    shutdown_rx: Receiver<()>,
) -> Result<(), RaftLoopError> {
    let logger = raft_node.logger().clone();
    info!(logger, "Raft ready loop starting"; "node_id" => raft_node.get_state().node_id);

    let tick_duration = Duration::from_millis(100);
    let mut tick_timer = Instant::now();
    let mut previous_leader_id: Option<u64> = None;

    loop {
        // Calculate timeout until next tick
        let timeout = tick_duration.saturating_sub(tick_timer.elapsed());

        // Phase 1: Input Reception
        // Wait for input with timeout using crossbeam select!
        select! {
            recv(cmd_rx) -> result => {
                match result {
                    Ok(cmd) => {
                        debug!(logger, "Received user command"; "command" => format!("{:?}", cmd));

                        // Handle different command types
                        match cmd {
                            UserCommand::Put { key, value } => {
                                // Propose command to Raft
                                match raft_node.propose_command(key.clone(), value.clone()) {
                                    Ok(rx) => {
                                        debug!(logger, "Proposed PUT command"; "key" => &key, "value" => &value);
                                        let _ = state_tx.send(StateUpdate::SystemMessage(
                                            format!("Proposed: PUT {} = {}", key, value)
                                        ));
                                        // The callback will be invoked when committed
                                        // For now, we just drop the receiver (fire and forget)
                                        // In a full implementation, we'd track this for client responses
                                        drop(rx);
                                    }
                                    Err(raft::Error::ProposalDropped) => {
                                        // Not the leader - provide helpful redirect message
                                        let state = raft_node.get_state();
                                        let leader_hint = if state.leader_id == 0 {
                                            "no leader elected yet".to_string()
                                        } else {
                                            format!("leader is node {}", state.leader_id)
                                        };

                                        warn!(logger, "Rejected PUT - not leader";
                                              "key" => &key,
                                              "role" => format!("{:?}", state.role),
                                              "leader_id" => state.leader_id);

                                        let _ = state_tx.send(StateUpdate::SystemMessage(
                                            format!("ERROR: Not leader ({}) - writes must go to leader", leader_hint)
                                        ));
                                    }
                                    Err(e) => {
                                        warn!(logger, "Failed to propose command"; "error" => format!("{}", e));
                                        // Send error message to TUI
                                        let _ = state_tx.send(StateUpdate::SystemMessage(
                                            format!("Error: Failed to propose PUT ({})", e)
                                        ));
                                    }
                                }
                            }
                            UserCommand::Campaign => {
                                // Trigger election by calling campaign()
                                if let Err(e) = raft_node.raw_node_mut().campaign() {
                                    warn!(logger, "Failed to campaign"; "error" => format!("{}", e));
                                } else {
                                    info!(logger, "Starting election campaign");
                                    let _ = state_tx.send(StateUpdate::SystemMessage(
                                        "Starting election campaign".to_string()
                                    ));
                                }
                            }
                            // STATUS displays current Raft state
                            UserCommand::Status => {
                                let state = raft_node.get_state();
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
                                let _ = state_tx.send(StateUpdate::SystemMessage(status_msg));
                            }
                            // Other commands (GET, KEYS) are read-only
                            // They don't go through Raft - just read local state
                            UserCommand::Get { .. } | UserCommand::Keys => {
                                debug!(logger, "Read-only command, applying locally");
                                // Apply locally (doesn't mutate Raft state)
                                let output = kv_node.apply_user_command(cmd);

                                // Send output back to TUI
                                if let crate::node::NodeOutput::Text(text) = output {
                                    let _ = state_tx.send(StateUpdate::SystemMessage(text));
                                }
                            }
                        }
                    }
                    Err(_) => {
                        // cmd_rx closed - shutdown
                        info!(logger, "Command channel closed, shutting down");
                        return Err(RaftLoopError::ChannelClosed("cmd_rx".to_string()));
                    }
                }
            },
            recv(msg_rx) -> result => {
                match result {
                    Ok(msg) => {
                        debug!(logger, "Received Raft message";
                               "from" => msg.from,
                               "to" => msg.to,
                               "msg_type" => format!("{:?}", msg.msg_type()));

                        // Step the Raft state machine with the message
                        if let Err(e) = raft_node.raw_node_mut().step(msg) {
                            warn!(logger, "Failed to step Raft message"; "error" => format!("{}", e));
                            // Log but continue - step() errors are non-fatal per ROADMAP
                        }
                    }
                    Err(_) => {
                        // msg_rx closed - shutdown
                        info!(logger, "Message channel closed, shutting down");
                        return Err(RaftLoopError::ChannelClosed("msg_rx".to_string()));
                    }
                }
            },
            recv(shutdown_rx) -> _ => {
                // Graceful shutdown requested
                info!(logger, "Shutdown signal received, exiting ready loop");
                return Ok(());
            },
            default(timeout) => {
                // Timeout - no input received, continue to tick check
            },
        }

        // Tick the Raft state machine if enough time has elapsed
        if tick_timer.elapsed() >= tick_duration {
            debug!(logger, "Ticking Raft state machine");
            raft_node.raw_node_mut().tick();
            tick_timer = Instant::now();
        }

        // Phase 2: Ready Check
        if !raft_node.raw_node().has_ready() {
            continue;
        }

        // Phase 3: Ready Processing
        let mut ready = raft_node.raw_node_mut().ready();

        debug!(logger, "Processing Ready state";
               "has_snapshot" => !ready.snapshot().is_empty(),
               "num_entries" => ready.entries().len(),
               "num_messages" => ready.messages().len(),
               "num_persisted_messages" => ready.persisted_messages().len(),
               "num_committed" => ready.committed_entries().len());

        // 1. Apply snapshot if present
        if !ready.snapshot().is_empty() {
            let snapshot = ready.snapshot();
            let snapshot_index = snapshot.get_metadata().index;
            let snapshot_term = snapshot.get_metadata().term;

            info!(logger, "Applying snapshot";
                  "index" => snapshot_index,
                  "term" => snapshot_term);

            // Apply snapshot to storage
            if let Err(e) = raft_node.raw_node_mut().mut_store().apply_snapshot(snapshot.clone()) {
                error!(logger, "Failed to apply snapshot"; "error" => format!("{}", e));
                return Err(RaftLoopError::StorageError(e));
            }

            // Restore KV state machine from snapshot data
            let snapshot_data = snapshot.get_data();
            if let Err(e) = kv_node.restore_from_snapshot(snapshot_data) {
                error!(logger, "Failed to restore KV state from snapshot";
                       "error" => format!("{}", e),
                       "snapshot_index" => snapshot_index);
                return Err(RaftLoopError::Other(format!(
                    "Snapshot restore failed at index {}: {}",
                    snapshot_index, e
                )));
            }

            // Update applied index to snapshot index
            raft_node.raw_node_mut().mut_store().set_applied_index(snapshot_index);

            info!(logger, "Snapshot applied successfully";
                  "index" => snapshot_index,
                  "term" => snapshot_term,
                  "kv_entries" => kv_node.get_internal_map().len());

            // Notify TUI
            let _ = state_tx.send(StateUpdate::SystemMessage(format!(
                "📸 Applied snapshot at index {} (term {})",
                snapshot_index, snapshot_term
            )));
        }

        // 2. Append new log entries to storage
        if !ready.entries().is_empty() {
            debug!(logger, "Appending entries to storage"; "count" => ready.entries().len());
            let entries = ready.entries();

            if let Err(e) = raft_node.raw_node_mut().mut_store().append(entries) {
                error!(logger, "Failed to append entries"; "error" => format!("{}", e));
                return Err(RaftLoopError::StorageError(e));
            }
        }

        // 3. Persist HardState changes (term, vote, commit)
        if let Some(hs) = ready.hs() {
            debug!(logger, "Persisting HardState";
                   "term" => hs.term,
                   "vote" => hs.vote,
                   "commit" => hs.commit);

            raft_node.raw_node_mut().mut_store().set_hardstate(hs.clone());
        }

        // 4. Send regular messages to peers (can be sent before persistence completes)
        let messages = ready.take_messages();
        if !messages.is_empty() {
            debug!(logger, "Sending regular messages to peers"; "count" => messages.len());

            for msg in messages {
                let to = msg.to;
                if let Err(e) = transport.send(to, msg) {
                    warn!(logger, "Failed to send regular message to peer";
                          "peer" => to,
                          "error" => format!("{}", e));
                    // Log but continue - transport errors are non-fatal
                    // Network may be temporarily down
                }
            }
        }

        // 5. Send persisted messages to peers (MUST be sent AFTER persistence!)
        let persisted_messages = ready.take_persisted_messages();
        if !persisted_messages.is_empty() {
            debug!(logger, "Sending persisted messages to peers"; "count" => persisted_messages.len());

            for msg in persisted_messages {
                let to = msg.to;
                if let Err(e) = transport.send(to, msg) {
                    warn!(logger, "Failed to send persisted message to peer";
                          "peer" => to,
                          "error" => format!("{}", e));
                }
            }
        }

        // 6. Apply committed entries to state machine
        let committed_entries = ready.take_committed_entries();
        if !committed_entries.is_empty() {
            for entry in committed_entries {
                debug!(logger, "Processing committed entry";
                       "index" => entry.index,
                       "term" => entry.term);

                // Empty entries can occur during leader election
                if entry.data.is_empty() {
                    debug!(logger, "Skipping empty entry (likely from leader election)");
                    continue;
                }

                match entry.get_entry_type() {
                    EntryType::EntryNormal => {
                        // Decode and apply to KV store
                        match kv_node.apply_kv_command(&entry.data) {
                            Ok(_) => {
                                debug!(logger, "Applied KV command";
                                       "index" => entry.index);

                                // Extract key/value for state update
                                // This is a bit inefficient (decode twice) but keeps code clean
                                // In production, we'd return the key/value from apply_kv_command
                                if let Ok(cmd) = crate::codec::decode::<crate::kvproto::KvCommand>(&entry.data) {
                                    if let Some(crate::kvproto::kv_command::Cmd::Put(put)) = cmd.cmd {
                                        // Send KV update to TUI
                                        let _ = state_tx.send(StateUpdate::KvUpdate {
                                            key: put.key.clone(),
                                            value: put.value.clone(),
                                        });

                                        // Send log entry summary
                                        let _ = state_tx.send(StateUpdate::LogEntry {
                                            index: entry.index,
                                            term: entry.term,
                                            data: format!("PUT {} = {}", put.key, put.value),
                                        });

                                        // Invoke callback if present
                                        if let Some(callback) = raft_node.take_callback(&entry.context) {
                                            let response = CommandResponse::Success {
                                                key: put.key,
                                                value: put.value,
                                            };
                                            let _ = callback.send(response);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(logger, "Failed to apply KV command";
                                      "error" => format!("{}", e),
                                      "index" => entry.index);
                                // Log but continue - apply errors are non-fatal per ROADMAP

                                // Invoke callback with error
                                if let Some(callback) = raft_node.take_callback(&entry.context) {
                                    let response = CommandResponse::Error(format!("Apply failed: {}", e));
                                    let _ = callback.send(response);
                                }
                            }
                        }

                        // Update applied index in storage
                        raft_node.raw_node_mut().mut_store().set_applied_index(entry.index);
                    }
                    EntryType::EntryConfChange => {
                        // Handle cluster configuration change
                        info!(logger, "Applying configuration change"; "index" => entry.index);
                        // TODO: Implement when we support dynamic membership
                        // For now, just update applied index
                        raft_node.raw_node_mut().mut_store().set_applied_index(entry.index);
                    }
                    EntryType::EntryConfChangeV2 => {
                        // Handle cluster configuration change (v2)
                        info!(logger, "Applying configuration change v2"; "index" => entry.index);
                        // TODO: Implement when we support dynamic membership
                        raft_node.raw_node_mut().mut_store().set_applied_index(entry.index);
                    }
                }
            }
        }

        // 7. Send Raft state update to TUI
        let raft_state = raft_node.get_state();

        // Detect leadership changes
        // Only log when we GET a new leader (skip intermediate "lost leader" state)
        if let Some(prev_leader) = previous_leader_id {
            if prev_leader != raft_state.leader_id && raft_state.leader_id != 0 {
                let msg = if prev_leader == 0 {
                    format!("New leader elected: {} (term {})", raft_state.leader_id, raft_state.term)
                } else {
                    format!(
                        "Leadership changed: {} → {} (term {})",
                        prev_leader,
                        raft_state.leader_id,
                        raft_state.term
                    )
                };
                info!(logger, "Leadership changed";
                      "previous_leader" => prev_leader,
                      "new_leader" => raft_state.leader_id,
                      "term" => raft_state.term);
                let _ = state_tx.send(StateUpdate::SystemMessage(msg));
            }
        }
        previous_leader_id = Some(raft_state.leader_id);

        if let Err(_) = state_tx.send(StateUpdate::RaftState(raft_state.clone())) {
            // TUI might be gone, but Raft continues
            debug!(logger, "Failed to send state update (TUI disconnected?)");
        }

        // 8. Advance to next Ready cycle
        let mut light_rd = raft_node.raw_node_mut().advance(ready);

        // Process LightReady (additional messages and committed entries)
        if let Some(commit) = light_rd.commit_index() {
            debug!(logger, "LightReady commit index"; "commit" => commit);
            // Note: We don't need to persist commit index separately here
            // as it's already handled by the Ready processing above
        }

        // Send any remaining messages from LightReady
        if !light_rd.messages().is_empty() {
            debug!(logger, "Sending LightReady messages"; "count" => light_rd.messages().len());

            for msg in light_rd.messages() {
                let to = msg.to;
                if let Err(e) = transport.send(to, msg.clone()) {
                    warn!(logger, "Failed to send LightReady message";
                          "peer" => to,
                          "error" => format!("{}", e));
                }
            }
        }

        // Apply any committed entries from LightReady
        let light_committed = light_rd.take_committed_entries();
        if !light_committed.is_empty() {
            for entry in light_committed {
                debug!(logger, "Processing LightReady committed entry";
                       "index" => entry.index);

                if entry.data.is_empty() {
                    continue;
                }

                // Same logic as above for committed entries
                match entry.get_entry_type() {
                    EntryType::EntryNormal => {
                        match kv_node.apply_kv_command(&entry.data) {
                            Ok(_) => {
                                debug!(logger, "Applied KV command from LightReady";
                                       "index" => entry.index);

                                if let Ok(cmd) = crate::codec::decode::<crate::kvproto::KvCommand>(&entry.data) {
                                    if let Some(crate::kvproto::kv_command::Cmd::Put(put)) = cmd.cmd {
                                        let _ = state_tx.send(StateUpdate::KvUpdate {
                                            key: put.key.clone(),
                                            value: put.value.clone(),
                                        });

                                        let _ = state_tx.send(StateUpdate::LogEntry {
                                            index: entry.index,
                                            term: entry.term,
                                            data: format!("PUT {} = {}", put.key, put.value),
                                        });

                                        if let Some(callback) = raft_node.take_callback(&entry.context) {
                                            let response = CommandResponse::Success {
                                                key: put.key,
                                                value: put.value,
                                            };
                                            let _ = callback.send(response);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(logger, "Failed to apply KV command from LightReady";
                                      "error" => format!("{}", e));

                                if let Some(callback) = raft_node.take_callback(&entry.context) {
                                    let response = CommandResponse::Error(format!("Apply failed: {}", e));
                                    let _ = callback.send(response);
                                }
                            }
                        }

                        raft_node.raw_node_mut().mut_store().set_applied_index(entry.index);
                    }
                    _ => {
                        raft_node.raw_node_mut().mut_store().set_applied_index(entry.index);
                    }
                }
            }
        }
    }
}
