use raft_a_tui::commands::{UserCommand, parse_command};

#[test]
fn test_put_parsing() {
    assert_eq!(
        parse_command("PUT foo bar"),
        Some(UserCommand::Put {
            key: "foo".into(),
            value: "bar".into()
        })
    );
}

#[test]
fn test_get_parsing() {
    assert_eq!(
        parse_command("GET alpha"),
        Some(UserCommand::Get {
            key: "alpha".into()
        })
    );
}

#[test]
fn test_keys_parsing() {
    assert_eq!(parse_command("KEYS"), Some(UserCommand::Keys));
}
