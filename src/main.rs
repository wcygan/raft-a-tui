pub mod kvproto {
    include!(concat!(env!("OUT_DIR"), "/kvraft.rs"));
}

fn main() {
    println!("Hello, world!");
}
