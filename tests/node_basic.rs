use raft_a_tui::commands::UserCommand;
use raft_a_tui::node::Node;
use raft_a_tui::node::NodeOutput::Text;

#[test]
fn test_put_and_get() {
    let mut node = Node::new();

    let out = node.apply_user_command(UserCommand::Put {
        key: "a".into(),
        value: "1".into(),
    });
    assert_eq!(out, Text("OK: set a = 1".into()));

    let out = node.apply_user_command(UserCommand::Get { key: "a".into() });
    assert_eq!(out, Text("Some(\"1\")".into()));
}

#[test]
fn test_keys_sorted() {
    let mut node = Node::new();

    node.apply_user_command(UserCommand::Put {
        key: "b".into(),
        value: "2".into(),
    });
    node.apply_user_command(UserCommand::Put {
        key: "a".into(),
        value: "1".into(),
    });
    node.apply_user_command(UserCommand::Put {
        key: "c".into(),
        value: "3".into(),
    });

    let out = node.apply_user_command(UserCommand::Keys);
    assert_eq!(out, Text("[\"a\", \"b\", \"c\"]".into()));
}

#[test]
fn test_status() {
    let mut node = Node::new();
    let out = node.apply_user_command(UserCommand::Status);
    assert_eq!(out, Text("STATUS: standalone node".into()));
}

#[test]
fn test_campaign_noop() {
    let mut node = Node::new();
    let out = node.apply_user_command(UserCommand::Campaign);
    assert_eq!(out, Text("CAMPAIGN ignored (no Raft yet)".into()));
}

#[test]
fn test_apply_kv_command_via_protobuf() {
    let mut node = Node::new();

    // prost-based encoded KvCommand::Put
    let data = Node::encode_put_command("hello", "world");
    node.apply_kv_command(&data).unwrap();

    assert_eq!(node.get_internal_map().get("hello"), Some(&"world".into()));
}
