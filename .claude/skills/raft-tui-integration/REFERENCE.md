# Raft-TUI Integration Reference Guide

Comprehensive reference for understanding and modifying the integration between raft-rs consensus and ratatui TUI in this project.

## Table of Contents

1. [Ready Loop Deep Dive](#ready-loop-deep-dive)
2. [TUI Event Loop Deep Dive](#tui-event-loop-deep-dive)
3. [Channel Communication Patterns](#channel-communication-patterns)
4. [State Update Flow](#state-update-flow)
5. [Performance Optimization](#performance-optimization)
6. [Testing Patterns](#testing-patterns)
7. [Debugging Techniques](#debugging-techniques)

---

## Ready Loop Deep Dive

### Complete Phase Breakdown

The `raft_ready_loop` function in `src/raft_loop.rs:93` implements the canonical raft-rs integration pattern.

#### Phase 1: Input Reception

**Location:** `src/raft_loop.rs:108-212`

**Pattern:** Multiplexed channel selection with tick timing

```rust
let tick_duration = Duration::from_millis(100);
let mut tick_timer = Instant::now();

loop {
    // Calculate timeout until next tick
    let timeout = tick_duration.saturating_sub(tick_timer.elapsed());

    // Wait for input using crossbeam select!
    select! {
        recv(cmd_rx) -> result => {
            // Handle user commands
            match result {
                Ok(UserCommand::Put { key, value }) => {
                    // Propose to Raft
                    raft_node.propose_command(key, value)?;
                }
                Ok(UserCommand::Campaign) => {
                    // Trigger election
                    raft_node.raw_node_mut().campaign()?;
                }
                Ok(UserCommand::Get { .. }) |
                Ok(UserCommand::Keys) |
                Ok(UserCommand::Status) => {
                    // Local read-only operation
                    let output = kv_node.apply_user_command(cmd);
                    state_tx.send(StateUpdate::SystemMessage(output))?;
                }
                Err(_) => {
                    // Channel closed - shutdown
                    return Err(ChannelClosed("cmd_rx"));
                }
            }
        },
        recv(msg_rx) -> result => {
            // Handle Raft messages from peers
            match result {
                Ok(msg) => {
                    // Step Raft state machine
                    if let Err(e) = raft_node.raw_node_mut().step(msg) {
                        // Log but continue - step() errors are non-fatal
                        warn!(logger, "Failed to step"; "error" => e);
                    }
                }
                Err(_) => {
                    return Err(ChannelClosed("msg_rx"));
                }
            }
        },
        recv(shutdown_rx) -> _ => {
            // Graceful shutdown
            return Ok(());
        },
        default(timeout) => {
            // No input received - continue to tick check
        },
    }

    // Tick if enough time elapsed
    if tick_timer.elapsed() >= tick_duration {
        raft_node.raw_node_mut().tick();
        tick_timer = Instant::now();
    }
}
```

**Key Points:**

1. **Tick timing:** Uses `saturating_sub()` to calculate remaining time until next tick
   - If tick is overdue, returns 0 (immediate timeout)
   - Ensures ticks happen every 100ms ±jitter from processing time

2. **Command routing:**
   - PUT/CAMPAIGN → `propose()` or `campaign()` (mutates Raft state)
   - GET/KEYS/STATUS → local `apply_user_command()` (read-only)

3. **Error handling:**
   - `step()` errors: Log and continue (resilience to malformed messages)
   - Channel closed: Fatal, return error (graceful shutdown)
   - Transport errors: Log and continue (network may recover)

4. **Shutdown:** Controlled via `shutdown_rx` channel from TUI thread

#### Phase 2: Ready Check

**Location:** `src/raft_loop.rs:214-217`

```rust
if !raft_node.raw_node().has_ready() {
    continue;
}

let mut ready = raft_node.raw_node_mut().ready();
```

**Why separate check:**
- `has_ready()` is cheap (checks flags)
- `ready()` is expensive (constructs Ready struct, copies data)
- Only call `ready()` when there's actually work to do

**When is Ready?**
- New messages to send (heartbeats, responses)
- New entries to append (from proposals or leader)
- HardState changed (term/vote/commit updated)
- Snapshot to apply
- Committed entries to apply to state machine

#### Phase 3: Ready Processing

**Critical: THIS ORDER CANNOT CHANGE**

##### Step 3.1: Apply Snapshot

**Location:** `src/raft_loop.rs:229-242`

```rust
if !ready.snapshot().is_empty() {
    let snapshot = ready.snapshot();
    storage.apply_snapshot(snapshot.clone())?;
    // TODO: Apply snapshot to KV store
}
```

**Why first:**
- Snapshot replaces entire log (compaction)
- Must apply before processing any entries
- Contains state machine state + ConfState (cluster membership)

**Production implementation:**
```rust
if !ready.snapshot().is_empty() {
    let snapshot = ready.snapshot();

    // 1. Apply to Raft storage
    storage.apply_snapshot(snapshot.clone())?;

    // 2. Decode and apply to KV store
    let kv_state: BTreeMap<String, String> =
        bincode::deserialize(&snapshot.data)?;
    kv_node.restore_from_snapshot(kv_state);

    // 3. Update applied index from snapshot metadata
    let applied = snapshot.get_metadata().index;
    storage.set_applied_index(applied);

    info!(logger, "Applied snapshot";
          "index" => applied,
          "kv_entries" => kv_state.len());
}
```

##### Step 3.2: Append Entries

**Location:** `src/raft_loop.rs:244-253`

```rust
if !ready.entries().is_empty() {
    let entries = ready.entries();
    storage.append(entries)?;
}
```

**Why second:**
- New log entries from leader or local proposals
- Must persist before updating HardState commit index
- Uses `MemStorage::append()` (handles index validation)

**Error handling:**
```rust
if let Err(e) = storage.append(entries) {
    error!(logger, "Failed to append"; "error" => e);
    // Fatal - log corruption, must exit
    return Err(StorageError(e));
}
```

##### Step 3.3: Persist HardState

**Location:** `src/raft_loop.rs:255-263`

```rust
if let Some(hs) = ready.hs() {
    storage.set_hardstate(hs.clone());
}
```

**Why third:**
- HardState = (term, vote, commit)
- Must persist term/vote BEFORE sending messages (safety)
- Prevents voting for multiple candidates in same term after crash

**Critical fields:**
- `term`: Current term (monotonically increasing)
- `vote`: Who we voted for in this term (at most one per term)
- `commit`: Highest known committed index

**Production pattern:**
```rust
if let Some(hs) = ready.hs() {
    debug!(logger, "Persisting HardState";
           "term" => hs.term,
           "vote" => hs.vote,
           "commit" => hs.commit);

    // For real storage, this must be durable (fsync)
    storage.set_hardstate(hs.clone());
}
```

##### Step 3.4: Send Messages

**Location:** `src/raft_loop.rs:265-280`

**TWO message types with different timing:**

```rust
// Regular messages - can send before persistence completes
let messages = ready.take_messages();
for msg in messages {
    if let Err(e) = transport.send(msg.to, msg) {
        warn!(logger, "Failed to send"; "to" => msg.to, "error" => e);
        // Non-fatal - network may be down
    }
}

// Persisted messages - MUST send AFTER persistence completes
let persisted_messages = ready.take_persisted_messages();
for msg in persisted_messages {
    if let Err(e) = transport.send(msg.to, msg) {
        warn!(logger, "Failed to send persisted"; "to" => msg.to, "error" => e);
    }
}
```

**Why the split:**
- Regular messages: Acknowledgments, queries (no safety risk if lost)
- Persisted messages: Votes, append confirmations (MUST be sent after disk write)

**Message types:**
```rust
match msg.msg_type() {
    MessageType::MsgHeartbeat => {
        // Leader → followers, regular interval
        // Can send immediately
    }
    MessageType::MsgAppend => {
        // Leader → followers, log replication
        // Usually regular messages (no safety risk)
    }
    MessageType::MsgRequestVote => {
        // Candidate → all, requesting vote
        // PERSISTED - must record vote before sending
    }
    MessageType::MsgRequestVoteResponse => {
        // Voter → candidate, vote response
        // PERSISTED - must record vote before responding
    }
}
```

##### Step 3.5: Apply Committed Entries

**Location:** `src/raft_loop.rs:297-380`

**Most complex step - applies consensus decisions to state machine**

```rust
let committed_entries = ready.take_committed_entries();
for entry in committed_entries {
    // Skip empty entries (from leader election)
    if entry.data.is_empty() {
        continue;
    }

    match entry.get_entry_type() {
        EntryType::EntryNormal => {
            // 1. Apply to KV store
            match kv_node.apply_kv_command(&entry.data) {
                Ok(_) => {
                    // 2. Extract key/value for state update
                    if let Ok(cmd) = decode::<KvCommand>(&entry.data) {
                        if let Some(Cmd::Put(put)) = cmd.cmd {
                            // 3. Send TUI update
                            state_tx.send(StateUpdate::KvUpdate {
                                key: put.key.clone(),
                                value: put.value.clone(),
                            })?;

                            // 4. Send log entry update
                            state_tx.send(StateUpdate::LogEntry {
                                index: entry.index,
                                term: entry.term,
                                data: format!("PUT {} = {}", put.key, put.value),
                            })?;

                            // 5. Invoke callback (if proposal came from this node)
                            if let Some(callback) = raft_node.take_callback(&entry.context) {
                                callback.send(CommandResponse::Success {
                                    key: put.key,
                                    value: put.value,
                                })?;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(logger, "Apply failed"; "error" => e);
                    // Still invoke callback with error
                    if let Some(callback) = raft_node.take_callback(&entry.context) {
                        callback.send(CommandResponse::Error(e.to_string()))?;
                    }
                }
            }

            // 6. Update applied index AFTER application
            storage.set_applied_index(entry.index);
        }
        EntryType::EntryConfChange | EntryType::EntryConfChangeV2 => {
            // Cluster membership changes
            // TODO: Implement when we support dynamic membership
            storage.set_applied_index(entry.index);
        }
    }
}
```

**Critical points:**

1. **Empty entries:** Leader election creates empty entry at start of term
   - Skip application (no data)
   - Don't update applied_index (will be updated when next real entry applied)

2. **Application order:** MUST apply in log order (index 1, 2, 3, ...)
   - Raft guarantees entries arrive in committed order
   - State machine must apply sequentially

3. **Callback tracking:**
   - When proposing: `raw_node.propose(callback_id, data)`
   - Store mapping: `callbacks.insert(callback_id, response_sender)`
   - On commit: Look up callback_id from `entry.context`, send response
   - One-time: Remove from map after sending

4. **Applied index:** Track highest applied entry
   - Prevents re-application after restart
   - MUST persist in snapshots (see CLAUDE.md warning)
   - Set AFTER successful application

5. **Error handling:**
   - Apply errors: Log, send error response to callback, continue
   - State update send errors: Log, continue (TUI may be disconnected)
   - Storage errors: Fatal, return (corruption)

##### Step 3.6: Send Raft State Update

**Location:** `src/raft_loop.rs:382-387`

```rust
let raft_state = raft_node.get_state();
if let Err(_) = state_tx.send(StateUpdate::RaftState(raft_state)) {
    debug!(logger, "Failed to send state (TUI disconnected?)");
}
```

**When:** After all processing, before advance()

**Why:** TUI shows current Raft state (term, role, leader, indices)

**RaftState fields:**
```rust
pub struct RaftState {
    pub node_id: u64,
    pub term: u64,
    pub role: StateRole,      // Leader, Follower, Candidate
    pub leader_id: u64,
    pub commit_index: u64,
    pub applied_index: u64,
}
```

##### Step 3.7: Advance

**Location:** `src/raft_loop.rs:389-390`

```rust
let mut light_rd = raft_node.raw_node_mut().advance(ready);
```

**Critical:** Must call after processing ALL Ready state

**What advance() does:**
- Marks current Ready as processed
- Prepares for next Ready cycle
- Returns LightReady with additional work

**LightReady vs Ready:**
- Ready: Full state snapshot (entries, messages, hardstate, etc.)
- LightReady: Incremental updates (only messages and committed entries)
- LightReady is cheaper (no allocation of entry slices)

##### Step 3.8: Process LightReady

**Location:** `src/raft_loop.rs:392-473`

**Why needed:** Additional messages/entries may be generated during advance()

```rust
// Check for additional committed entries
let light_committed = light_rd.take_committed_entries();
if !light_committed.is_empty() {
    for entry in light_committed {
        // Same logic as Ready committed entries
        // (duplicated code - could extract helper)
    }
}

// Send additional messages
for msg in light_rd.messages() {
    transport.send(msg.to, msg.clone())?;
}
```

**Note:** This implementation duplicates entry application logic from Ready.
**Refactoring opportunity:** Extract `apply_committed_entry()` helper.

---

## TUI Event Loop Deep Dive

### Complete Loop Breakdown

**Location:** `src/main.rs:520-554`

The TUI loop runs in the main thread, providing real-time visualization at 20+ FPS.

#### Loop Structure

```rust
fn run_tui(app: &mut App) -> Result<(), Box<dyn Error>> {
    let mut terminal = setup_terminal()?;

    let result = (|| -> Result<(), Box<dyn Error>> {
        loop {
            // Step 1: Drain state updates (non-blocking)
            while let Ok(update) = app.state_rx.try_recv() {
                app.apply_state_update(update);
            }

            // Step 2: Render current state
            terminal.draw(|frame| ui(frame, app))?;

            // Step 3: Poll keyboard input (50ms timeout)
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match app.handle_key_event(key)? {
                        KeyAction::Quit => {
                            app.shutdown_tx.send(())?;
                            break;
                        }
                        KeyAction::Continue => {}
                    }
                }
            }
        }
        Ok(())
    })();

    // Always restore terminal state
    restore_terminal(terminal)?;
    result
}
```

#### Step 1: Drain State Updates

**Pattern:** Non-blocking batch processing

```rust
while let Ok(update) = app.state_rx.try_recv() {
    app.apply_state_update(update);
}
```

**Why `try_recv()` not `recv()`:**
- `recv()` blocks until message arrives (freezes UI)
- `try_recv()` returns immediately if empty (responsive)

**Why loop until empty:**
- Batch all pending updates before rendering
- Avoids redundant renders if multiple updates queued
- Ensures UI shows most recent state

**Implementation:**

```rust
fn apply_state_update(&mut self, update: StateUpdate) {
    match update {
        StateUpdate::RaftState(state) => {
            self.raft_state = Some(state);
        }
        StateUpdate::KvUpdate { key, value } => {
            self.kv_store.insert(key.clone(), value.clone());
            self.add_system_message(format!("KV: {} = {}", key, value));
        }
        StateUpdate::LogEntry { index, term, data } => {
            self.add_system_message(format!(
                "Log: idx={} term={} data={}",
                index, term, data
            ));
        }
        StateUpdate::SystemMessage(msg) => {
            self.add_system_message(msg);
        }
    }
}
```

**State maintained:**
- `raft_state`: Latest Raft state (term, role, leader, etc.)
- `kv_store`: Current KV pairs (built from KvUpdate messages)
- `system_messages`: Ring buffer of events (max 100)
- `command_history`: Ring buffer of commands (max 50)

#### Step 2: Render Current State

**Pattern:** Immediate-mode UI (redraw entire screen each frame)

```rust
terminal.draw(|frame| ui(frame, app))?;
```

**Why full redraw:**
- ratatui is immediate-mode (stateless rendering)
- Terminal backend diffs and sends only changes
- Simpler than retained-mode (tracking widget state)

**Layout structure:**

```
┌─────────────────────────────────────────────────────┐
│ Title Bar (3 lines)                                 │
│ Node 1 | Term: 5 | Role: Leader | Leader: 1         │
├──────────────────────┬──────────────────────────────┤
│ Raft State (40%)     │ KV Store (50%)               │
│                      │                              │
│ Term: 5              │ foo = bar                    │
│ Role: Leader         │ x = 123                      │
│ Leader ID: 1         │                              │
│ Commit: 10           │                              │
│ Applied: 10          │                              │
├──────────────────────┼──────────────────────────────┤
│ System Logs (60%)    │ Command History (50%)        │
│                      │                              │
│ KV: foo = bar        │ PUT foo bar                  │
│ Log: idx=10 ...      │ CAMPAIGN                     │
│ Starting election... │ GET foo                      │
│ ...                  │ ...                          │
├──────────────────────┴──────────────────────────────┤
│ Input Area (3 lines)                                │
│ > PUT x 123_                                        │
└─────────────────────────────────────────────────────┘
```

**Rendering performance:**
- Limit visible items: `take(area.height - 2)`
- Most recent first: `iter().rev()`
- No allocations in render functions (use slices/iterators)

**Color coding:**
```rust
fn draw_system_logs(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app.system_messages
        .iter()
        .rev()
        .take(area.height as usize - 2)
        .map(|msg| {
            // TODO: Color by message type
            // Could parse prefixes: "KV:", "Log:", "Error:", etc.
            let style = Style::default().fg(Color::Gray);
            ListItem::new(msg.as_str()).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("System Logs"));

    frame.render_widget(list, area);
}
```

**Refactoring opportunity:** Add color coding based on message content.

#### Step 3: Poll Keyboard Input

**Pattern:** Short timeout poll for responsiveness

```rust
if event::poll(Duration::from_millis(50))? {
    if let Event::Key(key) = event::read()? {
        match app.handle_key_event(key)? {
            KeyAction::Quit => {
                app.shutdown_tx.send(())?;
                break;
            }
            KeyAction::Continue => {}
        }
    }
}
```

**Why 50ms timeout:**
- 1000ms / 50ms = 20 FPS minimum
- Even without keyboard input, UI updates 20x per second
- Fast enough for smooth Raft state updates

**Input modes:**

```rust
enum InputMode {
    Normal,   // 'i' to edit, 'q' to quit
    Editing,  // Type commands, Enter to submit, Esc to cancel
}
```

**Mode switching:**
```rust
fn handle_key_event(&mut self, key: KeyEvent) -> KeyResult {
    match self.input_mode {
        InputMode::Normal => match key.code {
            KeyCode::Char('q') => Quit,
            KeyCode::Char('i') => {
                self.input_mode = InputMode::Editing;
                Continue
            }
        },
        InputMode::Editing => match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                Continue
            }
            KeyCode::Char('c') if key.modifiers == CONTROL => Quit,
            KeyCode::Enter => self.submit_command(),
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_position, c);
                self.cursor_position += 1;
                Continue
            }
            KeyCode::Backspace => { /* delete char */ },
            KeyCode::Left => { /* move cursor */ },
            KeyCode::Right => { /* move cursor */ },
        },
    }
}
```

**Command submission:**

```rust
fn submit_command(&mut self) -> KeyResult {
    let input = self.input.drain(..).collect::<String>();
    self.cursor_position = 0;

    // Special quit commands
    if input.trim() == "quit" || input.trim() == "exit" {
        return Quit;
    }

    // Parse and send to Raft thread
    match parse_command(&input) {
        Some(cmd) => {
            self.cmd_tx.send(cmd)?;
            self.add_command_history(input.clone());
            self.add_system_message(format!("Sent: {}", input));
            Continue
        }
        None => {
            self.add_system_message(format!("Unknown: {}", input));
            Continue
        }
    }
}
```

---

## Channel Communication Patterns

### Channel Types and Usage

#### 1. Unbounded Channels

**Used for:** External input (unpredictable rate)

```rust
let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
let (msg_tx, msg_rx) = crossbeam_channel::unbounded();
let (shutdown_tx, shutdown_rx) = crossbeam_channel::unbounded();
```

**Why unbounded:**
- User input rate unknown (could be bursty)
- Network message rate unknown (could be flood)
- Don't want to block senders (TCP transport, TUI)

**Risk:** Unbounded growth if receiver too slow

**Mitigation:**
- Ready loop processes at high throughput (~100 msg/sec)
- TUI drains at 20+ FPS (fast enough for user perception)
- Shutdown is single message (no growth)

#### 2. Bounded Channels

**Used for:** State updates (controlled producer)

```rust
// Consider bounded for backpressure
let (state_tx, state_rx) = crossbeam_channel::bounded(100);
```

**Why bounded:**
- State updates generated by Ready loop (controlled rate)
- If TUI too slow, better to block Ready loop than OOM
- 100 messages = ~5 seconds of updates at 20 FPS

**Tradeoff:**
- Bounded: Backpressure (slows Raft if TUI frozen)
- Unbounded: OOM risk (memory grows if TUI frozen)

**Current choice:** Unbounded (TUI assumed fast enough)

**Future improvement:** Monitor state_rx queue depth, add bounded with smart overflow handling

#### 3. Select Pattern

**Used for:** Multiplexing multiple inputs

```rust
use crossbeam_channel::select;

select! {
    recv(cmd_rx) -> msg => {
        // Handle user command
    },
    recv(msg_rx) -> msg => {
        // Handle network message
    },
    recv(shutdown_rx) -> _ => {
        // Handle shutdown
    },
    default(timeout) => {
        // Timeout - no message received
    },
}
```

**Guarantees:**
- Receives from first ready channel
- Fair: Doesn't starve any channel
- Timeout: Allows periodic work (ticks)

**Pattern in Ready loop:**
```rust
let timeout = tick_duration.saturating_sub(tick_timer.elapsed());
select! {
    recv(cmd_rx) -> result => { /* ... */ },
    recv(msg_rx) -> result => { /* ... */ },
    recv(shutdown_rx) -> _ => { return Ok(()); },
    default(timeout) => {
        // No input - continue to tick check
    },
}
```

### State Update Flow Example

**Scenario:** User types "PUT foo bar" and presses Enter

1. **TUI thread (main.rs):**
   ```rust
   // User presses Enter in Editing mode
   submit_command() {
       cmd_tx.send(UserCommand::Put {
           key: "foo".into(),
           value: "bar".into(),
       })?;
   }
   ```

2. **Raft thread (raft_loop.rs):**
   ```rust
   // Receive in Ready loop Phase 1
   select! {
       recv(cmd_rx) -> Ok(UserCommand::Put { key, value }) => {
           // Generate unique callback ID
           let callback_id = Uuid::new_v4().as_bytes().to_vec();
           let (tx, rx) = bounded(1);
           callbacks.insert(callback_id.clone(), tx);

           // Encode command
           let data = encode_put_command(&key, &value);

           // Propose to Raft
           raw_node.propose(callback_id, data)?;

           // Send "Proposing..." update
           state_tx.send(StateUpdate::SystemMessage(
               format!("Proposing: PUT {} = {}", key, value)
           ))?;
       }
   }
   ```

3. **Raft consensus (inside raft-rs):**
   - If leader: Append to log, replicate to followers
   - If follower: Forward to leader (or fail proposal)
   - Wait for quorum (2/3 nodes)
   - Mark entry as committed

4. **Raft thread (Ready loop Phase 3.5):**
   ```rust
   // Process committed entry
   for entry in ready.take_committed_entries() {
       if let Ok(cmd) = decode::<KvCommand>(&entry.data) {
           if let Some(Cmd::Put(put)) = cmd.cmd {
               // Apply to KV store
               kv_node.apply_put(&put.key, &put.value)?;

               // Send KV update to TUI
               state_tx.send(StateUpdate::KvUpdate {
                   key: put.key.clone(),
                   value: put.value.clone(),
               })?;

               // Send log entry update
               state_tx.send(StateUpdate::LogEntry {
                   index: entry.index,
                   term: entry.term,
                   data: format!("PUT {} = {}", put.key, put.value),
               })?;

               // Invoke callback
               if let Some(callback) = callbacks.remove(&entry.context) {
                   callback.send(CommandResponse::Success {
                       key: put.key,
                       value: put.value,
                   })?;
               }
           }
       }
   }
   ```

5. **TUI thread (main.rs):**
   ```rust
   // Drain state updates (Step 1 of TUI loop)
   while let Ok(update) = state_rx.try_recv() {
       match update {
           StateUpdate::KvUpdate { key, value } => {
               // Update local KV store view
               app.kv_store.insert(key.clone(), value.clone());
               app.add_system_message(format!("KV: {} = {}", key, value));
           }
           StateUpdate::LogEntry { index, term, data } => {
               app.add_system_message(format!(
                   "Log: idx={} term={} data={}",
                   index, term, data
               ));
           }
           _ => { /* ... */ }
       }
   }

   // Render updated state (Step 2 of TUI loop)
   terminal.draw(|frame| ui(frame, app))?;
   ```

6. **User sees in TUI:**
   - KV Store pane: "foo = bar"
   - System Logs: "Proposing: PUT foo = bar"
   - System Logs: "KV: foo = bar"
   - System Logs: "Log: idx=10 term=5 data=PUT foo = bar"

**Total latency:** ~100-300ms (depends on cluster size, network)

---

## Performance Optimization

### Profiling Tools

**CPU profiling:**
```bash
cargo install flamegraph
cargo flamegraph --bin raft-a-tui -- --id 1 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
```

**Memory profiling:**
```bash
cargo install cargo-instruments
cargo instruments --bin raft-a-tui --template Allocations
```

**Benchmark critical paths:**
```bash
cargo bench --bench ready_loop
cargo bench --bench tui_rendering
```

### Common Bottlenecks

#### 1. Excessive Allocations in Hot Loops

**Bad:**
```rust
for entry in ready.take_committed_entries() {
    let cmd_str = format!("PUT {} = {}", key, value);  // Allocation
    let msg = format!("Log: {}", cmd_str);  // Another allocation
    state_tx.send(StateUpdate::SystemMessage(msg))?;
}
```

**Good:**
```rust
for entry in ready.take_committed_entries() {
    // Pre-allocated buffer (reuse across iterations)
    let mut buf = String::with_capacity(64);
    write!(buf, "PUT {} = {}", key, value)?;
    state_tx.send(StateUpdate::SystemMessage(buf))?;
}
```

**Best:** Avoid allocations entirely
```rust
// Send structured data, format later in TUI thread
state_tx.send(StateUpdate::LogEntry {
    index: entry.index,
    term: entry.term,
    data: format!("PUT {} = {}", key, value),  // Once, not in loop
})?;
```

#### 2. Blocking Operations in TUI Thread

**Bad:**
```rust
// Blocks until message arrives
let update = state_rx.recv()?;  // FREEZES UI!
app.apply_state_update(update);
terminal.draw(|frame| ui(frame, app))?;
```

**Good:**
```rust
// Non-blocking drain
while let Ok(update) = state_rx.try_recv() {
    app.apply_state_update(update);
}
terminal.draw(|frame| ui(frame, app))?;
```

#### 3. Redundant Decoding

**Current issue in raft_loop.rs:**
```rust
// Decode once for application
kv_node.apply_kv_command(&entry.data)?;

// Decode AGAIN for state update
if let Ok(cmd) = decode::<KvCommand>(&entry.data) {  // Redundant!
    if let Some(Cmd::Put(put)) = cmd.cmd {
        state_tx.send(StateUpdate::KvUpdate {
            key: put.key,
            value: put.value,
        })?;
    }
}
```

**Fixed:**
```rust
// Decode once, use twice
if let Ok(cmd) = decode::<KvCommand>(&entry.data) {
    if let Some(Cmd::Put(put)) = cmd.cmd {
        // Apply
        kv_node.apply_put(&put.key, &put.value)?;

        // Send update (reuse decoded data)
        state_tx.send(StateUpdate::KvUpdate {
            key: put.key,
            value: put.value,
        })?;
    }
}
```

**Better:** Return decoded data from apply method
```rust
// Change apply signature
impl Node {
    pub fn apply_kv_command(&mut self, data: &[u8])
        -> Result<(String, String), Error>
    {
        let cmd = decode::<KvCommand>(data)?;
        if let Some(Cmd::Put(put)) = cmd.cmd {
            self.kv_store.insert(put.key.clone(), put.value.clone());
            Ok((put.key, put.value))
        } else {
            Err("Not a PUT command")
        }
    }
}

// Use in Ready loop
match kv_node.apply_kv_command(&entry.data) {
    Ok((key, value)) => {
        state_tx.send(StateUpdate::KvUpdate { key, value })?;
    }
    Err(e) => { /* ... */ }
}
```

#### 4. Channel Backpressure

**Symptom:** Ready loop slowing down or blocking

**Diagnosis:**
```rust
// Add monitoring
let queue_depth = state_rx.len();
if queue_depth > 50 {
    warn!(logger, "State update queue growing"; "depth" => queue_depth);
}
```

**Fixes:**
1. **Bounded channel with overflow handling:**
   ```rust
   let (state_tx, state_rx) = bounded(100);

   // In Ready loop
   match state_tx.try_send(update) {
       Ok(_) => {}
       Err(TrySendError::Full(_)) => {
           // TUI too slow - drop update (or log warning)
           debug!(logger, "Dropped state update (TUI slow)");
       }
       Err(TrySendError::Disconnected(_)) => {
           // TUI gone - continue Raft loop
       }
   }
   ```

2. **Batch updates:**
   ```rust
   // Instead of sending individual KV updates
   state_tx.send(KvUpdate { key: "foo", value: "bar" })?;
   state_tx.send(KvUpdate { key: "baz", value: "qux" })?;

   // Send batches
   let mut batch = Vec::new();
   for entry in committed_entries {
       if let Ok((key, value)) = apply_entry(entry) {
           batch.push((key, value));
       }
   }
   state_tx.send(StateUpdate::KvBatch(batch))?;
   ```

3. **Coalesce updates:**
   ```rust
   // Only send RaftState update if changed
   let new_state = raft_node.get_state();
   if new_state != last_sent_state {
       state_tx.send(StateUpdate::RaftState(new_state.clone()))?;
       last_sent_state = new_state;
   }
   ```

### Performance Targets

**Ready loop:**
- Process 100+ proposals/sec (3-node cluster)
- <10ms latency per Ready cycle (average)
- <1% CPU usage when idle (just heartbeats)

**TUI:**
- 20+ FPS (50ms frame time)
- <5ms render time (terminal draw)
- <1MB memory for display state

**Network:**
- <10ms message send latency (local TCP)
- <100ms end-to-end commit latency (3 nodes)

**Measure with:**
```rust
use std::time::Instant;

let start = Instant::now();
// ... do work ...
let elapsed = start.elapsed();
debug!(logger, "Ready cycle"; "elapsed_us" => elapsed.as_micros());
```

---

## Testing Patterns

### Unit Testing Helpers

**Test state updates:**
```rust
#[test]
fn test_state_update_application() {
    let (state_tx, state_rx) = unbounded();
    let (cmd_tx, _cmd_rx) = unbounded();
    let (shutdown_tx, _shutdown_rx) = unbounded();

    let mut app = App::new(1, state_rx, cmd_tx, shutdown_tx);

    // Send mock update
    state_tx.send(StateUpdate::KvUpdate {
        key: "foo".into(),
        value: "bar".into(),
    }).unwrap();

    // Drain (simulate TUI loop Step 1)
    while let Ok(update) = app.state_rx.try_recv() {
        app.apply_state_update(update);
    }

    // Verify
    assert_eq!(app.kv_store.get("foo"), Some(&"bar".to_string()));
    assert!(app.system_messages.iter().any(|m| m.contains("foo")));
}
```

**Test command routing:**
```rust
#[test]
fn test_put_command_routing() {
    let (cmd_tx, cmd_rx) = unbounded();
    let (state_tx, state_rx) = unbounded();

    let raft_node = create_test_raft_node();
    let kv_node = Node::new();

    // Spawn Ready loop in thread
    let handle = thread::spawn(move || {
        raft_ready_loop(raft_node, kv_node, cmd_rx, ..., state_tx, ...)
    });

    // Send PUT command
    cmd_tx.send(UserCommand::Put {
        key: "test".into(),
        value: "123".into(),
    }).unwrap();

    // Wait for state update
    let timeout = Duration::from_secs(5);
    let update = state_rx.recv_timeout(timeout).unwrap();

    // Verify
    match update {
        StateUpdate::SystemMessage(msg) => {
            assert!(msg.contains("Proposing"));
        }
        _ => panic!("Expected SystemMessage"),
    }

    // Cleanup
    drop(cmd_tx);  // Close channel
    handle.join().unwrap().unwrap();
}
```

### Integration Testing

**Test 3-node cluster:**
```rust
#[test]
fn test_three_node_consensus() {
    // Setup 3 nodes
    let (node1, node2, node3) = setup_three_nodes();

    // Connect via local transport
    let transport = LocalTransport::new(...);

    // Spawn Ready loops
    let h1 = spawn_ready_loop(node1, transport.clone());
    let h2 = spawn_ready_loop(node2, transport.clone());
    let h3 = spawn_ready_loop(node3, transport.clone());

    // Wait for leader election
    thread::sleep(Duration::from_secs(2));

    // Propose command on leader
    let leader_cmd_tx = find_leader_cmd_tx(&[&h1, &h2, &h3]);
    leader_cmd_tx.send(UserCommand::Put {
        key: "test".into(),
        value: "123".into(),
    }).unwrap();

    // Wait for commit
    thread::sleep(Duration::from_secs(1));

    // Verify all nodes have same state
    for handle in [&h1, &h2, &h3] {
        let state_rx = handle.state_rx();
        // Check KvUpdate received
    }

    // Cleanup
    shutdown_all(&[h1, h2, h3]);
}
```

---

## Debugging Techniques

### Log File Analysis

**Each node writes to `node-{id}.log`**

**Check for election:**
```bash
rg "became.*leader|became.*candidate" node-1.log
```

**Check message flow:**
```bash
rg "Sending from.*to" node-1.log
```

**Check commit latency:**
```bash
rg "Proposed PUT|Applied KV" node-1.log
```

**Common issues:**

1. **No leader elected:**
   - Check: Election timeout firing
   - Check: Vote requests being sent
   - Check: Vote responses received
   - Fix: Ensure messages have correct `from` field

2. **PUT not applying:**
   - Check: "Proposed PUT" in logs
   - Check: Entry appears in committed_entries
   - Check: "Applied KV" after commit
   - Fix: Ensure applied_index being updated

3. **TUI not updating:**
   - Check: State updates being sent (Ready loop)
   - Check: State updates being received (TUI loop)
   - Check: `try_recv()` draining messages
   - Fix: Verify channels connected, no blocking

### Runtime Monitoring

**Add metrics:**
```rust
struct Metrics {
    ready_cycles: AtomicU64,
    entries_applied: AtomicU64,
    messages_sent: AtomicU64,
    state_updates_sent: AtomicU64,
}

// In Ready loop
metrics.ready_cycles.fetch_add(1, Ordering::Relaxed);

// Periodically log
if tick_count % 100 == 0 {
    info!(logger, "Metrics";
          "ready_cycles" => metrics.ready_cycles.load(Ordering::Relaxed),
          "entries_applied" => metrics.entries_applied.load(Ordering::Relaxed));
}
```

**Monitor channel depths:**
```rust
let cmd_depth = cmd_rx.len();
let msg_depth = msg_rx.len();
let state_depth = state_rx.len();

if state_depth > 50 {
    warn!(logger, "State queue growing"; "depth" => state_depth);
}
```

---

This reference provides comprehensive details for understanding and modifying the Raft-TUI integration. Refer to specific sections when working on related tasks.
