---
name: raft-tui-integration
description: Expert understanding of raft-a-tui's dual-thread architecture (Raft Ready loop + ratatui TUI). Refactor and optimize integration between consensus layer and terminal UI. Use when working with TUI rendering, event loops, Ready loop patterns, channel communication, or performance optimization. Keywords: TUI, ratatui, event loop, Ready loop, refactor, integration, channels, state updates, performance, real-time, rendering
---

# Raft-TUI Integration Expert

Deep understanding of this project's unique architecture combining raft-rs consensus with ratatui terminal UI through channel-based message passing.

## When to Activate

This skill activates for tasks involving:
- **TUI rendering issues**: Lag, visual glitches, unresponsive UI
- **Ready loop modifications**: Changing Raft state processing logic
- **Integration refactoring**: Improving code between Raft and TUI threads
- **Performance optimization**: Reducing latency, improving responsiveness
- **Channel communication**: Adding/modifying state updates
- **Event handling**: User input processing, keyboard shortcuts
- **State synchronization**: Ensuring TUI reflects Raft state accurately

## Architecture Overview

### Dual-Thread Design

```
┌─────────────────────────────────────────────────────────────┐
│                        MAIN THREAD (TUI)                     │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Event Loop (50ms poll)                                 │ │
│  │ 1. Drain state_rx (try_recv) - non-blocking           │ │
│  │ 2. Render UI (4-pane layout)                           │ │
│  │ 3. Poll keyboard input                                 │ │
│  │ 4. Send commands to cmd_tx                             │ │
│  └────────────────────────────────────────────────────────┘ │
│                          ▲                 │                 │
│                    state_tx            cmd_rx                │
└────────────────────────────┼──────────────┼──────────────────┘
                             │              │
                             │              ▼
┌────────────────────────────┼──────────────┼──────────────────┐
│                    RAFT THREAD (Consensus)                    │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Ready Loop (raft_ready_loop)                           │ │
│  │ Phase 1: Input (cmd_rx, msg_rx, tick timer)            │ │
│  │ Phase 2: Ready check (has_ready)                       │ │
│  │ Phase 3: Process Ready:                                │ │
│  │   1. Apply snapshot                                    │ │
│  │   2. Append entries                                    │ │
│  │   3. Persist HardState                                 │ │
│  │   4. Send messages                                     │ │
│  │   5. Apply committed entries                           │ │
│  │   6. Send state_tx updates                             │ │
│  │   7. Advance                                           │ │
│  │   8. Process LightReady                                │ │
│  └────────────────────────────────────────────────────────┘ │
│                          ▲                                    │
│                     Network (TCP)                             │
└───────────────────────────────────────────────────────────────┘
```

### Key Components

**Channels:**
- `cmd_tx/cmd_rx`: User commands (PUT, CAMPAIGN, GET, etc.)
- `msg_tx/msg_rx`: Raft messages from network
- `state_tx/state_rx`: State updates for TUI visualization
- `shutdown_tx/shutdown_rx`: Graceful shutdown signal

**Files:**
- `src/main.rs`: TUI thread, App struct, event loop
- `src/raft_loop.rs`: Ready loop, state update sending
- `src/raft_node.rs`: RaftNode wrapper, callback tracking
- `src/storage.rs`: RaftStorage with applied_index tracking

**State Updates (StateUpdate enum):**
- `RaftState`: Term, role, leader, commit/applied indices
- `KvUpdate`: Key-value pair applied to store
- `LogEntry`: Committed log entry details
- `SystemMessage`: Human-readable event messages

## Critical Patterns to Preserve

### 1. Ready Loop Ordering (MUST NOT CHANGE)

The raft-rs Ready processing has a strict order mandated by the algorithm:

```rust
// Phase 3: Ready Processing (ORDER MATTERS!)
if has_ready() {
    let mut ready = raw_node.ready();

    // 1. Apply snapshot FIRST
    if !ready.snapshot().is_empty() {
        storage.apply_snapshot(ready.snapshot())?;
    }

    // 2. Append entries SECOND
    if !ready.entries().is_empty() {
        storage.append(ready.entries())?;
    }

    // 3. Persist HardState THIRD
    if let Some(hs) = ready.hs() {
        storage.set_hardstate(hs);
    }

    // 4. Send messages AFTER persistence
    for msg in ready.take_messages() {
        transport.send(msg)?;
    }

    // 5. Apply committed entries to state machine
    for entry in ready.take_committed_entries() {
        kv_node.apply_kv_command(&entry.data)?;
        state_tx.send(StateUpdate::KvUpdate { ... })?;
    }

    // 6. Advance to next cycle
    let light_rd = raw_node.advance(ready);

    // 7. Process LightReady
    // (messages, committed_entries)
}
```

**Why order matters:**
- Snapshot must be applied before entries (log compaction)
- Entries must be appended before HardState (durability)
- HardState must persist before sending messages (safety)
- Committed entries applied after persistence (consistency)

**When refactoring:** Never change this order. Extract helpers, but keep the sequence.

### 2. TUI Non-Blocking Pattern

The TUI must remain responsive at 20+ FPS:

```rust
loop {
    // 1. DRAIN all pending updates (non-blocking)
    while let Ok(update) = state_rx.try_recv() {
        app.apply_state_update(update);
    }

    // 2. Render current state (fast)
    terminal.draw(|frame| ui(frame, app))?;

    // 3. Poll input with SHORT timeout (50ms = 20 FPS)
    if event::poll(Duration::from_millis(50))? {
        if let Event::Key(key) = event::read()? {
            handle_key_event(key)?;
        }
    }
}
```

**Critical points:**
- Use `try_recv()` not `recv()` (non-blocking)
- Drain ALL updates before rendering (batch processing)
- Keep poll timeout ≤50ms for smooth updates
- Never block on channels in TUI thread

**When refactoring:** Maintain these timing guarantees. Don't add blocking calls.

### 3. Command Routing Logic

Commands route differently based on type:

```rust
match cmd {
    // Replicated commands → Raft proposal
    UserCommand::Put { key, value } => {
        raft_node.propose_command(key, value)?;
        // Response comes later via callback
    }

    UserCommand::Campaign => {
        raft_node.raw_node_mut().campaign()?;
        // Triggers election
    }

    // Read-only commands → local state
    UserCommand::Get { .. } |
    UserCommand::Keys |
    UserCommand::Status => {
        let output = kv_node.apply_user_command(cmd);
        state_tx.send(StateUpdate::SystemMessage(output))?;
    }
}
```

**Why split:**
- PUT/CAMPAIGN mutate Raft state → must go through consensus
- GET/KEYS/STATUS read local state → instant response, no replication

**When adding commands:** Categorize correctly (replicated vs local).

### 4. Ring Buffers for History

Prevent unbounded memory growth:

```rust
fn add_system_message(&mut self, msg: String) {
    if self.system_messages.len() >= 100 {
        self.system_messages.pop_front();
    }
    self.system_messages.push_back(msg);
}
```

**Pattern:** VecDeque with max size, pop_front when full.

**When refactoring:** Keep max sizes reasonable (50-100 items).

## Refactoring Guidelines

### Priority 1: Raft Correctness

**Never break:**
- Ready loop ordering (snapshot → entries → hardstate → send → apply → advance)
- Persistence before message sending
- Applied index tracking (`storage.set_applied_index()`)
- Callback invocation on committed entries

**Safe changes:**
- Extract helper functions (apply_entry, send_state_update)
- Add logging/metrics
- Improve error messages
- Add tests for edge cases

### Priority 2: Performance

**Optimization opportunities:**

**In Ready loop (raft_loop.rs):**
- Reduce allocations in hot loops
- Batch state updates before sending
- Cache decoded protobuf messages
- Use `take()` methods to avoid clones

**In TUI loop (main.rs):**
- Limit items rendered (already doing: `take(area.height)`)
- Avoid string allocations in render functions
- Use static strings where possible
- Profile with `cargo flamegraph`

**Channel sizing:**
- `cmd_rx`/`msg_rx`: Unbounded (external input, unpredictable rate)
- `state_rx`: Consider bounded (backpressure if TUI too slow)

**Anti-patterns to fix:**
- Blocking recv() in TUI thread
- Excessive cloning of large structures
- String formatting in tight loops
- Rendering more items than visible

### Priority 3: Real-Time UX

**Current targets:**
- 20+ FPS rendering (50ms poll timeout)
- <100ms latency from Raft event → TUI display
- Instant keyboard response

**Improvements:**

**Color coding:**
```rust
fn draw_system_logs(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app.system_messages
        .iter()
        .map(|msg| {
            let style = match classify_message(msg) {
                MessageType::Election => Style::default().fg(Color::Yellow),
                MessageType::LogReplication => Style::default().fg(Color::Green),
                MessageType::Error => Style::default().fg(Color::Red),
                _ => Style::default().fg(Color::Gray),
            };
            ListItem::new(msg.as_str()).style(style)
        })
        .collect();
    // ...
}
```

**Progress indicators:**
- Show "Proposing..." while waiting for commit
- Highlight current leader in peer list
- Visual feedback for state transitions

**Keyboard shortcuts:**
- 'p' for quick PUT
- 'c' for CAMPAIGN
- 'k' for KEYS
- Tab to cycle focus between panes

### Priority 4: Maintainability

**Code organization:**

**Extract repeated logic:**
```rust
// Current: Duplicated entry application in Ready and LightReady
// Better:
fn apply_committed_entry(
    entry: &Entry,
    kv_node: &mut Node,
    state_tx: &Sender<StateUpdate>,
    callbacks: &mut HashMap<Vec<u8>, Sender<CommandResponse>>,
    storage: &mut RaftStorage,
) -> Result<(), Error> {
    // Single implementation
}
```

**Separate concerns:**
- `raft_loop.rs`: Pure Raft logic (no UI concerns)
- `main.rs`: Pure TUI logic (no Raft details)
- `raft_node.rs`: Raft wrapper interface
- Keep state updates generic (`StateUpdate` enum)

**Testing:**
- Unit test helpers independently
- Integration test Ready loop with mock transport
- Test TUI state updates with mock channel

**Documentation:**
- Comment WHY not WHAT (e.g., "// Persist before send - required by Raft safety")
- Document invariants (e.g., "// INVARIANT: applied_index <= commit_index")
- Add examples to public functions

## Common Refactoring Tasks

### Task 1: Add New State Update Type

**Steps:**
1. Add variant to `StateUpdate` enum in `raft_loop.rs`:
   ```rust
   pub enum StateUpdate {
       // ...
       PeerConnectivity { peer_id: u64, connected: bool },
   }
   ```

2. Send update in Ready loop at appropriate point:
   ```rust
   // After sending messages
   for peer_id in peers {
       let connected = transport.is_connected(peer_id);
       state_tx.send(StateUpdate::PeerConnectivity { peer_id, connected })?;
   }
   ```

3. Handle in TUI `apply_state_update()`:
   ```rust
   StateUpdate::PeerConnectivity { peer_id, connected } => {
       self.peer_status.insert(peer_id, connected);
   }
   ```

4. Render in appropriate pane:
   ```rust
   fn draw_peer_status(frame: &mut Frame, app: &App, area: Rect) {
       // Use app.peer_status to show connectivity
   }
   ```

### Task 2: Optimize Ready Loop Performance

**Before:**
```rust
// Decoding twice (inefficient)
match kv_node.apply_kv_command(&entry.data) {
    Ok(_) => {
        if let Ok(cmd) = decode::<KvCommand>(&entry.data) {
            // Extract key/value for update
        }
    }
}
```

**After:**
```rust
// Decode once, apply, and extract in one pass
match decode::<KvCommand>(&entry.data) {
    Ok(cmd) => {
        if let Some(Cmd::Put(put)) = cmd.cmd {
            kv_node.apply_put(&put.key, &put.value)?;
            state_tx.send(StateUpdate::KvUpdate {
                key: put.key,
                value: put.value
            })?;
        }
    }
}
```

**Strategy:** Return values from methods instead of re-computing.

### Task 3: Add Visual Feedback

**Example: Show proposal status**

1. Add state to App:
   ```rust
   struct App {
       // ...
       pending_proposals: HashMap<String, Instant>,  // key → time proposed
   }
   ```

2. Track proposals:
   ```rust
   UserCommand::Put { key, value } => {
       app.pending_proposals.insert(key.clone(), Instant::now());
       cmd_tx.send(UserCommand::Put { key, value })?;
   }
   ```

3. Remove on commit:
   ```rust
   StateUpdate::KvUpdate { key, .. } => {
       if let Some(start) = app.pending_proposals.remove(&key) {
           let latency = start.elapsed();
           app.add_system_message(format!("✓ Committed in {:?}", latency));
       }
   }
   ```

4. Render pending with indicator:
   ```rust
   for (key, time) in &app.pending_proposals {
       items.push(ListItem::new(format!("{} ⏳ pending...", key)));
   }
   ```

## Testing Integration Changes

### Test Strategy

**Unit tests:**
- Test helpers in isolation (e.g., `apply_committed_entry`)
- Mock channels with crossbeam test receivers/senders
- Verify state update messages contain expected data

**Integration tests:**
```rust
#[test]
fn test_ready_loop_sends_state_updates() {
    let (state_tx, state_rx) = unbounded();
    let (cmd_tx, cmd_rx) = unbounded();

    // Setup mock Raft node, transport
    let raft_node = create_test_raft_node();

    // Spawn Ready loop in thread
    thread::spawn(|| {
        raft_ready_loop(raft_node, ..., state_tx, ...)
    });

    // Send PUT command
    cmd_tx.send(UserCommand::Put {
        key: "test".into(),
        value: "123".into()
    })?;

    // Wait for state update
    let update = state_rx.recv_timeout(Duration::from_secs(5))?;
    assert!(matches!(update, StateUpdate::KvUpdate { .. }));
}
```

**TUI tests:**
```rust
#[test]
fn test_tui_applies_state_updates() {
    let (state_tx, state_rx) = unbounded();

    // Create app
    let mut app = App::new(1, state_rx, ...);

    // Send mock update
    state_tx.send(StateUpdate::KvUpdate {
        key: "foo".into(),
        value: "bar".into()
    })?;

    // Drain updates (simulate TUI loop)
    while let Ok(update) = app.state_rx.try_recv() {
        app.apply_state_update(update);
    }

    // Verify state
    assert_eq!(app.kv_store.get("foo"), Some(&"bar".to_string()));
}
```

### Debugging Integration Issues

**Raft thread not sending updates?**
- Check `state_tx.send()` return value
- Add debug logging around send points
- Verify Ready loop is reaching state update code

**TUI not displaying updates?**
- Check `try_recv()` is being called
- Verify `apply_state_update()` is updating App state
- Check render functions are reading updated state

**State out of sync?**
- Ensure KV updates sent AFTER successful apply
- Check applied_index tracking
- Verify no duplicate applications

**Performance degradation?**
- Profile with `cargo flamegraph`
- Check for blocking recv() calls
- Measure state_rx queue depth
- Count allocations in hot loops

## Anti-Patterns to Avoid

### ❌ Blocking the TUI Thread

```rust
// WRONG: Blocks until state arrives
while let Ok(update) = app.state_rx.recv() {
    app.apply_state_update(update);
}
```

```rust
// RIGHT: Non-blocking drain
while let Ok(update) = app.state_rx.try_recv() {
    app.apply_state_update(update);
}
```

### ❌ Changing Ready Loop Order

```rust
// WRONG: Sending before persisting
for msg in ready.take_messages() {
    transport.send(msg)?;
}
if let Some(hs) = ready.hs() {
    storage.set_hardstate(hs);  // TOO LATE!
}
```

### ❌ Forgetting Applied Index

```rust
// WRONG: Missing applied_index tracking
for entry in ready.take_committed_entries() {
    kv_node.apply_kv_command(&entry.data)?;
    // Missing: storage.set_applied_index(entry.index);
}
```

### ❌ Unbounded Growth

```rust
// WRONG: No max size
fn add_system_message(&mut self, msg: String) {
    self.system_messages.push_back(msg);  // Grows forever!
}
```

## Implementation Checklist

When making integration changes, verify:

- [ ] Ready loop ordering preserved
- [ ] No blocking calls in TUI thread
- [ ] Applied index updated after each committed entry
- [ ] State updates sent AFTER successful state machine application
- [ ] Ring buffers have max size limits
- [ ] Tests added for new behavior
- [ ] `cargo test` passes
- [ ] `cargo clippy` clean
- [ ] `cargo fmt` applied
- [ ] Performance acceptable (run 3-node cluster, monitor FPS)

## Quick Reference

**File locations:**
- Ready loop: `src/raft_loop.rs:93` (raft_ready_loop function)
- TUI loop: `src/main.rs:520` (run_tui function)
- State updates: `src/raft_loop.rs:16` (StateUpdate enum)
- App state: `src/main.rs:37` (App struct)

**Channel patterns:**
- Unbounded: `crossbeam_channel::unbounded()`
- Bounded: `crossbeam_channel::bounded(100)`
- Try receive: `rx.try_recv()` (returns Err if empty)
- Timeout receive: `rx.recv_timeout(duration)` (waits up to duration)
- Select: `crossbeam_channel::select!` (multiplex multiple channels)

**Common errors:**
- `ChannelClosed`: Sender dropped, loop should exit
- `ChannelFull`: Bounded channel full, decide whether to block or drop
- `StorageError`: Fatal, return error from Ready loop
- `TransportError`: Non-fatal, log and continue

For detailed patterns and examples, see REFERENCE.md.
