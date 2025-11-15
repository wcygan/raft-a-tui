// Template: Raft Ready Loop Pattern
//
// This template provides the canonical Ready loop structure for raft-rs integration.
// Replace placeholder types and logic with your actual implementation.

use crossbeam_channel::{Receiver, RecvTimeoutError};
use raft::prelude::*;
use std::time::{Duration, Instant};

/// Messages received by the Raft event loop
pub enum RaftMessage {
    /// Raft protocol message from peer
    Peer(Message),

    /// Client proposal with callback context
    Proposal {
        data: Vec<u8>,
        callback_id: u64,
    },

    /// Graceful shutdown signal
    Shutdown,
}

/// Main Raft event loop - processes messages and drives consensus
///
/// # Arguments
/// * `raw_node` - Initialized RawNode instance
/// * `msg_rx` - Channel receiving RaftMessage from network/clients
/// * `transport` - Network layer for sending messages to peers
///
/// # Returns
/// Result indicating success or error during operation
pub fn raft_ready_loop<T: Transport>(
    mut raw_node: RawNode<impl Storage>,
    msg_rx: Receiver<RaftMessage>,
    mut transport: T,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tick_timer = Instant::now();
    let tick_interval = Duration::from_millis(100);

    loop {
        // ═══════════════════════════════════════════════════════════════
        // PHASE 1: INPUT RECEPTION
        // ═══════════════════════════════════════════════════════════════
        // Handle incoming messages with timeout to allow periodic ticking

        match msg_rx.recv_timeout(tick_interval) {
            Ok(RaftMessage::Peer(msg)) => {
                // Advance Raft state machine with peer message
                raw_node.step(msg)?;
            }

            Ok(RaftMessage::Proposal { data, callback_id }) => {
                // Propose new entry for replication
                // Use callback_id as proposal context for response routing
                let context = callback_id.to_le_bytes().to_vec();
                raw_node.propose(context, data)?;
            }

            Ok(RaftMessage::Shutdown) => {
                println!("Shutdown signal received, exiting ready loop");
                break;
            }

            Err(RecvTimeoutError::Timeout) => {
                // No message received - continue to tick handling
            }

            Err(RecvTimeoutError::Disconnected) => {
                println!("Message channel disconnected, exiting ready loop");
                break;
            }
        }

        // Tick the Raft state machine at regular intervals
        if tick_timer.elapsed() >= tick_interval {
            raw_node.tick();
            tick_timer = Instant::now();
        }

        // ═══════════════════════════════════════════════════════════════
        // PHASE 2: READY CHECK
        // ═══════════════════════════════════════════════════════════════
        // Check if Raft has state changes requiring action

        if !raw_node.has_ready() {
            continue; // No work to do, loop back to message reception
        }

        let mut ready = raw_node.ready();

        // ═══════════════════════════════════════════════════════════════
        // PHASE 3: READY PROCESSING
        // ═══════════════════════════════════════════════════════════════
        // Process state changes in EXACT order (critical for correctness!)

        // ─────────────────────────────────────────────────────────────
        // Step 3.1: Apply Snapshot (if present)
        // ─────────────────────────────────────────────────────────────
        // Must happen BEFORE appending entries (snapshot may truncate log)

        if !ready.snapshot().is_empty() {
            let snapshot = ready.snapshot();
            println!(
                "Applying snapshot: index={}, term={}",
                snapshot.get_metadata().index,
                snapshot.get_metadata().term
            );

            // Apply snapshot to storage
            // TODO: Implement snapshot application
            // storage.apply_snapshot(snapshot)?;
        }

        // ─────────────────────────────────────────────────────────────
        // Step 3.2: Append Log Entries
        // ─────────────────────────────────────────────────────────────
        // Must happen BEFORE HardState update (log must exist before commit advances)

        if !ready.entries().is_empty() {
            let entries = ready.entries();
            println!("Appending {} log entries", entries.len());

            // Append entries to stable storage
            // TODO: Implement entry persistence
            // storage.append(entries)?;
        }

        // ─────────────────────────────────────────────────────────────
        // Step 3.3: Persist HardState
        // ─────────────────────────────────────────────────────────────
        // Critical: Must persist term/vote/commit before sending messages

        if let Some(hs) = ready.hs() {
            println!(
                "Persisting HardState: term={}, vote={}, commit={}",
                hs.term, hs.vote, hs.commit
            );

            // Persist HardState atomically
            // TODO: Implement HardState persistence
            // storage.set_hardstate(hs.clone())?;

            // If must_sync is true, fsync before sending persisted messages
            if ready.must_sync() {
                // TODO: Implement fsync
                // storage.sync()?;
            }
        }

        // ─────────────────────────────────────────────────────────────
        // Step 3.4: Send Messages to Peers
        // ─────────────────────────────────────────────────────────────
        // Can only send AFTER persistence completes (safety requirement)

        for msg in ready.take_messages() {
            println!(
                "Sending {:?} to node {} (term {})",
                msg.msg_type, msg.to, msg.term
            );

            // Send message via network transport
            transport.send(msg)?;
        }

        // Send persisted messages (only after fsync if must_sync was true)
        for msg in ready.take_persisted_messages() {
            println!(
                "Sending persisted {:?} to node {}",
                msg.msg_type, msg.to
            );
            transport.send(msg)?;
        }

        // ─────────────────────────────────────────────────────────────
        // Step 3.5: Apply Committed Entries to State Machine
        // ─────────────────────────────────────────────────────────────
        // Safe to apply only after replication and persistence

        for entry in ready.take_committed_entries() {
            if entry.data.is_empty() {
                // Empty entry from new leader election - safe to skip
                continue;
            }

            match entry.get_entry_type() {
                EntryType::EntryNormal => {
                    println!(
                        "Applying committed entry: index={}, term={}",
                        entry.index, entry.term
                    );

                    // Apply entry to state machine
                    // TODO: Implement state machine application
                    // state_machine.apply(&entry.data)?;

                    // If this entry has a proposal context, invoke callback
                    if !entry.context.is_empty() {
                        let callback_id = u64::from_le_bytes(
                            entry.context[..8].try_into().unwrap_or([0; 8])
                        );
                        println!("Entry applied, notifying callback {}", callback_id);
                        // TODO: Invoke callback with response
                        // callbacks.notify(callback_id, result)?;
                    }
                }

                EntryType::EntryConfChange => {
                    println!("Applying configuration change");

                    // Decode and apply configuration change
                    let cc = ConfChange::decode(&entry.data)?;

                    // Apply to RawNode
                    let cs = raw_node.apply_conf_change(&cc)?;

                    println!("New configuration: voters={:?}", cs.voters);

                    // Update local state
                    // TODO: Update peer list, transport connections, etc.
                }

                EntryType::EntryConfChangeV2 => {
                    println!("Applying configuration change v2");

                    // Decode and apply v2 configuration change
                    let cc = ConfChangeV2::decode(&entry.data)?;
                    let cs = raw_node.apply_conf_change(&cc)?;

                    println!("New configuration: voters={:?}", cs.voters);

                    // TODO: Update peer list
                }
            }
        }

        // ─────────────────────────────────────────────────────────────
        // Step 3.6: Advance to Next Ready State
        // ─────────────────────────────────────────────────────────────
        // Signal that current Ready has been fully processed

        let mut light_ready = raw_node.advance(ready);

        // ═══════════════════════════════════════════════════════════════
        // PHASE 4: LIGHT READY PROCESSING
        // ═══════════════════════════════════════════════════════════════
        // Handle additional messages and commits returned from advance()

        // Send any additional messages
        for msg in light_ready.take_messages() {
            transport.send(msg)?;
        }

        // Apply any additional committed entries
        for entry in light_ready.take_committed_entries() {
            if !entry.data.is_empty() {
                println!(
                    "Applying light_ready entry: index={}",
                    entry.index
                );
                // TODO: Apply to state machine (same as Step 3.5)
                // state_machine.apply(&entry.data)?;
            }
        }

        // Commit light_ready changes
        raw_node.advance_apply();
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════
// Transport Trait
// ═══════════════════════════════════════════════════════════════════════
// Abstract interface for network communication

pub trait Transport {
    /// Send a Raft message to the specified peer
    fn send(&mut self, msg: Message) -> Result<(), Box<dyn std::error::Error>>;
}

// ═══════════════════════════════════════════════════════════════════════
// Example: Local Transport using Crossbeam Channels
// ═══════════════════════════════════════════════════════════════════════

use crossbeam_channel::Sender;
use std::collections::HashMap;

pub struct LocalTransport {
    /// Map peer_id -> message sender
    peers: HashMap<u64, Sender<Message>>,
}

impl LocalTransport {
    pub fn new(peers: HashMap<u64, Sender<Message>>) -> Self {
        Self { peers }
    }
}

impl Transport for LocalTransport {
    fn send(&mut self, msg: Message) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(sender) = self.peers.get(&msg.to) {
            sender.send(msg)?;
            Ok(())
        } else {
            Err(format!("Unknown peer: {}", msg.to).into())
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Usage Example
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod example {
    use super::*;

    #[test]
    fn example_ready_loop_setup() {
        // This is a template - replace with actual implementation

        // 1. Create storage
        // let storage = MemStorage::new();

        // 2. Configure RawNode
        // let config = Config {
        //     id: 1,
        //     election_tick: 10,
        //     heartbeat_tick: 3,
        //     ..Default::default()
        // };

        // 3. Create RawNode
        // let raw_node = RawNode::new(&config, storage, &logger)?;

        // 4. Setup communication channels
        // let (msg_tx, msg_rx) = crossbeam_channel::unbounded();

        // 5. Create transport
        // let peers = HashMap::new(); // peer_id -> sender mapping
        // let transport = LocalTransport::new(peers);

        // 6. Spawn Ready loop thread
        // std::thread::spawn(move || {
        //     raft_ready_loop(raw_node, msg_rx, transport)
        // });

        // 7. Send proposals via msg_tx
        // msg_tx.send(RaftMessage::Proposal {
        //     data: encode_command("PUT", "key", "value"),
        //     callback_id: 123,
        // })?;
    }
}
