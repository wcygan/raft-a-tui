/// Commands entered by the user in the TUI/REPL.
/// Only `Put` mutates state and will eventually become a protobuf (KvCommand).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserCommand {
    Put { key: String, value: String },
    Get { key: String },
    Keys,
    Status,
    Campaign,
}

/// Simple parser for REPL commands.
pub fn parse_command(input: &str) -> Option<UserCommand> {
    let mut parts = input.split_whitespace();
    let cmd = parts.next()?.to_ascii_uppercase();

    match cmd.as_str() {
        "PUT" => {
            let key = parts.next()?.to_string();
            let value = parts.collect::<Vec<_>>().join(" ");
            if value.is_empty() {
                return None;
            }
            Some(UserCommand::Put { key, value })
        }

        "GET" => {
            let key = parts.next()?.to_string();
            Some(UserCommand::Get { key })
        }

        "KEYS" => Some(UserCommand::Keys),
        "STATUS" => Some(UserCommand::Status),
        "CAMPAIGN" => Some(UserCommand::Campaign),

        _ => None,
    }
}
