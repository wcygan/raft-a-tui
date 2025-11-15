---
name: raft-implementation-guide
description: Expert guidance for implementing Raft consensus algorithm using raft-rs (TiKV). Covers RawNode integration, Ready loop patterns, storage/network layers, debugging elections and log replication, and production deployment. Use when implementing Raft, debugging consensus issues, or designing distributed systems. Keywords: raft, consensus, distributed systems, raft-rs, TiKV, RawNode, Ready loop, election, log replication, quorum, leader election, state machine
---

# Raft Implementation Guide

Comprehensive guidance for implementing the Raft consensus algorithm using TiKV's raft-rs library, based on the Raft paper, official documentation, and production experience.

## When to Activate

Use this skill when:
- Implementing Raft consensus from scratch
- Integrating raft-rs into a new or existing system
- Debugging consensus issues (split-brain, election storms, log divergence)
- Optimizing Raft performance or designing multi-raft systems
- Understanding Raft concepts and translating them to raft-rs APIs

## Core Raft Concepts

### The Three Roles

**Leader** - Handles all client requests, replicates log entries to followers
- Sends periodic heartbeats to maintain authority
- Appends entries to follower logs
- Advances commit index when majority acknowledges

**Follower** - Passive, responds to leader and candidate requests
- Resets election timer on valid leader heartbeat
- Votes for at most one candidate per term
- Becomes candidate if election timeout elapses

**Candidate** - Transitional state during elections
- Increments term, votes for self
- Requests votes from peers
- Becomes leader if majority votes received
- Reverts to follower if higher term discovered or valid leader emerges

### Safety Properties

1. **Election Safety**: At most one leader per term
2. **Leader Append-Only**: Leaders never overwrite/delete log entries
3. **Log Matching**: If two logs contain entry with same index/term, all preceding entries are identical
4. **Leader Completeness**: If entry committed in term T, it will be present in leaders of all future terms
5. **State Machine Safety**: If server applies log entry at index i, no other server will apply different entry at i

## raft-rs Integration Pattern

### Phase 1: Storage Layer

Implement or wrap a `Storage` trait implementation:

```rust
use raft::{Storage, StorageError, RaftState};

struct RaftStorage {
    mem_storage: MemStorage,
    applied_index: u64,  // CRITICAL: Track separately from commit_index
}

impl RaftStorage {
    fn new() -> Self {
        Self {
            mem_storage: MemStorage::new(),
            applied_index: 0,
        }
    }

    fn apply_entry(&mut self, index: u64, data: &[u8]) -> Result<()> {
        // Apply to state machine ONLY if not already applied
        if index <= self.applied_index {
            return Ok(()); // Idempotent application
        }

        // Apply state machine transition
        // ...

        self.applied_index = index;
        Ok(())
    }
}

impl Storage for RaftStorage {
    // Delegate to MemStorage with applied_index tracking
    fn initial_state(&self) -> Result<RaftState> {
        self.mem_storage.initial_state()
    }
    // ... other methods
}
```

**Key Points:**
- `applied_index` prevents reapplication after restarts
- Must persist applied_index in snapshots
- Use dummy log entry to preserve last truncated term

### Phase 2: Network Transport

Create message passing infrastructure:

```rust
use crossbeam_channel::{Sender, Receiver};
use raft::prelude::Message;

struct LocalTransport {
    // Map peer_id -> sender channel
    peers: HashMap<u64, Sender<Message>>,
}

impl LocalTransport {
    fn send(&self, msg: Message) -> Result<()> {
        let peer_id = msg.to;
        if let Some(sender) = self.peers.get(&peer_id) {
            sender.send(msg)?;
        }
        Ok(())
    }
}
```

**Production considerations:**
- TCP/UDP for multi-machine deployments
- Batch messages respecting `max_size_per_msg`
- Pipeline within `max_inflight_msgs` limit
- Handle network partitions gracefully

### Phase 3: RawNode Setup

Initialize Raft node with proper configuration:

```rust
use raft::{Config, RawNode};

fn create_raft_node(node_id: u64, peers: Vec<u64>) -> Result<RawNode<RaftStorage>> {
    let mut config = Config {
        id: node_id,
        election_tick: 10,      // 1 second at 100ms tick
        heartbeat_tick: 3,      // 300ms heartbeats
        max_size_per_msg: 1024 * 1024,  // 1MB
        max_inflight_msgs: 256,
        check_quorum: true,     // Leader verifies quorum
        pre_vote: true,         // Prevent disruptive elections
        ..Default::default()
    };

    config.validate()?;

    let storage = RaftStorage::new();
    let mut raw_node = RawNode::new(&config, storage, &logger)?;

    // Bootstrap cluster (only on initial creation)
    if is_first_start {
        let peer_ids = peers.iter().map(|&id| id).collect();
        raw_node.bootstrap(peer_ids)?;
    }

    Ok(raw_node)
}
```

**Configuration Guidelines:**
- `election_tick > heartbeat_tick` (typically 3-5x)
- Longer election timeout = fewer elections, slower failover
- `check_quorum = true` prevents stale leaders during partitions
- `pre_vote = true` reduces election disruptions

### Phase 4: The Ready Loop

**CRITICAL PATTERN** - The heart of Raft integration:

```rust
fn raft_ready_loop(
    mut raw_node: RawNode<RaftStorage>,
    msg_rx: Receiver<RaftMessage>,
    transport: LocalTransport,
) -> Result<()> {
    let mut tick_timer = Instant::now();

    loop {
        // === PHASE 1: INPUT RECEPTION ===
        match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(RaftMessage::Peer(msg)) => {
                raw_node.step(msg)?;
            }
            Ok(RaftMessage::Proposal { data, callback }) => {
                raw_node.propose(vec![], data)?;
            }
            Err(RecvTimeoutError::Timeout) => {
                // Tick every 100ms
                if tick_timer.elapsed() >= Duration::from_millis(100) {
                    raw_node.tick();
                    tick_timer = Instant::now();
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // === PHASE 2: READY CHECK ===
        if !raw_node.has_ready() {
            continue;
        }

        let mut ready = raw_node.ready();

        // === PHASE 3: READY PROCESSING ===
        // ORDER MATTERS - DO NOT REORDER THESE STEPS!

        // 1. Apply snapshot (if present)
        if !ready.snapshot().is_empty() {
            let snapshot = ready.snapshot();
            raw_node.mut_store().apply_snapshot(snapshot)?;
        }

        // 2. Append new log entries to storage
        if !ready.entries().is_empty() {
            let entries = ready.entries();
            raw_node.mut_store().append(entries)?;
        }

        // 3. Persist HardState (term, vote, commit)
        if let Some(hs) = ready.hs() {
            raw_node.mut_store().set_hardstate(hs.clone())?;
        }

        // 4. Send messages to peers (AFTER persistence!)
        for msg in ready.take_messages() {
            transport.send(msg)?;
        }

        // 5. Apply committed entries to state machine
        for entry in ready.take_committed_entries() {
            if entry.data.is_empty() {
                // Empty entry from leader election - safe to skip
                continue;
            }

            match entry.get_entry_type() {
                EntryType::EntryNormal => {
                    // Apply to state machine
                    raw_node.mut_store().apply_entry(entry.index, &entry.data)?;
                }
                EntryType::EntryConfChange | EntryType::EntryConfChangeV2 => {
                    // Handle cluster membership changes
                    let cc = ConfChange::decode(&entry.data)?;
                    raw_node.apply_conf_change(&cc)?;
                }
            }
        }

        // 6. Advance to next Ready
        let mut light_ready = raw_node.advance(ready);

        // Process light_ready (additional messages/commits)
        for msg in light_ready.take_messages() {
            transport.send(msg)?;
        }

        for entry in light_ready.take_committed_entries() {
            // Apply similarly to main ready
            if !entry.data.is_empty() {
                raw_node.mut_store().apply_entry(entry.index, &entry.data)?;
            }
        }
    }

    Ok(())
}
```

**Why This Order?**
1. Snapshot before entries (snapshot may truncate log)
2. Append before HardState (log must exist before commit advances)
3. Persist before sending (safety guarantee)
4. Send before apply (asynchronous replication)
5. Apply after all persistence (state machine consistency)

## Common Pitfalls & Solutions

### Pitfall 1: Applying Entries Twice After Restart

**Problem:** `applied_index` not persisted, entries reapplied on restart

**Solution:**
```rust
// Include applied_index in state machine snapshots
struct Snapshot {
    data: BTreeMap<String, String>,
    applied_index: u64,  // Must persist this!
    conf_state: ConfState,
}
```

### Pitfall 2: Split-Brain (Two Leaders)

**Cause:** Network partition allows both sides to elect leaders

**Detection:**
```rust
// Enable quorum checking in config
config.check_quorum = true;

// Leaders without quorum step down automatically
```

**Prevention:**
- Always use odd-numbered clusters (3, 5, 7)
- Ensure majority-based quorums
- Use pre-vote to prevent disruptive elections

### Pitfall 3: Election Storms

**Symptoms:** Frequent elections, no stable leader

**Causes:**
- Election timeout too short
- Network latency exceeds heartbeat interval
- Asymmetric network partitions

**Solutions:**
```rust
// Increase election timeout
config.election_tick = 20;  // 2 seconds at 100ms tick

// Increase heartbeat frequency
config.heartbeat_tick = 3;  // Still 300ms

// Enable pre-vote
config.pre_vote = true;
```

### Pitfall 4: Log Divergence

**Cause:** Improper handling of conflicting entries

**Detection:**
```rust
// Raft automatically detects via log matching property
// Check logs for consistency:
fn verify_log_consistency(storage: &dyn Storage) -> Result<()> {
    let last_index = storage.last_index()?;
    for i in storage.first_index()?..last_index {
        let term_i = storage.term(i)?;
        let term_i_plus_1 = storage.term(i + 1)?;
        // Terms must be monotonically non-decreasing
        assert!(term_i <= term_i_plus_1);
    }
    Ok(())
}
```

**Automatic Handling:** raft-rs handles log conflicts automatically via AppendEntries RPC

### Pitfall 5: Not Handling Empty Committed Entries

**Problem:** Panic or incorrect state on empty entries from leader elections

**Solution:**
```rust
for entry in ready.take_committed_entries() {
    if entry.data.is_empty() {
        continue;  // Skip empty entries safely
    }
    // Apply to state machine
}
```

## Debugging Techniques

### Enable Detailed Logging

```rust
use slog::{Drain, Logger, o};

let decorator = slog_term::PlainDecorator::new(std::io::stdout());
let drain = slog_term::CompactFormat::new(decorator).build().fuse();
let drain = slog_async::Async::new(drain).build().fuse();
let logger = Logger::root(drain, o!("version" => "1.0"));
```

### Track Raft State Transitions

```rust
fn log_raft_state(raw_node: &RawNode<RaftStorage>, logger: &Logger) {
    let raft = raw_node.raft;
    info!(logger, "Raft state";
        "term" => raft.term,
        "role" => format!("{:?}", raft.state),
        "leader" => raft.leader_id,
        "commit" => raft.raft_log.committed,
        "applied" => raft.raft_log.applied,
        "last_index" => raft.raft_log.last_index(),
    );
}
```

### Visualize Message Flow

```rust
fn log_message(msg: &Message, direction: &str, logger: &Logger) {
    debug!(logger, "Message {}", direction;
        "type" => format!("{:?}", msg.msg_type),
        "from" => msg.from,
        "to" => msg.to,
        "term" => msg.term,
        "index" => msg.index,
        "log_term" => msg.log_term,
        "entries" => msg.entries.len(),
    );
}

// In Ready loop:
for msg in ready.take_messages() {
    log_message(&msg, "SEND", &logger);
    transport.send(msg)?;
}
```

### Analyze Election Patterns

```rust
// Track election metrics
struct ElectionMetrics {
    elections_started: AtomicU64,
    elections_won: AtomicU64,
    elections_lost: AtomicU64,
    average_election_duration: AtomicU64,
}

// Detect election storms
if elections_started.load(Ordering::Relaxed) > 10 {
    warn!(logger, "Potential election storm detected");
    // Increase election timeout dynamically
}
```

## Performance Optimization

### Batch Proposals

```rust
// Instead of proposing one entry at a time:
for cmd in commands {
    raw_node.propose(vec![], cmd)?;  // ❌ Inefficient
}

// Batch multiple commands:
let batch = encode_batch_command(commands);
raw_node.propose(vec![], batch)?;  // ✅ Better throughput
```

### Async I/O for Persistence

```rust
// Use async storage operations
raw_node.advance_append_async(ready);

// Later, after persistence completes:
raw_node.on_persist_ready(persist_index);
```

### Pipeline Proposals

```rust
// Configure higher inflight limit
config.max_inflight_msgs = 512;  // More concurrent proposals

// Monitor pipeline depth
let inflight = raw_node.raft.prs().voter_ids().iter()
    .map(|id| raw_node.raft.prs().get(*id).unwrap().ins.in_flight())
    .max()
    .unwrap_or(0);
```

### Snapshot Compression

```rust
use flate2::write::GzEncoder;

fn create_compressed_snapshot(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}
```

## Production Deployment Patterns

### Multi-Raft (Sharding)

For large-scale systems, run multiple Raft groups per node:

```rust
struct MultiRaft {
    // Map region_id -> RawNode
    regions: HashMap<u64, RawNode<RaftStorage>>,

    // Shared transport for all regions
    transport: Arc<Transport>,
}

impl MultiRaft {
    fn route_message(&mut self, msg: Message) -> Result<()> {
        // Extract region_id from message context
        let region_id = extract_region_id(&msg);

        if let Some(raw_node) = self.regions.get_mut(&region_id) {
            raw_node.step(msg)?;
        }

        Ok(())
    }
}
```

### Read Optimization (Lease-Based Reads)

```rust
// Configure read-only option
config.read_only_option = ReadOnlyOption::LeaseBased;

// Leader can serve reads without quorum check
if raw_node.raft.state == StateRole::Leader {
    // Serve read immediately (assumes lease validity)
    return state_machine.get(key);
}
```

### Graceful Shutdown

```rust
fn shutdown_raft_node(raw_node: &mut RawNode<RaftStorage>) -> Result<()> {
    // If leader, transfer leadership
    if raw_node.raft.state == StateRole::Leader {
        let peers = raw_node.raft.prs().voter_ids();
        if let Some(&target) = peers.iter().next() {
            raw_node.transfer_leader(target);
        }
    }

    // Wait for leadership transfer
    thread::sleep(Duration::from_secs(1));

    // Persist final state
    if raw_node.has_ready() {
        let ready = raw_node.ready();
        // Process ready one last time
        // ...
        raw_node.advance(ready);
    }

    Ok(())
}
```

## Testing Strategies

### Unit Tests: State Machine Logic

```rust
#[test]
fn test_state_machine_application() {
    let mut storage = RaftStorage::new();

    // Apply entries in sequence
    storage.apply_entry(1, &encode_put("key1", "value1"))?;
    storage.apply_entry(2, &encode_put("key2", "value2"))?;

    assert_eq!(storage.get("key1"), Some("value1"));
    assert_eq!(storage.get("key2"), Some("value2"));
    assert_eq!(storage.applied_index, 2);
}

#[test]
fn test_idempotent_application() {
    let mut storage = RaftStorage::new();

    storage.apply_entry(1, &encode_put("key", "value"))?;
    storage.apply_entry(1, &encode_put("key", "different"))?;  // Same index

    // Should not reapply
    assert_eq!(storage.get("key"), Some("value"));
}
```

### Integration Tests: 3-Node Cluster

```rust
#[test]
fn test_leader_election() {
    let (nodes, transports) = setup_cluster(3);

    // Start nodes
    let handles: Vec<_> = nodes.into_iter()
        .zip(transports)
        .map(|(node, transport)| {
            thread::spawn(move || raft_ready_loop(node, transport))
        })
        .collect();

    // Wait for leader election
    thread::sleep(Duration::from_secs(2));

    // Verify exactly one leader
    let leader_count = count_leaders(&handles);
    assert_eq!(leader_count, 1);
}

#[test]
fn test_log_replication() {
    let cluster = setup_cluster(3);
    let leader = find_leader(&cluster);

    // Propose entry on leader
    leader.propose(vec![], encode_put("x", "100"))?;

    // Wait for replication
    thread::sleep(Duration::from_millis(500));

    // Verify all nodes applied entry
    for node in &cluster.nodes {
        assert_eq!(node.state_machine.get("x"), Some("100"));
    }
}
```

### Chaos Tests: Network Partitions

```rust
#[test]
fn test_partition_recovery() {
    let mut cluster = setup_cluster(5);

    // Partition: [1,2] | [3,4,5]
    cluster.partition(&[1, 2], &[3, 4, 5]);

    // Majority side should elect leader
    thread::sleep(Duration::from_secs(2));
    let majority_leaders = count_leaders(&cluster.nodes[2..5]);
    assert_eq!(majority_leaders, 1);

    // Minority side should have no leader
    let minority_leaders = count_leaders(&cluster.nodes[0..2]);
    assert_eq!(minority_leaders, 0);

    // Heal partition
    cluster.heal_partition();

    // All nodes should converge to one leader
    thread::sleep(Duration::from_secs(2));
    assert_eq!(count_leaders(&cluster.nodes), 1);
}
```

## References

- **Raft Paper**: https://raft.github.io/raft.pdf (authoritative source)
- **raft-rs Documentation**: https://docs.rs/raft/latest/raft/
- **TiKV Blog**: https://tikv.org/blog/implement-raft-in-rust/
- **Example Implementation**: raft-rs/examples/five_mem_node

For detailed Raft concept explanations, see REFERENCE.md in this skill directory.

## Output Guidelines

When helping with Raft implementation:

1. **Reference specific phases**: "You're currently in Phase 2 (Network Layer)"
2. **Cite the paper**: "Per section 5.3 of the Raft paper, log matching property requires..."
3. **Show code patterns**: Provide concrete raft-rs examples
4. **Identify pitfalls**: "Watch out for Pitfall 3 (election storms)"
5. **Suggest tests**: "Add a chaos test for network partition scenario"
6. **Verify order**: "Ready loop Phase 3 requires persistence before sending messages"

Always prioritize correctness over performance, and safety over optimization.
