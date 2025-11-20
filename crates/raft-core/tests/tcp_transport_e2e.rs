use std::collections::HashMap;
use std::net::SocketAddr;
use std::thread;
use std::time::Duration;

use crossbeam_channel::unbounded;
use raft::prelude::{Message, MessageType};
use raft_core::network::Transport;
use raft_core::tcp_transport::TcpTransport;
use slog::{o, Drain, Logger};

fn setup_logger() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    Logger::root(drain, o!())
}

fn get_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

#[test]
fn test_tcp_transport_e2e() {
    let logger = setup_logger();

    let port1 = get_free_port();
    let port2 = get_free_port();

    // Ensure ports are different
    assert_ne!(port1, port2);

    let addr1: SocketAddr = format!("127.0.0.1:{}", port1).parse().unwrap();
    let addr2: SocketAddr = format!("127.0.0.1:{}", port2).parse().unwrap();

    let mut peers = HashMap::new();
    peers.insert(1, addr1);
    peers.insert(2, addr2);

    let (tx1, _rx1) = unbounded();
    let (tx2, rx2) = unbounded();

    // Start Transport 1
    let t1 = TcpTransport::new(1, addr1, peers.clone(), tx1, logger.clone())
        .expect("Failed to create transport 1");

    // Start Transport 2
    let _t2 = TcpTransport::new(2, addr2, peers.clone(), tx2, logger.clone())
        .expect("Failed to create transport 2");

    // Allow time for listeners to bind
    thread::sleep(Duration::from_millis(100));

    // Test 1: Single Message
    let msg = Message {
        msg_type: MessageType::MsgHeartbeat.into(),
        to: 2,
        from: 1,
        term: 10,
        ..Default::default()
    };

    t1.send(2, msg).expect("Failed to send message");

    let received = rx2
        .recv_timeout(Duration::from_secs(2))
        .expect("Failed to receive message on node 2");

    assert_eq!(received.term, 10);
    assert_eq!(received.from, 1);
    assert_eq!(received.to, 2);

    // Test 2: Multiple messages (Persistent connection check)
    // If the connection wasn't persistent or had issues, rapid firing might fail or be slow
    for i in 0..5 {
        let msg = Message {
            msg_type: MessageType::MsgHeartbeat.into(),
            to: 2,
            from: 1,
            term: 20 + i as u64,
            ..Default::default()
        };
        t1.send(2, msg).expect("Failed to send sequence message");
    }

    for i in 0..5 {
        let received = rx2
            .recv_timeout(Duration::from_secs(1))
            .expect("Failed to receive sequence message");
        assert_eq!(received.term, 20 + i as u64);
    }
}
