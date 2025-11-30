use crossbeam_channel::Sender;
use slog::{debug, info, warn, Logger};

use crate::commands::{RpcCommand, ServerCommand, UserCommand};
use crate::node::{Node, NodeOutput};
use crate::raft_loop::{RaftLoopError, StateUpdate};
use crate::raft_node::RaftNode;

/// Result of command handling.
#[derive(Debug)]
pub enum CommandResult {
    Ok,
    Proposed,
    Error(String),
}

/// Handles user (TUI) and RPC commands.
pub struct CommandHandler<'a> {
    raft_node: &'a mut RaftNode,
    kv_node: &'a mut Node,
    state_tx: &'a Sender<StateUpdate>,
    logger: &'a Logger,
}

impl<'a> CommandHandler<'a> {
    pub fn new(
        raft_node: &'a mut RaftNode,
        kv_node: &'a mut Node,
        state_tx: &'a Sender<StateUpdate>,
        logger: &'a Logger,
    ) -> Self {
        Self {
            raft_node,
            kv_node,
            state_tx,
            logger,
        }
    }

    /// Handle a server command (dispatch to user or RPC handler).
    pub fn handle(&mut self, cmd: ServerCommand) -> Result<(), RaftLoopError> {
        match cmd {
            ServerCommand::User(cmd) => self.handle_user_command(cmd),
            ServerCommand::Rpc(cmd) => self.handle_rpc_command(cmd),
        }
    }

    fn handle_rpc_command(&mut self, cmd: RpcCommand) -> Result<(), RaftLoopError> {
        match cmd {
            RpcCommand::Put { key, value, resp } => {
                if !self.raft_node.is_leader() {
                    let state = self.raft_node.get_state();
                    let _ = resp.send(Err(format!("Not leader: {}", state.leader_id)));
                    return Ok(());
                }

                match self
                    .raft_node
                    .propose_command(key.clone(), value.clone(), Some(resp))
                {
                    Ok(_) => {
                        debug!(self.logger, "Proposed RPC PUT"; "key" => &key);
                    }
                    Err(e) => {
                        // If we are here, we thought we were leader but propose failed?
                        // OR propose_command returned error.
                        // Since we passed `resp`, it's gone. The caller (server.rs) gets a channel closed error.
                        warn!(self.logger, "Failed to propose RPC command"; "error" => format!("{}", e));
                    }
                }
            }
            RpcCommand::Get { key, resp } => {
                // Linearizable reads require ReadIndex. For now, return local state (eventually consistent).
                let value = self.kv_node.get_internal_map().get(&key).cloned();
                let _ = resp.send(value);
            }
        }
        Ok(())
    }

    fn handle_user_command(&mut self, cmd: UserCommand) -> Result<(), RaftLoopError> {
        debug!(self.logger, "Received user command"; "command" => format!("{:?}", cmd));

        match cmd {
            UserCommand::Put { key, value } => {
                match self
                    .raft_node
                    .propose_command(key.clone(), value.clone(), None)
                {
                    Ok(_) => {
                        debug!(self.logger, "Proposed PUT command"; "key" => &key, "value" => &value);
                        let _ = self.state_tx.send(StateUpdate::SystemMessage(format!(
                            "Proposed: PUT {} = {}",
                            key, value
                        )));
                        // No callback for TUI
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

                        let _ = self.state_tx.send(StateUpdate::SystemMessage(format!(
                            "ERROR: Not leader ({}) - writes must go to leader",
                            leader_hint
                        )));
                    }
                    Err(e) => {
                        warn!(self.logger, "Failed to propose command"; "error" => format!("{}", e));
                        let _ = self.state_tx.send(StateUpdate::SystemMessage(format!(
                            "Error: Failed to propose PUT ({})",
                            e
                        )));
                    }
                }
            }
            UserCommand::Campaign => {
                if let Err(e) = self.raft_node.raw_node_mut().campaign() {
                    warn!(self.logger, "Failed to campaign"; "error" => format!("{}", e));
                } else {
                    info!(self.logger, "Starting election campaign");
                    let _ = self.state_tx.send(StateUpdate::SystemMessage(
                        "Starting election campaign".to_string(),
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
                if let NodeOutput::Text(text) = output {
                    let _ = self.state_tx.send(StateUpdate::SystemMessage(text));
                }
            }
        }
        Ok(())
    }
}
