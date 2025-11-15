pub mod kvproto {
    include!(concat!(env!("OUT_DIR"), "/kvraft.rs"));
}

pub mod codec;
pub mod commands;
pub mod network;
pub mod node;
pub mod raft_loop;
pub mod raft_node;
pub mod storage;

// Re-export key types for convenience
pub use raft_loop::StateUpdate;
