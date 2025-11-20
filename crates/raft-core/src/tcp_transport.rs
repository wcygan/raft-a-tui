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
const RETRY_DELAY: Duration = Duration::from_millis(500);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// TCP-based transport for sending Raft messages between nodes.
///
/// This transport uses persistent TCP connections to send protobuf-encoded Raft messages.
/// Messages are framed with a 4-byte length prefix (big-endian u32).
///
/// # Architecture
/// - **Main thread**: Creates TcpTransport, dispatches messages to per-peer sender threads.
/// - **Listener thread**: Accepts incoming connections, deserializes messages, forwards to msg_rx.
/// - **Peer Sender threads**: One thread per peer. Maintains a persistent connection, handles reconnection, and sends messages.
pub struct TcpTransport {
    node_id: u64,
    peer_senders: HashMap<u64, Sender<Message>>,
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
    pub fn new(
        node_id: u64,
        listen_addr: SocketAddr,
        peers: HashMap<u64, SocketAddr>,
        msg_tx: Sender<Message>,
        logger: Logger,
    ) -> io::Result<Self> {
        let mut peer_senders = HashMap::new();
        let peers_arc = Arc::new(peers.clone());

        // Start listener thread
        let listener_logger = logger.clone();
        let listener_peers = peers_arc.clone();
        let listener_msg_tx = msg_tx.clone();
        thread::Builder::new()
            .name(format!("tcp-listener-{}", node_id))
            .spawn(move || {
                if let Err(e) = listener_thread(
                    node_id,
                    listen_addr,
                    listener_msg_tx,
                    listener_peers,
                    listener_logger.clone(),
                ) {
                    error!(listener_logger, "Listener thread failed"; "error" => format!("{}", e));
                }
            })?;

        // Start per-peer sender threads
        for (&peer_id, &peer_addr) in &peers {
            if peer_id == node_id {
                continue; // Don't spawn sender for self
            }

            let (tx, rx) = unbounded();
            peer_senders.insert(peer_id, tx);

            let sender_logger = logger.clone();
            thread::Builder::new()
                .name(format!("tcp-sender-{}-{}", node_id, peer_id))
                .spawn(move || {
                    peer_sender_thread(node_id, peer_id, peer_addr, rx, sender_logger);
                })?;
        }

        info!(logger, "TCP transport started";
              "node_id" => node_id,
              "listen_addr" => format!("{}", listen_addr),
              "peer_count" => peers.len());

        Ok(Self {
            node_id,
            peer_senders,
            logger,
        })
    }

    /// Get this node's ID.
    pub fn node_id(&self) -> u64 {
        self.node_id
    }
}

impl Transport for TcpTransport {
    fn send(&self, to: u64, mut msg: Message) -> Result<(), TransportError> {
        // raft-rs generates messages with from=0, transport layer must fill in sender ID
        msg.from = self.node_id;

        // Get the sender channel for the peer
        if let Some(sender) = self.peer_senders.get(&to) {
            // Queue message for sender thread
            // If the channel is full or disconnected, that's a critical error
            sender.send(msg).map_err(|_| {
                error!(self.logger, "Failed to queue message for sending"; "to" => to);
                TransportError::ChannelClosed(to)
            })
        } else {
            warn!(self.logger, "Attempted to send to unknown peer"; "to" => to);
            Err(TransportError::PeerNotFound(to))
        }
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
                let peers = peers.clone();

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
    // Set read timeout to detect dead connections eventually?
    // For now, let's leave it blocking. Raft heartbeats should keep it alive or we'll read 0 bytes.

    loop {
        // Read message from stream
        match read_message(stream) {
            Ok(msg) => {
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
                    return Err(io::Error::other("msg_tx channel closed"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                // Connection closed gracefully
                debug!(logger, "Connection closed by peer");
                return Ok(());
            }
            Err(e) => {
                // Any other error is fatal for this connection
                warn!(logger, "Failed to read message"; "error" => format!("{}", e));
                return Err(e);
            }
        }
    }
}

/// Peer Sender Thread: Maintains a persistent connection to a specific peer.
fn peer_sender_thread(
    _node_id: u64,
    peer_id: u64,
    peer_addr: SocketAddr,
    rx: Receiver<Message>,
    logger: Logger,
) {
    info!(logger, "Peer sender thread started"; "peer_id" => peer_id, "peer_addr" => format!("{}", peer_addr));

    let mut stream: Option<TcpStream> = None;

    // Loop until the channel is closed (Transport is dropped)
    for msg in rx {
        let mut sent = false;
        let mut attempt = 0;

        while !sent && attempt < MAX_RETRIES {
            attempt += 1;

            // 1. Ensure connection exists
            if stream.is_none() {
                debug!(logger, "Connecting to peer"; "peer_id" => peer_id, "attempt" => attempt);
                match TcpStream::connect_timeout(&peer_addr, CONNECT_TIMEOUT) {
                    Ok(s) => {
                        if let Err(e) = s.set_nodelay(true) {
                            warn!(logger, "Failed to set nodelay"; "error" => format!("{}", e));
                        }
                        stream = Some(s);
                        debug!(logger, "Connected to peer"; "peer_id" => peer_id);
                    }
                    Err(e) => {
                        warn!(logger, "Failed to connect"; "peer_id" => peer_id, "error" => format!("{}", e));
                        thread::sleep(RETRY_DELAY);
                        continue; // Retry loop
                    }
                }
            }

            // 2. Write message
            if let Some(mut s) = stream.take() {
                match write_message(&mut s, &msg) {
                    Ok(_) => {
                        sent = true;
                        stream = Some(s); // Put it back
                    }
                    Err(e) => {
                        warn!(logger, "Failed to send message";
                              "peer_id" => peer_id,
                              "msg_type" => format!("{:?}", msg.msg_type()),
                              "error" => format!("{}", e));

                        // Stream is dead (taken), don't put it back.
                        // Wait before retry
                        thread::sleep(RETRY_DELAY);
                    }
                }
            }
        }

        if !sent {
            error!(logger, "Dropped message after max retries";
                    "peer_id" => peer_id,
                    "msg_type" => format!("{:?}", msg.msg_type()));
        }
    }

    info!(logger, "Peer sender thread exiting"; "peer_id" => peer_id);
}

/// Read a length-prefixed message from a stream.
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
    let mut msg = Message::default();

    // Use prost's merge directly (raft uses prost 0.11)
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
        let msg = Message {
            msg_type: MessageType::MsgHeartbeat.into(),
            to: 2,
            from: 1,
            term: 5,
            ..Default::default()
        };

        // Encode
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();

        // Decode
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

        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        assert_eq!(len, buf.len() - 4);
    }
}
