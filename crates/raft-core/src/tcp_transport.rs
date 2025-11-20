use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use raft::prelude::Message;
use slog::{debug, error, info, warn, Logger};

use crate::network::{Transport, TransportError};

const MAX_RETRIES: usize = 3;
const RETRY_DELAY: Duration = Duration::from_millis(10);

/// TCP-based transport for sending Raft messages between nodes.
///
/// This transport uses TCP connections to send protobuf-encoded Raft messages.
/// Messages are framed with a 4-byte length prefix (big-endian u32).
///
/// # Architecture
/// - **Main thread**: Creates TcpTransport, sends messages via channel
/// - **Listener thread**: Accepts incoming connections, deserializes messages, forwards to msg_rx
/// - **Sender thread**: Dequeues from channel, connects to peer, sends serialized message
///
/// # Message Format
/// ```text
/// [4 bytes: message length (big-endian u32)]
/// [N bytes: prost-encoded raft::Message]
/// ```
pub struct TcpTransport {
    node_id: u64,
    peers: Arc<HashMap<u64, SocketAddr>>,
    sender_tx: Sender<(u64, Message)>,
    logger: Logger,
}

impl TcpTransport {
    /// Create a new TCP transport and start background threads.
    ///
    /// # Arguments
    /// * `node_id` - This node's ID
    /// * `listen_addr` - Address to listen on for incoming connections
    /// * `peers` - Map of peer IDs to their addresses (including self)
    /// * `msg_tx` - Channel to send received messages to the ready loop
    /// * `logger` - Logger for transport events
    ///
    /// # Returns
    /// The transport instance, or an error if binding fails.
    ///
    /// # Background Threads
    /// This constructor spawns two background threads:
    /// 1. Listener thread - accepts connections and forwards messages to msg_tx
    /// 2. Sender thread - sends queued messages to peers
    pub fn new(
        node_id: u64,
        listen_addr: SocketAddr,
        peers: HashMap<u64, SocketAddr>,
        msg_tx: Sender<Message>,
        logger: Logger,
    ) -> io::Result<Self> {
        let (sender_tx, sender_rx) = unbounded();

        let peers_arc = Arc::new(peers);

        // Start listener thread
        let listener_logger = logger.clone();
        let listener_logger_err = logger.clone();
        let listener_peers = Arc::clone(&peers_arc);
        thread::Builder::new()
            .name(format!("tcp-listener-{}", node_id))
            .spawn(move || {
                if let Err(e) =
                    listener_thread(node_id, listen_addr, msg_tx, listener_peers, listener_logger)
                {
                    error!(listener_logger_err, "Listener thread failed"; "error" => format!("{}", e));
                }
            })?;

        // Start sender thread
        let sender_logger = logger.clone();
        let sender_peers = Arc::clone(&peers_arc);
        thread::Builder::new()
            .name(format!("tcp-sender-{}", node_id))
            .spawn(move || {
                sender_thread(node_id, sender_rx, sender_peers, sender_logger);
            })?;

        info!(logger, "TCP transport started";
              "node_id" => node_id,
              "listen_addr" => format!("{}", listen_addr),
              "peer_count" => peers_arc.len());

        Ok(Self {
            node_id,
            peers: peers_arc,
            sender_tx,
            logger,
        })
    }

    /// Get this node's ID.
    pub fn node_id(&self) -> u64 {
        self.node_id
    }

    /// Get the number of peers (including self).
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

impl Transport for TcpTransport {
    fn send(&self, to: u64, mut msg: Message) -> Result<(), TransportError> {
        // raft-rs generates messages with from=0, transport layer must fill in sender ID
        msg.from = self.node_id;

        // Check if peer exists
        if !self.peers.contains_key(&to) {
            warn!(self.logger, "Attempted to send to unknown peer"; "to" => to);
            return Err(TransportError::PeerNotFound(to));
        }

        // Queue message for sender thread
        // If the channel is full or disconnected, that's a critical error
        self.sender_tx.send((to, msg)).map_err(|_| {
            error!(self.logger, "Failed to queue message for sending"; "to" => to);
            TransportError::ChannelClosed(to)
        })
    }
}

/// Listener thread: accepts incoming TCP connections and deserializes messages.
fn listener_thread(
    node_id: u64,
    listen_addr: SocketAddr,
    msg_tx: Sender<Message>,
    peers: Arc<HashMap<u64, SocketAddr>>,
    logger: Logger,
) -> io::Result<()> {
    let listener = TcpListener::bind(listen_addr)?;
    listener.set_nonblocking(false)?; // Blocking accept is fine

    info!(logger, "Listener thread started"; "listen_addr" => format!("{}", listen_addr));

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let peer_addr = stream.peer_addr().ok();
                debug!(logger, "Accepted connection"; "peer_addr" => format!("{:?}", peer_addr));

                // Spawn a thread to handle this connection
                let msg_tx = msg_tx.clone();
                let logger = logger.clone();
                let peers = Arc::clone(&peers);

                thread::spawn(move || {
                    if let Err(e) =
                        handle_connection(node_id, &mut stream, msg_tx, peers, logger.clone())
                    {
                        debug!(logger, "Connection handler error"; "error" => format!("{}", e));
                    }
                });
            }
            Err(e) => {
                warn!(logger, "Failed to accept connection"; "error" => format!("{}", e));
            }
        }
    }

    Ok(())
}

/// Handle a single incoming connection, reading messages until the connection closes.
fn handle_connection(
    node_id: u64,
    stream: &mut TcpStream,
    msg_tx: Sender<Message>,
    _peers: Arc<HashMap<u64, SocketAddr>>,
    logger: Logger,
) -> io::Result<()> {
    loop {
        // Read message from stream
        match read_message(stream) {
            Ok(msg) => {
                debug!(logger, "Received message";
                       "from" => msg.from,
                       "to" => msg.to,
                       "msg_type" => format!("{:?}", msg.msg_type()));

                // Verify message is for us
                if msg.to != node_id {
                    warn!(logger, "Received message for wrong node";
                          "expected" => node_id,
                          "actual" => msg.to);
                    continue;
                }

                // Forward to ready loop
                if let Err(e) = msg_tx.send(msg) {
                    error!(logger, "Failed to forward message to ready loop"; "error" => format!("{}", e));
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "msg_tx channel closed",
                    ));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                // Connection closed gracefully
                debug!(logger, "Connection closed");
                return Ok(());
            }
            Err(e) => {
                warn!(logger, "Failed to read message"; "error" => format!("{}", e));
                return Err(e);
            }
        }
    }
}

/// Sender thread: dequeues messages and sends them to peers via TCP.
fn sender_thread(
    node_id: u64,
    sender_rx: Receiver<(u64, Message)>,
    peers: Arc<HashMap<u64, SocketAddr>>,
    logger: Logger,
) {
    info!(logger, "Sender thread started"; "node_id" => node_id);

    for (to, msg) in sender_rx {
        // Get peer address
        let peer_addr = match peers.get(&to) {
            Some(addr) => *addr,
            None => {
                warn!(logger, "Peer not found"; "peer_id" => to);
                continue;
            }
        };

        // Connect and send (with retries)
        match send_message_with_retry(&peer_addr, &msg, &logger) {
            Ok(_) => {
                debug!(logger, "Sent message";
                       "to" => to,
                       "peer_addr" => format!("{}", peer_addr),
                       "msg_type" => format!("{:?}", msg.msg_type()));
            }
            Err(e) => {
                // Log but continue - peer may be down
                warn!(logger, "Failed to send message";
                      "to" => to,
                      "peer_addr" => format!("{}", peer_addr),
                      "error" => format!("{}", e));
            }
        }
    }

    info!(logger, "Sender thread exiting"; "node_id" => node_id);
}

/// Send a message to a peer with retries.
fn send_message_with_retry(
    peer_addr: &SocketAddr,
    msg: &Message,
    logger: &Logger,
) -> io::Result<()> {
    let mut last_error = None;

    for attempt in 0..MAX_RETRIES {
        match TcpStream::connect_timeout(peer_addr, Duration::from_millis(100)) {
            Ok(mut stream) => {
                // Successfully connected, send the message
                return write_message(&mut stream, msg);
            }
            Err(e) => {
                debug!(logger, "Connection attempt failed";
                       "attempt" => attempt + 1,
                       "max_retries" => MAX_RETRIES,
                       "error" => format!("{}", e));
                last_error = Some(e);

                if attempt + 1 < MAX_RETRIES {
                    thread::sleep(RETRY_DELAY);
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, "Max retries exceeded")))
}

/// Read a length-prefixed message from a stream.
///
/// Message format:
/// - 4 bytes: message length (big-endian u32)
/// - N bytes: prost-encoded raft::Message
fn read_message<R: Read>(stream: &mut R) -> io::Result<Message> {
    // Read 4-byte length prefix
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes)?;
    let len = u32::from_be_bytes(len_bytes) as usize;

    // Sanity check: max message size 10MB
    if len > 10 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Message too large: {} bytes", len),
        ));
    }

    // Read message bytes
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;

    // Deserialize protobuf message
    // Create a default message and use merge to decode
    let mut msg = Message::default();

    // Use prost's merge directly (raft uses prost 0.11)
    // We need to import the trait from the correct prost version
    use prost_011::Message as ProstMessage011;
    msg.merge(&buf[..]).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to decode message: {}", e),
        )
    })?;

    Ok(msg)
}

/// Write a length-prefixed message to a stream.
fn write_message<W: Write>(stream: &mut W, msg: &Message) -> io::Result<()> {
    // Encode message to bytes using prost 0.11
    use prost_011::Message as ProstMessage011;

    let mut buf = Vec::new();
    msg.encode(&mut buf).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to encode message: {}", e),
        )
    })?;

    // Write length prefix
    let len = buf.len() as u32;
    stream.write_all(&len.to_be_bytes())?;

    // Write message bytes
    stream.write_all(&buf)?;

    // Flush to ensure message is sent
    stream.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raft::prelude::MessageType;
    use std::io::Cursor;

    #[test]
    fn test_message_serialization_roundtrip() {
        use prost_011::Message as ProstMessage011;

        let msg = Message {
            msg_type: MessageType::MsgHeartbeat.into(),
            to: 2,
            from: 1,
            term: 5,
            ..Default::default()
        };

        // Serialize using prost 0.11
        let mut msg_bytes = Vec::new();
        msg.encode(&mut msg_bytes).unwrap();

        // Write with length prefix
        let mut buf = Vec::new();
        let len = msg_bytes.len() as u32;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&msg_bytes);

        // Deserialize
        let mut cursor = Cursor::new(buf);
        let decoded = read_message(&mut cursor).unwrap();

        assert_eq!(msg.msg_type, decoded.msg_type);
        assert_eq!(msg.to, decoded.to);
        assert_eq!(msg.from, decoded.from);
        assert_eq!(msg.term, decoded.term);
    }

    #[test]
    fn test_message_length_prefix() {
        let msg = Message {
            msg_type: MessageType::MsgHeartbeat.into(),
            to: 2,
            from: 1,
            term: 5,
            ..Default::default()
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();

        // Check length prefix
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        assert_eq!(
            len,
            buf.len() - 4,
            "Length prefix should match message size"
        );
    }

    #[test]
    fn test_message_too_large() {
        // Create a buffer with a length prefix indicating 11MB
        let mut buf = Vec::new();
        let large_len = (11 * 1024 * 1024u32).to_be_bytes();
        buf.extend_from_slice(&large_len);

        let mut cursor = Cursor::new(buf);
        let result = read_message(&mut cursor);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Message too large"));
    }
}
