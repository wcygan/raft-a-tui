use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};

use clap::Parser;

/// Raft-a-TUI: Interactive Raft consensus playground
#[derive(Parser, Debug)]
#[command(name = "raft-a-tui")]
#[command(about = "Educational Raft consensus implementation with TUI visualization")]
#[command(version)]
pub struct Args {
    /// Node ID (must be unique in cluster, typically 1, 2, or 3)
    #[arg(long)]
    pub id: u64,

    /// Peer addresses in format: 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
    ///
    /// Each peer is specified as ID=HOST:PORT, separated by commas.
    /// The list must include this node's own ID and address.
    #[arg(long)]
    pub peers: String,

    /// Data directory for persistent storage.
    ///
    /// If not specified, defaults to "./data/node-{id}".
    /// Use "--data-dir :memory:" for in-memory storage (no persistence).
    #[arg(long)]
    pub data_dir: Option<String>,
}

/// Parse peer addresses from CLI argument format.
///
/// # Format
/// `1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003`
///
/// # Returns
/// HashMap mapping node ID to SocketAddr, or an error if parsing fails.
///
/// # Errors
/// - Invalid format (missing '=' or ':')
/// - Invalid node ID (not a valid u64)
/// - Invalid address (cannot resolve to SocketAddr)
///
/// # Example
/// ```
/// use raft_server::cli::parse_peers;
///
/// let peers = parse_peers("1=127.0.0.1:6001,2=127.0.0.1:6002").unwrap();
/// assert_eq!(peers.len(), 2);
/// ```
pub fn parse_peers(peers_str: &str) -> Result<HashMap<u64, SocketAddr>, String> {
    let mut peers = HashMap::new();

    for peer_entry in peers_str.split(',') {
        let peer_entry = peer_entry.trim();

        if peer_entry.is_empty() {
            continue;
        }

        // Split on '=' to get ID and address
        let parts: Vec<&str> = peer_entry.split('=').collect();
        if parts.len() != 2 {
            return Err(format!(
                "Invalid peer format '{}': expected ID=HOST:PORT",
                peer_entry
            ));
        }

        // Parse node ID
        let id: u64 = parts[0].parse().map_err(|e| {
            format!(
                "Invalid node ID '{}' in peer '{}': {}",
                parts[0], peer_entry, e
            )
        })?;

        // Parse socket address
        let addr_str = parts[1];
        let addr: SocketAddr = addr_str
            .to_socket_addrs()
            .map_err(|e| {
                format!(
                    "Invalid address '{}' in peer '{}': {}",
                    addr_str, peer_entry, e
                )
            })?
            .next()
            .ok_or_else(|| {
                format!(
                    "Could not resolve address '{}' in peer '{}'",
                    addr_str, peer_entry
                )
            })?;

        // Check for duplicate IDs
        if peers.insert(id, addr).is_some() {
            return Err(format!("Duplicate node ID {} in peer list", id));
        }
    }

    if peers.is_empty() {
        return Err("Peer list is empty".to_string());
    }

    Ok(peers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_peers_valid_single() {
        let peers = parse_peers("1=127.0.0.1:6001").unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(
            peers.get(&1).unwrap().to_string(),
            "127.0.0.1:6001"
        );
    }

    #[test]
    fn test_parse_peers_valid_multiple() {
        let peers = parse_peers("1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003").unwrap();
        assert_eq!(peers.len(), 3);
        assert_eq!(peers.get(&1).unwrap().to_string(), "127.0.0.1:6001");
        assert_eq!(peers.get(&2).unwrap().to_string(), "127.0.0.1:6002");
        assert_eq!(peers.get(&3).unwrap().to_string(), "127.0.0.1:6003");
    }

    #[test]
    fn test_parse_peers_with_whitespace() {
        let peers = parse_peers(" 1=127.0.0.1:6001 , 2=127.0.0.1:6002 ").unwrap();
        assert_eq!(peers.len(), 2);
    }

    #[test]
    fn test_parse_peers_invalid_format_missing_equals() {
        let result = parse_peers("1-127.0.0.1:6001");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected ID=HOST:PORT"));
    }

    #[test]
    fn test_parse_peers_invalid_id() {
        let result = parse_peers("abc=127.0.0.1:6001");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid node ID"));
    }

    #[test]
    fn test_parse_peers_invalid_address() {
        let result = parse_peers("1=invalid:6001");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid address"));
    }

    #[test]
    fn test_parse_peers_duplicate_id() {
        let result = parse_peers("1=127.0.0.1:6001,1=127.0.0.1:6002");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Duplicate node ID"));
    }

    #[test]
    fn test_parse_peers_empty() {
        let result = parse_peers("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn test_parse_peers_localhost_variants() {
        // Test that localhost resolves correctly
        let peers = parse_peers("1=localhost:6001").unwrap();
        assert_eq!(peers.len(), 1);
        // Should resolve to either 127.0.0.1 or ::1 depending on system
        assert!(peers.contains_key(&1));
    }
}
