pub mod kvproto {
    include!(concat!(env!("OUT_DIR"), "/kvraft.rs"));
}

pub mod codec;
pub mod commands;
pub mod network;
pub mod node;
pub mod storage;
