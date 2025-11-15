# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**raft-a-tui** is an educational interactive playground for learning the Raft consensus algorithm. It combines:
- **raft-rs** (TiKV's Raft implementation) for distributed consensus
- **ratatui** for terminal-based UI visualization

The project is in early development. The foundational KV store, command parsing, and protobuf infrastructure exist. Raft integration and the TUI are not yet implemented (main.rs is a stub).

## Development Philosophy: Human-in-the-Loop

**CRITICAL:** This is an educational project emphasizing understanding over automation.

**Implementation Approach:**
- **Follow ROADMAP.md:** All implementation must align with the roadmap phases
- **One task at a time:** Complete and test individual tasks before moving forward
- **Human review required:** Wait for explicit approval before proceeding to next task/phase
- **Update ROADMAP.md:** Mark tasks complete (✅), update status, add learnings
- **No auto-completion:** Do not implement entire features without human guidance
- **Ask questions:** When design decisions arise, present options and wait for human choice
- **Keep it modular:** Each component should be understandable and testable in isolation
- **Commit frequently:** After each logical unit of work (one task or subtask)

**When Working on Tasks:**
1. **Read ROADMAP.md** to understand current phase and task
2. **Review decision points** before implementing
3. **Implement the specific task** (not the entire phase)
4. **Test in isolation** with unit/integration tests
5. **Request human review** before marking task complete
6. **Update ROADMAP.md** with status, learnings, and any new decision points
7. **Commit changes** with clear message referencing roadmap task

**Example Workflow:**
```
Human: "Let's start on Phase 1.1 - Storage Layer"
AI: *Reads ROADMAP.md Phase 1.1*
AI: "I see we need to create RaftStorage wrapping MemStorage. Before I start,
     should we use the default MemStorage::new() or configure it? Also, for
     applied_index, should it be part of a snapshot or separate field?"
Human: "Separate field for now, default MemStorage is fine"
AI: *Implements RaftStorage with applied_index field*
AI: *Writes tests*
AI: "Implementation complete. Here's what I did... Ready for review."
Human: "Looks good, mark it complete"
AI: *Updates ROADMAP.md, commits*
```

**What NOT to Do:**
- ❌ Implement entire phases without human involvement
- ❌ Skip decision points without asking
- ❌ Move to next phase before current one is reviewed
- ❌ Make architectural decisions independently
- ❌ Implement features not in roadmap without discussion

## Development Commands

```bash
# Build
cargo build
just build

# Run tests
cargo test
just test

# Run specific test
cargo test test_put_and_get

# Check compilation
cargo check
just check

# Format code
cargo fmt

# Future: Run node (not yet implemented)
cargo run -- --id 1 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
```

## Architecture

### Module Structure

```
src/
├── lib.rs       # Module exports + protobuf inclusion
├── main.rs      # Stub (future TUI entry point)
├── commands.rs  # REPL command parser (PUT, GET, KEYS, STATUS, CAMPAIGN)
├── codec.rs     # Generic protobuf encode/decode helpers
└── node.rs      # Core Node with KV state machine (BTreeMap)
```

### Key Abstractions

**Dual Command Paths:**
- **`UserCommand`** (commands.rs) - Ephemeral REPL actions parsed from user input
  - Long forms: `PUT`, `GET`, `KEYS`, `STATUS`, `CAMPAIGN`
  - Short aliases: `p`, `g`, `k`, `s`, `c`
  - Applied via `Node::apply_user_command()` - immediate, non-replicated

- **`KvCommand`** (protobuf) - Replicated commands for Raft log entries
  - Serialized via `Node::encode_put_command(key, value) -> Vec<u8>`
  - Applied via `Node::apply_kv_command(&bytes)` - replicated, future Raft integration

**Data Flow (Future Raft Integration):**
1. User types `PUT x hello` → parsed to `UserCommand::Put`
2. Submit to Raft: `RawNode::propose(encode_put_command("x", "hello"))`
3. Raft replicates → committed log entry
4. Apply: `node.apply_kv_command(&entry.data)` → mutates BTreeMap

**State Storage:**
- `Node` uses `BTreeMap<String, String>` (not HashMap) for deterministic iteration order
- Critical for consistent snapshots and predictable serialization in Raft
- Keys are always sorted in output

### Protobuf Schema

Located in `proto/kv.proto`:
```protobuf
message KvCommand {
  oneof cmd {
    Put put = 1;
  }
}
```

Built at compile-time via `build.rs` → generates code in `$OUT_DIR/kvraft.rs` → included in lib.rs.

### Testing Strategy

All tests are integration tests in `/tests/` (no unit test modules in src files):
- `node_basic.rs` - Node state machine operations
- `user_command_parse.rs` - Long-form command parsing
- `user_command_aliases.rs` - Short-form aliases + equivalence
- `codec_generic.rs` - Generic protobuf encode/decode
- `protobuf_roundtrip.rs` - Serialization correctness

Pattern: Small focused tests with descriptive names. Run single tests during development for fast feedback.

## Important Implementation Details

**BTreeMap Rationale:**
- Use `BTreeMap` instead of `HashMap` for Node's KV store
- Ensures deterministic iteration order for Raft snapshots
- Enables sorted `KEYS` output and predictable testing

**Command Parser:**
- Case-insensitive matching
- Both long (`PUT foo bar`) and short (`p foo bar`) forms supported
- Returns `Option<UserCommand>` (None for invalid input)

**NodeOutput:**
- Commands return `NodeOutput::Text(String)` or `NodeOutput::None`
- Future: Add variants for errors, Raft status, etc.

**Codec Abstraction:**
- Generic `encode<M: Message>()` and `decode<M: Message>()` in codec.rs
- Hides prost boilerplate from business logic
- Type-safe serialization for any protobuf message

## Raft Integration Guide

### Core raft-rs Types

**RawNode** - Primary Raft interface:
- Created via `RawNode::new(config, storage)`
- Drives consensus through `tick()`, `step()`, `propose()` methods
- Returns state changes via `ready()` when `has_ready()` is true

**Config** - Startup parameters:
- Must include node ID
- Call `validate()` before use
- Key fields: `election_tick`, `heartbeat_tick`, `id`
- Recommended: `heartbeat_tick=3`, `election_tick=10` (must be `election_tick > heartbeat_tick`)

**Storage trait** - Persistent state abstraction:
- `initial_state()` - Returns HardState (term, vote, commit) and ConfState (cluster membership)
- `entries(low, high)` - Retrieves log entries in range
- `term(index)`, `first_index()`, `last_index()` - Log queries
- `snapshot()` - Provides state machine snapshots
- Use `MemStorage` for testing, implement custom Storage for production

**Ready** - Encapsulates state changes requiring action:
- `messages` - Raft messages to send to peers
- `committed_entries` - Log entries ready to apply to state machine
- `entries` - New log entries to append
- `hs` - HardState changes to persist
- `snapshot` - Snapshot to apply (if present)

**LightReady** - Returned from `advance()`:
- Contains remaining messages and committed entries
- Process after handling main Ready state

### The Ready Loop Pattern

The main integration loop has three phases executed on each iteration:

**Phase 1: Input Reception**
```rust
let mut tick_timeout = Duration::from_millis(100);
loop {
    match receiver.recv_timeout(tick_timeout) {
        Ok(RaftMessage(msg)) => raw_node.step(msg)?,
        Ok(Proposal{data, callback_id}) => {
            callbacks.insert(callback_id, callback);
            raw_node.propose(context, data)?;
        }
        Err(Timeout) => raw_node.tick(),
    }

    // Phase 2 & 3 follow...
}
```

**Phase 2: Ready Check**
```rust
if !raw_node.has_ready() {
    continue;
}
let ready = raw_node.ready();
```

**Phase 3: Ready Processing (order matters!)**
```rust
// 1. Apply snapshot if present
if !ready.snapshot().is_empty() {
    storage.apply_snapshot(ready.snapshot())?;
}

// 2. Append new log entries
if !ready.entries().is_empty() {
    storage.append(ready.entries())?;
}

// 3. Persist HardState changes (term, vote, commit)
if let Some(hs) = ready.hs() {
    storage.set_hardstate(hs)?;
}

// 4. Send messages to peers (AFTER persistence!)
for msg in ready.messages {
    transport.send(msg)?;
}

// 5. Apply committed entries to state machine
for entry in ready.committed_entries {
    if entry.data.is_empty() {
        continue; // Empty entry from new leader election
    }
    match entry.get_entry_type() {
        EntryType::EntryNormal => {
            // Decode and apply to state machine
            node.apply_kv_command(&entry.data)?;
            // Find and invoke callback if present
            if let Some(cb) = callbacks.remove(&entry.context) {
                cb.send(response)?;
            }
        }
        EntryType::EntryConfChange | EntryType::EntryConfChangeV2 => {
            // Handle cluster membership changes
        }
    }
    applied_index = entry.index;
}

// 6. Advance to next Ready cycle
let light_ready = raw_node.advance(ready);
// Process light_ready.messages and light_ready.committed_entries similarly
```

### Storage Implementation Requirements

**Critical Guarantees:**
- Persist HardState (term, vote, commit) before sending messages
- Track `applied_index` separately to prevent reapplication after restart
- Use a dummy log entry to preserve the last truncated entry's term

**Pitfall Warning:**
"Although Raft guarantees only persisted committed entries will be applied, it doesn't guarantee commit index is persisted before being applied."
- Risk: Applied index may exceed commit index after restart → panic
- Solution: Persist applied index in state machine snapshots

**Snapshot Strategy:**
- Snapshots contain state machine data + applied index + ConfState
- Triggered manually when log grows too large
- Receiving nodes apply via `storage.apply_snapshot()`

### Network Layer Integration

raft-rs generates messages but doesn't send them. You must:

1. **Serialize messages** from `ready.messages` (already protobuf-compatible)
2. **Send to peers** via your transport (TCP, UDP, crossbeam channels for local testing)
3. **Receive and deserialize** peer messages
4. **Call `raw_node.step(msg)`** for each received message

**Optimization:**
- Batch entries respecting `max_size_per_msg` configuration
- Pipeline messages within `max_inflight_msgs` limit
- For local testing: Use crossbeam channels, one per peer connection

### Configuration Guidelines

**Timing Parameters:**
```rust
let config = Config {
    id: node_id,
    election_tick: 10,      // Election timeout in ticks (1 second at 100ms/tick)
    heartbeat_tick: 3,      // Heartbeat interval in ticks (300ms)
    check_quorum: true,     // Leader checks quorum connectivity
    pre_vote: true,         // Prevent unnecessary elections
    ..Default::default()
};
config.validate()?;
```

**Read Consistency:**
- `read_only_option = ReadOnlySafe` - Linearizable reads (slower, strong consistency)
- `read_only_option = ReadOnlyLeaseBased` - Lease-based reads (faster, may read stale during partitions)

**Tick Frequency:**
- Call `raw_node.tick()` every 100ms (recommended)
- Election fires after ~1 second of no leader heartbeats
- Heartbeats sent every ~300ms

### Common Raft Implementation Pitfalls

**Message Ordering:**
- Regular messages can be sent before persistence completes
- Persisted messages MUST only be sent AFTER writes complete
- Check `ready.persisted_messages` separately from `ready.messages`

**Async I/O:**
- Use `raw_node.advance_append_async()` to offload disk writes to background threads
- Improves latency predictability by not blocking Raft state machine
- Call `on_persist_ready()` after async writes complete

**Election Triggering:**
- Elections happen automatically via tick timeouts
- No explicit "start election" API
- CAMPAIGN command implementation: Force by setting election timeout to 0 or calling `raw_node.campaign()`

**Proposal Context:**
- Embed unique request IDs in `propose(context, data)`
- Maintain callback map: `HashMap<Vec<u8>, Sender<Response>>`
- When committed entry arrives, decode context to find callback

**Multi-Raft Pattern:**
- Production systems run multiple Raft groups per node
- Each "Region" (data shard) has its own RawNode instance
- Share transport layer, separate storage per group

**Testing Strategy:**
- Start with `MemStorage` and in-memory channels
- Example: `raft-rs/examples/five_mem_node` demonstrates complete working cluster
- Test election scenarios: kill leader, network partition, concurrent proposals

## TUI Integration with Ratatui

### Real-Time Event Loop Architecture

The TUI must run concurrently with the Raft Ready loop, providing instant visual feedback. Use a **message-passing architecture** where the Raft thread sends state updates to the TUI thread.

**Dual-Loop Pattern:**
```rust
// Main thread: TUI event loop
let (state_tx, state_rx) = crossbeam_channel::unbounded();
let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();

// Spawn Raft thread
thread::spawn(move || {
    raft_ready_loop(cmd_rx, state_tx);  // Receives commands, sends state updates
});

// TUI loop
let mut app = App::new(state_rx, cmd_tx);
app.run()?;
```

### Application State Structure

**Separate concerns:** TUI state vs. Raft state
```rust
struct App {
    // User interaction state
    input: String,
    input_mode: InputMode,  // Normal vs. Editing
    cursor_position: usize,

    // Raft state (updated via channel)
    raft_state: RaftState,
    kv_store: BTreeMap<String, String>,

    // Display state
    command_history: Vec<String>,
    log_entries: VecDeque<LogEntry>,  // Ring buffer for recent events
    system_messages: VecDeque<String>,

    // Communication channels
    state_rx: Receiver<StateUpdate>,
    cmd_tx: Sender<UserCommand>,
}

struct RaftState {
    node_id: u64,
    term: u64,
    role: Role,  // Leader, Follower, Candidate
    leader_id: Option<u64>,
    commit_index: u64,
    applied_index: u64,
    peers: Vec<u64>,
}

enum StateUpdate {
    RaftState(RaftState),
    KvUpdate { key: String, value: String },
    LogEntry(LogEntry),
    SystemMessage(String),
}
```

### Main TUI Loop Pattern

**Non-blocking event handling with periodic updates:**
```rust
impl App {
    fn run(&mut self) -> io::Result<()> {
        let mut terminal = setup_terminal()?;

        loop {
            // 1. Process all pending state updates from Raft (non-blocking)
            while let Ok(update) = self.state_rx.try_recv() {
                self.apply_state_update(update);
            }

            // 2. Render current state
            terminal.draw(|frame| self.draw(frame))?;

            // 3. Handle user input with timeout (for responsive updates)
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match self.handle_key_event(key) {
                        KeyResult::Quit => break,
                        KeyResult::Command(cmd) => {
                            self.cmd_tx.send(cmd)?;
                        }
                        KeyResult::Continue => {}
                    }
                }
            }
        }

        restore_terminal(terminal)?;
        Ok(())
    }
}
```

**Why `event::poll()` with timeout?**
- Allows checking for Raft state updates even without user input
- Ensures UI refreshes at least every 50ms (20 FPS)
- Critical for real-time visualization of elections, heartbeats, log replication

### Layout Structure (Multi-Pane TUI)

**Four-pane design matching README goals:**
```rust
fn draw(&self, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),     // Title bar (node info)
            Constraint::Min(10),       // Main content (split horizontally)
            Constraint::Length(3),     // Command input
        ])
        .split(frame.area());

    // Title: Node ID, Term, Role, Leader
    self.draw_title_bar(frame, chunks[0]);

    // Split main area horizontally
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),  // Left: Raft state + logs
            Constraint::Percentage(50),  // Right: KV store + command history
        ])
        .split(chunks[1]);

    // Left pane (split vertically)
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),  // Raft state
            Constraint::Percentage(60),  // System logs (color-coded)
        ])
        .split(main_chunks[0]);

    self.draw_raft_state(frame, left_chunks[0]);
    self.draw_system_logs(frame, left_chunks[1]);

    // Right pane (split vertically)
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),  // KV store
            Constraint::Percentage(50),  // Command history
        ])
        .split(main_chunks[1]);

    self.draw_kv_store(frame, right_chunks[0]);
    self.draw_command_history(frame, right_chunks[1]);

    // Command input at bottom
    self.draw_command_input(frame, chunks[2]);
}
```

### Color-Coded Event Display

**Visual feedback for different event types:**
```rust
fn draw_system_logs(&self, frame: &mut Frame, area: Rect) {
    let logs: Vec<ListItem> = self.system_messages
        .iter()
        .map(|msg| {
            let style = match msg.event_type {
                EventType::Election => Style::default().fg(Color::Yellow),
                EventType::LogReplication => Style::default().fg(Color::Green),
                EventType::HeartBeat => Style::default().fg(Color::Gray),
                EventType::StateChange => Style::default().fg(Color::Cyan),
                EventType::Error => Style::default().fg(Color::Red),
            };
            ListItem::new(msg.text.as_str()).style(style)
        })
        .collect();

    let list = List::new(logs)
        .block(Block::default()
            .borders(Borders::ALL)
            .title("System Events"));

    frame.render_widget(list, area);
}
```

### Input Handling with Mode Switching

**Two modes:** Normal (commands like 'q', 'i') and Editing (typing commands)
```rust
fn handle_key_event(&mut self, key: KeyEvent) -> KeyResult {
    match self.input_mode {
        InputMode::Normal => match key.code {
            KeyCode::Char('q') => KeyResult::Quit,
            KeyCode::Char('i') => {
                self.input_mode = InputMode::Editing;
                KeyResult::Continue
            }
            _ => KeyResult::Continue,
        },
        InputMode::Editing => match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                KeyResult::Continue
            }
            KeyCode::Enter => {
                let input = self.input.drain(..).collect::<String>();
                if let Some(cmd) = parse_command(&input) {
                    self.command_history.push_front(input);
                    KeyResult::Command(cmd)
                } else {
                    self.system_messages.push_front(
                        format!("Invalid command: {}", input)
                    );
                    KeyResult::Continue
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_position, c);
                self.cursor_position += 1;
                KeyResult::Continue
            }
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.input.remove(self.cursor_position - 1);
                    self.cursor_position -= 1;
                }
                KeyResult::Continue
            }
            _ => KeyResult::Continue,
        },
    }
}
```

### Integration with Raft Ready Loop

**Raft thread sends updates to TUI:**
```rust
fn raft_ready_loop(
    cmd_rx: Receiver<UserCommand>,
    state_tx: Sender<StateUpdate>,
) {
    let mut raw_node = setup_raft_node();
    let mut tick_timer = Instant::now();

    loop {
        // Handle incoming commands with short timeout
        match cmd_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(UserCommand::Put { key, value }) => {
                let data = Node::encode_put_command(&key, &value);
                raw_node.propose(vec![], data)?;
            }
            Ok(UserCommand::Campaign) => {
                raw_node.campaign()?;
                state_tx.send(StateUpdate::SystemMessage(
                    "Starting election campaign".to_string()
                ))?;
            }
            _ => {}
        }

        // Tick every 100ms
        if tick_timer.elapsed() >= Duration::from_millis(100) {
            raw_node.tick();
            tick_timer = Instant::now();
        }

        // Process Ready state
        if raw_node.has_ready() {
            let ready = raw_node.ready();

            // Send Raft state update to TUI
            state_tx.send(StateUpdate::RaftState(RaftState {
                term: raw_node.raft.term,
                role: raw_node.raft.state,
                leader_id: raw_node.raft.leader_id,
                commit_index: raw_node.raft.raft_log.committed,
                applied_index: raw_node.raft.raft_log.applied,
                // ...
            }))?;

            // Apply committed entries
            for entry in ready.committed_entries {
                if !entry.data.is_empty() {
                    // Apply to state machine
                    let result = node.apply_kv_command(&entry.data);

                    // Send KV update to TUI
                    state_tx.send(StateUpdate::KvUpdate {
                        key: extracted_key,
                        value: extracted_value,
                    })?;

                    // Send log entry to TUI
                    state_tx.send(StateUpdate::LogEntry(LogEntry {
                        index: entry.index,
                        term: entry.term,
                        data: format!("Applied: {} = {}", key, value),
                    }))?;
                }
            }

            // Send messages, persist state, etc.
            // ...

            raw_node.advance(ready);
        }
    }
}
```

### Terminal Setup and Cleanup

**Always restore terminal state on exit:**
```rust
fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal(
    mut terminal: Terminal<CrosstermBackend<io::Stdout>>
) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
```

### Key Design Principles for Real-Time Updates

1. **Non-blocking state reads:** Use `try_recv()` to drain all pending updates before rendering
2. **Frequent render cycles:** 50ms poll timeout = 20 FPS minimum update rate
3. **Ring buffers for logs:** Use `VecDeque` with max size to prevent unbounded memory growth
4. **Separate state from rendering:** App state mutations happen in dedicated methods, draw() is pure
5. **Event batching:** Process all channel messages before rendering to avoid redundant draws
6. **Graceful degradation:** If channel is full, drop oldest messages rather than blocking Raft thread

### Testing the TUI

**Test without Raft first:**
```rust
// Mock state updates for TUI testing
#[test]
fn test_tui_rendering() {
    let (state_tx, state_rx) = crossbeam_channel::unbounded();
    let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();

    // Send mock state updates
    state_tx.send(StateUpdate::RaftState(mock_raft_state()))?;
    state_tx.send(StateUpdate::KvUpdate {
        key: "test".into(),
        value: "123".into()
    })?;

    let mut app = App::new(state_rx, cmd_tx);

    // Verify state updates are applied
    while let Ok(update) = app.state_rx.try_recv() {
        app.apply_state_update(update);
    }

    assert_eq!(app.kv_store.get("test"), Some(&"123".to_string()));
}
```

## Implementation Roadmap

**See ROADMAP.md for detailed implementation plan.**

The roadmap is organized into phases with specific tasks, decision points, and human review checkpoints:

**Phase 1: Raft Core Integration** (Get consensus working with 3 nodes)
- Storage layer (MemStorage wrapper with applied_index tracking)
- Network layer (local transport using crossbeam channels)
- Raft node wrapper (RawNode initialization and configuration)
- Raft Ready loop (tick → ready → process → advance)
- CLI integration and 3-node testing

**Phase 2: TUI Visualization** (Interactive real-time display)
- TUI state management (App struct with channels)
- Multi-pane rendering (4-pane layout with color coding)
- Event loop (50ms poll timeout for 20 FPS updates)
- Input handling (Normal/Editing modes)
- Integration with Raft thread

**Phase 3: Polish & Advanced Features** (Production-ready)
- Enhanced commands (STATUS, CAMPAIGN, DELETE, SNAPSHOT)
- Better visualizations (sparklines, connectivity status)
- Network partition testing (packet loss simulation)
- Persistent storage (disk-backed, crash recovery)
- Multi-machine deployment (TCP transport)

**Remember:** Follow the human-in-the-loop workflow. Read ROADMAP.md before starting any task, implement one task at a time, request review, update roadmap, commit.
