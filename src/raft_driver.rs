use raft::prelude::*;
use raft::{Config, RawNode};
use raft::storage::MemStorage;

use crate::node::Node;
use crate::codec::{encode, decode};
use crate::kvproto::{KvCommand};

/// A thin wrapper around RawNode + your KV Node.
///
/// This struct manages a *single-node* raft cluster.
/// No networking, no transport, no multi-node simulation yet.
pub struct RaftDriver {
    raw: RawNode<MemStorage>,
    pub kv: Node,
}

impl RaftDriver {
    /// Build a single-node Raft with an empty KV state machine.
    pub fn new(id: u64) -> Self {
        // Configure raft
        let mut cfg = Config::default();
        cfg.id = id;
        cfg.election_tick = 10;
        cfg.heartbeat_tick = 3;
        cfg.max_inflight_msgs = 256;

        let storage = MemStorage::new();
        let raw = RawNode::new(&cfg, storage, &raft::default_logger()).unwrap();

        Self {
            raw,
            kv: Node::new(),
        }
    }

    /// Tick the Raft state machine. Should be called periodically.
    pub fn tick(&mut self) {
        self.raw.tick();
    }

    /// Propose a new KvCommand::Put
    pub fn propose_put(&mut self, key: &str, value: &str) {
        let data = Node::encode_put_command(key, value);
        self.raw.propose(vec![], data).unwrap();
    }

    /// Drive the state machine:
    /// - Process Ready
    /// - Apply committed entries to the KV node
    ///   (In multi-node world, send messages and handle them too).
    pub fn on_ready(&mut self) {
        if !self.raw.has_ready() {
            return;
        }

        let mut ready = self.raw.ready();

        // For single-node, no outgoing messages
        // (In multi-node we'll handle ready.messages here)

        // Apply committed entries
        for entry in ready.take_committed_entries() {
            if entry.data.is_empty() {
                continue;
            }
            let cmd: KvCommand = decode(&entry.data).unwrap();
            self.apply_kv_command(cmd);
        }

        // Advance RawNode's internal state
        self.raw.advance(ready);
    }

    /// Apply an already-decoded KvCommand into the Node
    fn apply_kv_command(&mut self, cmd: KvCommand) {
        use crate::kvproto::kv_command::Cmd;

        match cmd.cmd {
            Some(Cmd::Put(p)) => {
                self.kv
                    .apply_kv_command(&Node::encode_put_command(&p.key, &p.value))
                    .unwrap();
            }
            None => {}
        }
    }
}
