pub mod kvraft {
    include!(concat!(env!("OUT_DIR"), "/kvraft.rs"));
}

pub mod rpc {
    include!(concat!(env!("OUT_DIR"), "/rpc.rs"));
}
