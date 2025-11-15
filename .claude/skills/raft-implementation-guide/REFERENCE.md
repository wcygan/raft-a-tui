# Raft Consensus Algorithm - Deep Reference

Detailed explanations of Raft concepts, algorithm details, and raft-rs specific implementation notes.

## Table of Contents

1. [Core Algorithm Details](#core-algorithm-details)
2. [Safety Proof Sketch](#safety-proof-sketch)
3. [raft-rs API Deep Dive](#raft-rs-api-deep-dive)
4. [Edge Cases and Corner Scenarios](#edge-cases-and-corner-scenarios)
5. [Performance Characteristics](#performance-characteristics)

---

## Core Algorithm Details

### Leader Election (Section 5.2)

**Election Timeout Randomization:**
- Each follower chooses random timeout in [T, 2T] (typically T=150ms)
- Prevents simultaneous candidates (split votes)
- First timeout expires → becomes candidate

**Election Process:**
1. Increment current term
2. Vote for self
3. Reset election timer
4. Send RequestVote RPCs to all peers
5. Possible outcomes:
   - **Win**: Receive votes from majority → become leader
   - **Lose**: Receive AppendEntries from valid leader → revert to follower
   - **Split**: Timeout expires without majority → start new election

**RequestVote RPC:**
```
Arguments:
  term          - candidate's term
  candidateId   - candidate requesting vote
  lastLogIndex  - index of candidate's last log entry
  lastLogTerm   - term of candidate's last log entry

Results:
  term          - currentTerm, for candidate to update itself
  voteGranted   - true means candidate received vote

Receiver implementation:
  1. Reply false if term < currentTerm
  2. If votedFor is null or candidateId, and candidate's log is at
     least as up-to-date as receiver's log, grant vote (§5.4)
```

**Log Up-to-Date Check (Section 5.4.1):**
```rust
fn is_log_up_to_date(candidate_last_term: u64, candidate_last_index: u64,
                      my_last_term: u64, my_last_index: u64) -> bool {
    // First compare terms
    if candidate_last_term != my_last_term {
        return candidate_last_term > my_last_term;
    }
    // If terms equal, compare indices
    candidate_last_index >= my_last_index
}
```

**Why This Works:**
- Ensures elected leader has all committed entries
- Prevents candidate with incomplete log from becoming leader
- Critical for Leader Completeness property

### Log Replication (Section 5.3)

**AppendEntries RPC (Heartbeat and Log Replication):**
```
Arguments:
  term              - leader's term
  leaderId          - so follower can redirect clients
  prevLogIndex      - index of log entry immediately preceding new ones
  prevLogTerm       - term of prevLogIndex entry
  entries[]         - log entries to store (empty for heartbeat)
  leaderCommit      - leader's commitIndex

Results:
  term              - currentTerm, for leader to update itself
  success           - true if follower contained entry matching prevLogIndex and prevLogTerm

Receiver implementation:
  1. Reply false if term < currentTerm (§5.1)
  2. Reply false if log doesn't contain an entry at prevLogIndex
     whose term matches prevLogTerm (§5.3)
  3. If an existing entry conflicts with a new one (same index
     but different terms), delete the existing entry and all that
     follow it (§5.3)
  4. Append any new entries not already in the log
  5. If leaderCommit > commitIndex, set commitIndex =
     min(leaderCommit, index of last new entry)
```

**Consistency Check:**
- Leader maintains `nextIndex` for each follower (next entry to send)
- Leader maintains `matchIndex` for each follower (highest known replicated entry)
- On AppendEntries failure: decrement `nextIndex`, retry
- On success: update `nextIndex` and `matchIndex`

**Commitment Rule:**
- Leader can commit entry from current term once replicated on majority
- Cannot commit entries from previous terms directly (Figure 8 scenario)
- Must commit at least one entry from current term first

### Safety Properties (Section 5.4)

**Election Restriction (5.4.1):**
- Prevents candidate from winning election without all committed entries
- Implemented via log up-to-date check in RequestVote

**Commitment Property (5.4.2):**
- Leader knows entry is committed when replicated on majority
- Leader must also have entry from current term committed
- Ensures safe to apply entry to state machine

**Leader Completeness:**
- If entry committed at index i in term T, all future leaders will have it
- Proof by induction on term numbers
- Relies on election restriction and log matching

**State Machine Safety:**
- If server applies entry at index i, no other server applies different entry at i
- Follows from Leader Completeness and log matching

---

## Safety Proof Sketch

### Theorem: State Machine Safety

**Claim:** If a server has applied a log entry at index i, no other server will ever apply a different log entry at index i.

**Proof Outline:**

1. **Assume contradiction**: Servers A and B apply different entries at index i
   - Let A apply entry E1 at index i in term T1
   - Let B apply entry E2 at index i in term T2 (E1 ≠ E2)

2. **Case 1: T1 = T2**
   - Both entries in same term
   - Leader can only commit one entry per index per term
   - Contradiction! ⚡

3. **Case 2: T1 < T2 (without loss of generality)**
   - A committed E1 in earlier term T1
   - B committed E2 in later term T2

4. **Apply Leader Completeness:**
   - If E1 committed in T1, leader of T2 must have E1 at index i
   - Leader of T2 cannot have E2 at index i (different entry)
   - Contradiction! ⚡

5. **Conclusion:** State Machine Safety holds

### Leader Completeness Proof Sketch

**Claim:** If log entry committed in term T, then present in logs of all leaders for terms > T.

**Proof by Induction on Terms:**

**Base case:** Leader of term T+1
- For entry to commit in T, replicated on majority M_T
- For candidate to win election in T+1, must receive votes from majority M_{T+1}
- M_T ∩ M_{T+1} ≠ ∅ (majorities always overlap)
- At least one voter has committed entry
- Election restriction ensures candidate has log at least as up-to-date
- Therefore leader of T+1 has committed entry

**Inductive step:** Assume true for all terms ≤ K, prove for K+1
- Leader of K has committed entry (inductive hypothesis)
- Same majority overlap argument applies
- Leader of K+1 must have entry

---

## raft-rs API Deep Dive

### RawNode Lifecycle

```rust
// 1. Creation
let mut config = Config {
    id: node_id,
    election_tick: 10,
    heartbeat_tick: 3,
    ..Default::default()
};
config.validate()?;

let storage = MemStorage::new();
let raw_node = RawNode::new(&config, storage, &logger)?;

// 2. Bootstrap (first start only)
raw_node.bootstrap(vec![peer_id1, peer_id2, peer_id3])?;

// 3. Operation
loop {
    // Drive state machine
    raw_node.tick();
    raw_node.step(msg)?;
    raw_node.propose(context, data)?;

    // Process state changes
    if raw_node.has_ready() {
        let ready = raw_node.ready();
        // ... handle ready
        raw_node.advance(ready);
    }
}
```

### Config Parameters

**Timing:**
- `election_tick` - Election timeout in ticks (typically 10-50)
- `heartbeat_tick` - Heartbeat interval in ticks (typically 1-10)
- **Constraint:** `election_tick > heartbeat_tick` (usually 3-5x)

**Message Limits:**
- `max_size_per_msg` - Max bytes per message (default 1MB)
- `max_inflight_msgs` - Max concurrent messages to follower (default 256)
- `max_uncommitted_size` - Max uncommitted log bytes (default 1GB)

**Safety Features:**
- `check_quorum` - Leader verifies connectivity to majority
- `pre_vote` - Prevents disruptive elections from partitioned nodes
- `skip_bcast_commit` - Skip broadcasting commit index (optimization)

**Read Optimization:**
- `read_only_option`
  - `ReadOnlySafe` - Linearizable reads (quorum check)
  - `ReadOnlyLeaseBased` - Lease-based reads (faster, may read stale during partition)

### Storage Trait Requirements

**Must Implement:**

```rust
pub trait Storage {
    /// Returns the current hard state and conf state.
    fn initial_state(&self) -> Result<RaftState>;

    /// Returns a slice of log entries in [low, high).
    fn entries(&self, low: u64, high: u64, max_size: impl Into<Option<u64>>,
               context: GetEntriesContext) -> Result<Vec<Entry>>;

    /// Returns the term of entry idx.
    fn term(&self, idx: u64) -> Result<u64>;

    /// Returns the index of the first log entry.
    fn first_index(&self) -> Result<u64>;

    /// Returns the index of the last log entry.
    fn last_index(&self) -> Result<u64>;

    /// Returns the most recent snapshot.
    fn snapshot(&self, request_index: u64, to: u64) -> Result<Snapshot>;
}
```

**Critical Guarantees:**

1. **Persistence before visibility:**
   - `entries()` must return persisted entries only
   - Don't expose in-memory entries not yet on disk

2. **Atomic updates:**
   - HardState updates must be atomic
   - Can't have partial term/vote/commit state

3. **Snapshot consistency:**
   - Snapshot must include all entries up to `applied_index`
   - Must include ConfState at time of snapshot

4. **First index handling:**
   - After snapshot, `first_index()` returns snapshot_index + 1
   - Use dummy entry to preserve last truncated term

### Ready State Processing

**Ready Fields:**

```rust
pub struct Ready {
    /// Entries to append to stable storage
    pub entries: Vec<Entry>,

    /// Current HardState (term, vote, commit)
    pub hs: Option<HardState>,

    /// Messages to send to peers
    pub messages: Vec<Message>,

    /// Entries committed and ready to apply
    pub committed_entries: Vec<Entry>,

    /// Snapshot to apply (if present)
    pub snapshot: Snapshot,

    /// Must persist before sending persisted_messages
    pub must_sync: bool,

    // ... other fields
}
```

**Processing Order (CRITICAL):**

```rust
// 1. Snapshot (may truncate log)
if !ready.snapshot().is_empty() {
    storage.apply_snapshot(ready.snapshot())?;
}

// 2. Append entries (log must have entries before commit advances)
if !ready.entries().is_empty() {
    storage.append(ready.entries())?;
}

// 3. HardState (persist term/vote/commit)
if let Some(hs) = ready.hs() {
    storage.set_hardstate(hs)?;
}

// 4. Send messages (AFTER persistence for safety)
for msg in ready.messages {
    transport.send(msg)?;
}

// 5. Apply committed entries (state machine transition)
for entry in ready.committed_entries {
    state_machine.apply(entry)?;
}

// 6. Advance
raw_node.advance(ready);
```

**Why must_sync Matters:**
- If `must_sync = true`, MUST fsync before sending `persisted_messages`
- Safety requirement: don't send votes/confirmations before durable
- Can async write regular messages while syncing

### Message Types

**Vote Messages:**
- `MsgRequestVote` - Request vote in election
- `MsgRequestVoteResponse` - Grant or deny vote

**Replication Messages:**
- `MsgAppend` - Leader → Follower log entries
- `MsgAppendResponse` - Follower → Leader acknowledgment
- `MsgSnapshot` - Leader → Follower snapshot transfer

**Heartbeat Messages:**
- `MsgHeartbeat` - Leader → Follower keepalive
- `MsgHeartbeatResponse` - Follower → Leader acknowledgment

**Client Request:**
- `MsgPropose` - Internal: propose new entry

**Cluster Membership:**
- `MsgRequestPreVote` - Pre-vote request (if enabled)
- `MsgRequestPreVoteResponse` - Pre-vote response
- `MsgTransferLeader` - Request leadership transfer

---

## Edge Cases and Corner Scenarios

### Case 1: Delayed Message After Leadership Change

**Scenario:**
```
1. Node 1 is leader in term 5
2. Network partition isolates Node 1
3. Nodes 2,3,4,5 elect Node 2 as leader in term 6
4. Partition heals
5. Node 1 sends AppendEntries with term 5
```

**Handling:**
```rust
// In AppendEntries receiver
if msg.term < current_term {
    // Reply with current_term to update stale leader
    send_append_response(msg.from, current_term, false);
    return;
}
```

**raft-rs Behavior:**
- Stale leader receives response with higher term
- Automatically steps down to follower
- Updates term to current_term

### Case 2: Snapshot Arrives During Log Replication

**Scenario:**
```
1. Follower is behind, receiving AppendEntries
2. Leader sends snapshot (follower too far behind)
3. AppendEntries and snapshot in flight simultaneously
```

**Handling:**
```rust
// In Ready processing
if !ready.snapshot().is_empty() {
    // Snapshot takes precedence
    storage.apply_snapshot(ready.snapshot())?;

    // Discard conflicting entries
    storage.compact_to(snapshot.metadata.index)?;
}

// AppendEntries with index < snapshot index are ignored
```

### Case 3: Commit Index Regresses

**Scenario:**
```
1. Leader commits entry at index 100
2. Follower's commit index = 100
3. Leader crashes
4. New leader elected with commit index = 95
```

**Safety:**
- Cannot happen! Leader Completeness ensures new leader has all committed entries
- New leader's commit index ≥ previous leader's commit index

**raft-rs Guarantee:**
- Election restriction prevents this scenario
- Candidate without entry at index 100 cannot win election

### Case 4: Split Vote in Election

**Scenario:**
```
1. Election timeout on Nodes 1 and 2 simultaneously
2. Both increment to term 7, vote for self
3. Node 1 gets vote from Node 3
4. Node 2 gets vote from Node 4
5. Node 5 hasn't voted yet but gets both requests
6. Node 5 votes for Node 1 (first request received)
7. Node 1 wins 3/5 majority
```

**Pre-Vote Optimization:**
```rust
config.pre_vote = true;

// Before real election, send MsgRequestPreVote
// Only start real election if pre-vote succeeds
// Prevents term inflation from partitioned nodes
```

### Case 5: Uncommitted Entry Overwritten

**Scenario (Figure 8 from paper):**
```
Term 1: Leader appends entry at index 3, crashes before commit
Term 2: New leader appends different entry at index 3
```

**Why This is Safe:**
- Uncommitted entries can be overwritten
- State machines never see uncommitted entries
- Only committed entries applied

**raft-rs Protection:**
```rust
// Follower overwrites conflicting entries
if log[prev_index + 1].term != entries[0].term {
    // Delete conflicting entry and all following
    log.truncate(prev_index + 1);
}
log.append(entries);
```

---

## Performance Characteristics

### Latency

**Write Latency (Commit Time):**
- **Minimum:** 1 RTT (leader → majority followers → leader)
- **Typical:** 1-3ms local network, 10-50ms cross-region
- **Factors:**
  - Network latency (dominant)
  - Disk sync time (if `must_sync = true`)
  - Number of followers (only need majority)

**Read Latency:**
- **Linearizable (ReadOnlySafe):** 1 RTT (quorum check)
- **Lease-based (ReadOnlyLeaseBased):** 0 RTT (local read)
- **Follower reads:** Always stale (no consistency guarantee)

### Throughput

**Theoretical Maximum:**
```
Throughput = (max_inflight_msgs × max_size_per_msg) / RTT
```

**Example:**
- `max_inflight_msgs = 256`
- `max_size_per_msg = 1MB`
- RTT = 1ms
- **Throughput ≈ 256 GB/s** (saturates network first)

**Practical Limits:**
- Disk bandwidth (fsync bottleneck)
- Network bandwidth
- CPU (serialization, state machine application)

**Optimization Strategies:**

1. **Batching:**
```rust
// Batch multiple proposals into one log entry
let batch = encode_batch(&proposals);
raw_node.propose(vec![], batch)?;
```

2. **Pipelining:**
```rust
// Increase inflight message limit
config.max_inflight_msgs = 512;
```

3. **Async I/O:**
```rust
// Don't block Raft loop on disk writes
raw_node.advance_append_async(ready);

// Later, after async write completes:
raw_node.on_persist_ready(persist_index);
```

4. **Group Commit:**
```rust
// Batch multiple Raft groups' writes
for (region_id, ready) in readys {
    batch.append(region_id, ready.entries());
}
disk.write_batch(batch)?;  // Single fsync
```

### Scalability

**Cluster Size:**
- **Odd numbers:** 3, 5, 7 (avoid even to prevent ties)
- **Practical limit:** 5-7 nodes (more = slower elections, more message traffic)
- **Fault tolerance:**
  - 3 nodes: tolerate 1 failure
  - 5 nodes: tolerate 2 failures
  - 7 nodes: tolerate 3 failures

**Multi-Raft Scaling:**
- Run 100s-1000s of Raft groups per process
- Each group replicates a data shard (region)
- Share transport, separate storage per group
- Example: TiKV runs ~10k regions per node

**Message Overhead:**
```
Messages per second ≈ (N-1) × heartbeat_rate
```
- 5 nodes, 10 Hz heartbeat: 40 msgs/sec
- 1000 Raft groups: 40k msgs/sec
- Batch heartbeats to reduce overhead

### Memory Usage

**Per RawNode:**
```
Memory = log_size + in_memory_entries + peer_metadata
```

**Optimization:**
- Snapshot and truncate log regularly
- Don't keep all committed entries in memory
- Use `max_uncommitted_size` to limit memory

**Example:**
```rust
// Trigger snapshot when log exceeds 10k entries
if raw_node.raft.raft_log.last_index() - first_index > 10_000 {
    create_snapshot()?;
    compact_log()?;
}
```

---

## Additional Resources

### Raft Paper Sections

- **Section 5:** Core Raft algorithm
- **Section 6:** Cluster membership changes (ConfChange)
- **Section 7:** Log compaction (snapshots)
- **Section 8:** Client interaction (linearizability)
- **Figure 2:** Raft algorithm summary (essential reference)
- **Figure 8:** Safety scenario (commitment rules)

### raft-rs Examples

- `examples/five_mem_node` - 5-node cluster with in-memory storage
- `examples/single_mem_node` - Single node (useful for testing)
- Look at `tests/` directory for comprehensive test patterns

### TiKV Implementation

- **Blog:** https://tikv.org/blog/implement-raft-in-rust/
- **Code:** https://github.com/tikv/tikv/tree/master/components/raftstore
- Production multi-raft implementation with rich optimizations

### Visualization Tools

- **Raft Scope:** http://raftscope.github.io/ (trace visualizer)
- **Raft Visualization:** https://raft.github.io/ (interactive demo)

---

**This reference complements SKILL.md with deeper technical details for complex scenarios.**
