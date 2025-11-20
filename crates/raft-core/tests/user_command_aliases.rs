use raft_core::commands::{UserCommand, parse_command};

#[test]
fn test_put_alias() {
    assert_eq!(
        parse_command("p foo bar"),
        Some(UserCommand::Put {
            key: "foo".into(),
            value: "bar".into()
        })
    );
}

#[test]
fn test_get_alias() {
    assert_eq!(
        parse_command("g alpha"),
        Some(UserCommand::Get {
            key: "alpha".into()
        })
    );
}

#[test]
fn test_keys_alias() {
    assert_eq!(parse_command("k"), Some(UserCommand::Keys));
}

#[test]
fn test_status_alias() {
    assert_eq!(parse_command("s"), Some(UserCommand::Status));
}

#[test]
fn test_campaign_alias() {
    assert_eq!(parse_command("c"), Some(UserCommand::Campaign));
}

#[test]
fn long_and_short_forms_equivalence() {
    assert_eq!(parse_command("PUT foo bar"), parse_command("p foo bar"),);
    assert_eq!(parse_command("GET aaa"), parse_command("g aaa"),);
    assert_eq!(parse_command("KEYS"), parse_command("k"));
    assert_eq!(parse_command("STATUS"), parse_command("s"));
    assert_eq!(parse_command("CAMPAIGN"), parse_command("c"));
}
