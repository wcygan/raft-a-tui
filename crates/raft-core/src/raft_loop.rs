use std::time::{Duration, Instant};

use crossbeam_channel::{select, Receiver, Sender};
use raft::prelude::*;
use slog::{debug, info, warn};
use thiserror::Error;

use crate::command_handler::CommandHandler;
use crate::commands::ServerCommand;
use crate::entry_applicator::KvEntryApplicator;
use crate::network::{Transport, TransportError};
use crate::node::Node;
use crate::raft_node::{RaftNode, RaftState};
use crate::ready_processor::{DefaultReadyProcessor, ReadyProcessor};

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
#[derive(Debug, Error)]
pub enum RaftLoopError {
    /// The command channel (cmd_rx) was closed.
    #[error("Command channel closed unexpectedly")]
    CommandChannelClosed,

    /// The message channel (msg_rx) was closed.
    #[error("Message channel closed unexpectedly")]
    MessageChannelClosed,

    /// Storage operation failed.
    #[error("Storage error: {0}")]
    Storage(#[from] raft::Error),

    /// Transport failed (non-fatal, logged and continued).
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),

    /// Protobuf decode error.
    #[error("Failed to decode protobuf: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),

    /// Other errors with context.
    #[error("{0}")]
    Other(String),
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
    cmd_rx: Receiver<ServerCommand>,
    msg_rx: Receiver<Message>,
    state_tx: Sender<StateUpdate>,
    transport: T,
    shutdown_rx: Receiver<()>,
    logger: slog::Logger,
    previous_leader_id: Option<u64>,
}

impl<T: Transport> RaftDriver<T> {
    /// Private constructor called by RaftDriverBuilder.
    ///
    /// Use `RaftDriverBuilder` to construct a RaftDriver instance.
    pub(crate) fn from_parts(
        raft_node: RaftNode,
        kv_node: Node,
        cmd_rx: Receiver<ServerCommand>,
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
                        Ok(cmd) => self.handle_command(cmd)?,
                        Err(_) => {
                            info!(self.logger, "Command channel closed, shutting down");
                            return Err(RaftLoopError::CommandChannelClosed);
                        }
                    }
                },
                recv(self.msg_rx) -> result => {
                    match result {
                        Ok(msg) => self.handle_raft_message(msg)?,
                        Err(_) => {
                            info!(self.logger, "Message channel closed, shutting down");
                            return Err(RaftLoopError::MessageChannelClosed);
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

    fn handle_command(&mut self, cmd: ServerCommand) -> Result<(), RaftLoopError> {
        let mut handler = CommandHandler::new(
            &mut self.raft_node,
            &mut self.kv_node,
            &self.state_tx,
            &self.logger,
        );
        handler.handle(cmd)
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
        let mut entry_applicator = KvEntryApplicator::new(&mut self.kv_node, &self.state_tx, &self.logger);

        let mut processor = DefaultReadyProcessor::new(
            &mut self.raft_node,
            &self.transport,
            &mut entry_applicator,
            &self.state_tx,
            &self.logger,
            &mut self.previous_leader_id,
        );

        processor.process_ready()
    }
}
