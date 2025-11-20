use std::collections::BTreeMap;

use crate::codec::{decode, encode};
use crate::commands::UserCommand;
use raft_proto::kvraft::{kv_command, KvCommand, Put};

/// Output from a user command.
#[derive(Debug, PartialEq, Eq)]
pub enum NodeOutput {
    Text(String),
    None,
}

pub struct Node {
    kv: BTreeMap<String, String>,
}

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

impl Node {
    /// Create a new KV node with empty state.
    pub fn new() -> Self {
        Self {
            kv: BTreeMap::new(),
        }
    }

    /// Apply a *user* command (non-Raft). These are ephemeral REPL actions.
    /// - PUT mutates the local KV store (later: propose to Raft)
    /// - GET / KEYS / STATUS read local state
    /// - CAMPAIGN will integrate with Raft later; it's a no-op for now
    pub fn apply_user_command(&mut self, cmd: UserCommand) -> NodeOutput {
        match cmd {
            UserCommand::Put { key, value } => {
                self.kv.insert(key.clone(), value.clone());
                NodeOutput::Text(format!("OK: set {} = {}", key, value))
            }

            UserCommand::Get { key } => {
                let value = self.kv.get(&key).cloned();
                NodeOutput::Text(format!("{:?}", value))
            }

            UserCommand::Keys => {
                let mut keys: Vec<_> = self.kv.keys().cloned().collect();
                keys.sort();
                NodeOutput::Text(format!("{:?}", keys))
            }

            UserCommand::Status => {
                // Placeholder for Raft info (term, role, leader)
                NodeOutput::Text("STATUS: standalone node".to_string())
            }

            UserCommand::Campaign => {
                // No-op until Raft is integrated
                NodeOutput::Text("CAMPAIGN ignored (no Raft yet)".into())
            }
        }
    }

    /// Apply a *replicated* command (future Raft integration).
    /// In Raft, every committed log entry is a prost-encoded KvCommand.
    pub fn apply_kv_command(&mut self, data: &[u8]) -> Result<(), prost::DecodeError> {
        let cmd: KvCommand = decode(data)?;
        if let Some(kv_command::Cmd::Put(Put { key, value })) = cmd.cmd {
            self.kv.insert(key, value);
        }
        Ok(())
    }

    /// Helper to encode a KvCommand::Put entry for Raft.
    pub fn encode_put_command(key: &str, value: &str) -> Vec<u8> {
        use kv_command::Cmd;
        let put = Put {
            key: key.into(),
            value: value.into(),
        };
        let cmd = KvCommand {
            cmd: Some(Cmd::Put(put)),
        };
        encode(&cmd)
    }

    /// Helper for tests (and later debug UI)
    pub fn get_internal_map(&self) -> &BTreeMap<String, String> {
        &self.kv
    }

    /// Create a snapshot of the current KV state.
    ///
    /// Returns bincode-encoded BTreeMap<String, String>.
    /// This snapshot can be used to restore state after a crash or to
    /// bring a new node up to date with the cluster.
    ///
    /// # Returns
    /// - `Ok(Vec<u8>)` - Serialized snapshot data
    /// - `Err(...)` - Serialization error (should never happen for BTreeMap)
    pub fn create_snapshot(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        bincode::encode_to_vec(&self.kv, bincode::config::standard()).map_err(|e| e.into())
    }

    /// Restore KV state from a snapshot.
    ///
    /// Replaces ALL current state with the snapshot data. This is the correct
    /// Raft semantics - a snapshot is an authoritative point-in-time state.
    ///
    /// # Arguments
    /// * `data` - Bincode-encoded BTreeMap<String, String> from create_snapshot()
    ///
    /// # Returns
    /// - `Ok(())` - State successfully restored
    /// - `Err(...)` - Deserialization error (malformed snapshot data)
    ///
    /// # Behavior
    /// - Empty snapshot (0 bytes) → clears all state (empty KV store)
    /// - Non-empty snapshot → replaces all state with snapshot contents
    pub fn restore_from_snapshot(&mut self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        if data.is_empty() {
            // Empty snapshot = empty state
            self.kv.clear();
            return Ok(());
        }

        let (restored, _bytes_read): (BTreeMap<String, String>, usize) =
            bincode::decode_from_slice(data, bincode::config::standard())?;

        // Replace entire state (correct Raft semantics)
        self.kv = restored;
        Ok(())
    }
}
