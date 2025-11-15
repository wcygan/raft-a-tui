use raft_a_tui::codec::{decode, encode};
use raft_a_tui::kvproto::{KvCommand, Put, kv_command};

#[test]
fn roundtrip_put_proto() {
    let original = KvCommand {
        cmd: Some(kv_command::Cmd::Put(Put {
            key: "hello".into(),
            value: "world".into(),
        })),
    };

    let bytes = encode(&original);
    let decoded: KvCommand = decode(&bytes).expect("decode");

    match decoded.cmd.unwrap() {
        kv_command::Cmd::Put(p) => {
            assert_eq!(p.key, "hello");
            assert_eq!(p.value, "world");
        }
    }
}
