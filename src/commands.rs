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

/// Parse REPL input.
///
/// Long forms:
///   PUT <key> <value>
///   GET <key>
///   KEYS
///   STATUS
///   CAMPAIGN
///
/// Short forms:
///   p <key> <value>
///   g <key>
///   k
///   s
///   c
pub fn parse_command(input: &str) -> Option<UserCommand> {
    let mut parts = input.split_whitespace();
    let cmd = parts.next()?.to_ascii_lowercase();

    match cmd.as_str() {
        // PUT / p
        "put" | "p" => {
            let key = parts.next()?.to_string();
            let value = parts.collect::<Vec<_>>().join(" ");
            if value.is_empty() {
                return None;
            }
            Some(UserCommand::Put { key, value })
        }

        // GET / g
        "get" | "g" => {
            let key = parts.next()?.to_string();
            Some(UserCommand::Get { key })
        }

        // KEYS / k
        "keys" | "k" => Some(UserCommand::Keys),

        // STATUS / s
        "status" | "s" => Some(UserCommand::Status),

        // CAMPAIGN / c
        "campaign" | "c" => Some(UserCommand::Campaign),

        _ => None,
    }
}
