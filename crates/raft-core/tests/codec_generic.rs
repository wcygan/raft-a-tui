use raft_core::codec::{decode, encode};
use raft_proto::kvraft::{kv_command, KvCommand, Put};

#[test]
fn generic_encode_decode_works() {
    let msg = KvCommand {
        cmd: Some(kv_command::Cmd::Put(Put {
            key: "x".into(),
            value: "y".into(),
        })),
    };

    let bytes = encode(&msg);
    let decoded: KvCommand = decode(&bytes).unwrap();

    match decoded.cmd.unwrap() {
        kv_command::Cmd::Put(p) => {
            assert_eq!(p.key, "x");
            assert_eq!(p.value, "y");
        }
    }
}
