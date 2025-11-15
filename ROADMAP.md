# Raft-a-TUI Implementation Roadmap

This roadmap tracks the implementation progress from current state (foundation components) to a fully functional interactive Raft visualization TUI.

## Current State

**Completed (Foundation):**
- ✅ KV state machine (`src/node.rs`) with BTreeMap storage
- ✅ Command parser (`src/commands.rs`) with long/short forms
- ✅ Protobuf codec (`src/codec.rs`) for serialization
- ✅ Protobuf schema (`proto/kv.proto`) for KvCommand
- ✅ Comprehensive integration tests for all foundation components
- ✅ Build infrastructure (build.rs, Cargo.toml with all dependencies)

## Implementation Philosophy

**Human-in-the-Loop Development:**
- Each phase requires human review and decision-making before proceeding
- AI assists with implementation but doesn't auto-complete entire features
- Focus on understanding over speed - this is an educational project
- Keep implementations modular, clear, and well-tested
- Commit after each logical unit of work

**Incremental Progress:**
- Implement one component at a time
- Test in isolation before integration
- Update this roadmap as tasks complete
- Don't move to next phase until current phase is understood

---

## Phase 1: Raft Core Integration

**Goal:** Get Raft consensus working with 3 nodes (no TUI yet - logs to stdout/tests)

### 1.1 Storage Layer
**Status:** ✅ Complete
**File:** `src/storage.rs`

**Tasks:**
- [x] Create `RaftStorage` struct wrapping `MemStorage`
- [x] Add `applied_index` tracking (critical for restart safety)
- [x] Implement `Storage` trait by delegating to `MemStorage`
- [x] Add helper methods: `applied_index()`, `set_applied_index()`
- [x] Write unit tests for applied_index persistence logic

**Decision Points:**
- ✅ Use MemStorage directly for HardState persistence
- ✅ No custom snapshot logic yet (delegate to MemStorage)
- ✅ Use Arc<Mutex<u64>> for applied_index (thread-safe, shareable)

**Implementation Notes:**
- Used Arc<Mutex<u64>> for applied_index to enable sharing across clones
- Implemented Clone trait to share both inner MemStorage and applied_index
- Added 7 tests covering: get/set, clone sharing, ConfState initialization, Storage trait delegation
- All tests pass ✅

---

### 1.2 Network Layer (Local Transport)
**Status:** 🔲 Not Started
**File:** `src/network.rs`

**Tasks:**
- [ ] Define `Transport` trait with `send(to: u64, msg: Message)` method
- [ ] Implement `LocalTransport` using `HashMap<u64, Sender<Message>>`
- [ ] Add peer discovery from CLI `--peers` argument
- [ ] Message serialization/deserialization (already protobuf-compatible)
- [ ] Write tests for message routing between 3 mock nodes

**Decision Points:**
- Should we add message dropping/delays for partition testing? (Later phase)
- Error handling strategy for send failures? (Log and continue vs. panic)

**Human Review Required:**
- Review transport abstraction - is it flexible enough for future TCP?
- Validate message routing logic with diagrams/tests

---

### 1.3 Raft Node Wrapper
**Status:** 🔲 Not Started
**File:** `src/raft_node.rs`

**Tasks:**
- [ ] Create `RaftNode` struct containing `RawNode`, `Storage`, callbacks
- [ ] Implement `new(id, peers, storage)` constructor with Config setup
- [ ] Add `propose_command(key, value) -> callback_id` method
- [ ] Implement callback tracking: `HashMap<Vec<u8>, Sender<Response>>`
- [ ] Add helper methods: `get_state() -> RaftState`, `is_leader() -> bool`
- [ ] Write tests for RaftNode initialization and basic operations

**Decision Points:**
- How to generate unique callback IDs? (UUID, counter, hash?)
- Should `propose_command` block or return immediately? (Return immediately)
- Configuration values: Use recommended defaults or allow customization? (Defaults first)

**Human Review Required:**
- Review RawNode initialization - are config values reasonable?
- Verify callback mechanism design before implementing Ready loop

---

### 1.4 Raft Ready Loop
**Status:** 🔲 Not Started
**File:** `src/raft_loop.rs`

**Tasks:**
- [ ] Implement `raft_ready_loop(node, cmd_rx, state_tx, transport)` function
- [ ] Phase 1: Input reception (commands, network messages, tick timer)
- [ ] Phase 2: Ready state check with `has_ready()`
- [ ] Phase 3: Ready processing (persist, send, apply, advance)
- [ ] Integrate `Node::apply_kv_command()` for committed entries
- [ ] Send state updates via `state_tx` channel (for future TUI)
- [ ] Add comprehensive logging of all Raft events

**Decision Points:**
- Tick frequency: 100ms as recommended or different? (100ms)
- How to handle `step()` errors? (Log and continue)
- Should we support graceful shutdown signal? (Yes - use channel)

**Human Review Required:**
- Review Ready processing order - matches documentation?
- Verify committed entry application doesn't miss edge cases
- Test with println debugging before adding TUI

---

### 1.5 CLI Integration & 3-Node Test
**Status:** 🔲 Not Started
**Files:** `src/main.rs`, `tests/three_node_cluster.rs`

**Tasks:**
- [ ] Add clap argument parsing: `--id <u64>`, `--peers <peer_list>`
- [ ] Parse peer format: `1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003`
- [ ] Wire up: Storage → RaftNode → raft_ready_loop → Transport
- [ ] Add basic stdin command reading (temporary - replaced by TUI)
- [ ] Write integration test spawning 3 Raft instances
- [ ] Test: Leader election, log replication, PUT command commit

**Decision Points:**
- Should main.rs have TUI stub or just stdin? (Just stdin for now)
- Logging strategy: env_logger, tracing, or custom? (slog - already in deps)

**Human Review Required:**
- Manually test 3-node cluster with cargo run instances
- Verify elections work, logs replicate correctly
- Check STATUS command shows actual Raft state

**Milestone:** Phase 1 complete when you can run 3 terminals, submit PUT commands, see them replicate via logs.

---

## Phase 2: TUI Visualization

**Goal:** Replace stdin with interactive TUI showing real-time Raft state

### 2.1 TUI State & Updates
**Status:** 🔲 Not Started
**File:** `src/tui/app.rs`

**Tasks:**
- [ ] Create `App` struct with input state, Raft state, display state, channels
- [ ] Define `StateUpdate` enum (RaftState, KvUpdate, LogEntry, SystemMessage)
- [ ] Implement `apply_state_update(&mut self, update: StateUpdate)`
- [ ] Add state query methods for rendering
- [ ] Write tests for state update application logic

**Decision Points:**
- Log buffer size? (VecDeque with 100-1000 entries)
- Should we persist command history across restarts? (No - keep simple)

**Human Review Required:**
- Review state structure - does it capture all needed TUI data?
- Verify state updates handle all Raft events

---

### 2.2 TUI Rendering
**Status:** 🔲 Not Started
**File:** `src/tui/ui.rs`

**Tasks:**
- [ ] Implement `draw(app: &App, frame: &mut Frame)` main render function
- [ ] Create 4-pane layout (title, Raft state/logs, KV/history, input)
- [ ] Implement `draw_title_bar()` - node ID, term, role, leader
- [ ] Implement `draw_raft_state()` - detailed Raft info
- [ ] Implement `draw_system_logs()` - color-coded event list
- [ ] Implement `draw_kv_store()` - sorted key-value pairs
- [ ] Implement `draw_command_history()` - recent commands
- [ ] Implement `draw_command_input()` - current input with cursor

**Decision Points:**
- Color scheme for events? (Yellow=election, Green=replication, etc.)
- Should KV store show values or just keys for large values? (Truncate values)

**Human Review Required:**
- Does layout work well at different terminal sizes?
- Are colors accessible and distinguishable?

---

### 2.3 TUI Event Loop
**Status:** 🔲 Not Started
**File:** `src/tui/mod.rs`

**Tasks:**
- [ ] Implement `run(app: &mut App)` main TUI loop
- [ ] Terminal setup: raw mode, alternate screen
- [ ] Event handling: `event::poll(50ms)` with timeout
- [ ] Non-blocking state update drain: `while try_recv()`
- [ ] Keyboard input handling with mode switching
- [ ] Terminal cleanup on exit (even on panic)
- [ ] Graceful shutdown signal handling

**Decision Points:**
- Poll timeout: 50ms (20 FPS) or different? (Start with 50ms)
- Panic handler to restore terminal? (Yes - use Drop or panic hook)

**Human Review Required:**
- Test that terminal always restores correctly
- Verify no input lag or UI stuttering

---

### 2.4 Input Handling & Mode Switching
**Status:** 🔲 Not Started
**File:** `src/tui/input.rs`

**Tasks:**
- [ ] Define `InputMode` enum (Normal, Editing)
- [ ] Implement `handle_key_event(app: &mut App, key: KeyEvent) -> KeyResult`
- [ ] Normal mode: 'q' quit, 'i' edit, shortcuts for commands
- [ ] Editing mode: text input, cursor movement, backspace, enter to submit
- [ ] Parse input on Enter and send to Raft thread
- [ ] Add input validation and error messages

**Decision Points:**
- Support arrow keys for cursor movement? (Yes)
- Command history navigation (up/down arrows)? (Nice to have - Phase 3)

**Human Review Required:**
- Is input handling intuitive and responsive?
- Are there any missing common keyboard shortcuts?

---

### 2.5 Integration: TUI + Raft
**Status:** 🔲 Not Started
**File:** `src/main.rs`

**Tasks:**
- [ ] Create bidirectional channels: `(cmd_tx, cmd_rx)`, `(state_tx, state_rx)`
- [ ] Spawn Raft thread: `thread::spawn(|| raft_ready_loop(...))`
- [ ] Run TUI on main thread: `app.run()?`
- [ ] Wire state updates from Raft loop to TUI
- [ ] Wire commands from TUI to Raft loop
- [ ] Test with 3 terminal instances

**Decision Points:**
- Should Raft thread panic bring down TUI? (Yes - use thread::JoinHandle)
- Channel buffer sizes? (Unbounded for now)

**Human Review Required:**
- Manually test 3-node cluster with TUI
- Verify all events show up in real-time
- Check that commands work correctly

**Milestone:** Phase 2 complete when you can run 3 TUIs, watch elections, see log replication, submit commands interactively.

---

## Phase 3: Polish & Advanced Features

**Goal:** Production-ready features and enhanced UX

### 3.1 Enhanced Commands
**Status:** 🔲 Not Started

**Tasks:**
- [ ] STATUS: Show actual Raft metrics (not placeholder)
- [ ] CAMPAIGN: Trigger election via `raw_node.campaign()`
- [ ] DELETE: Add to protobuf, implement in Node
- [ ] SNAPSHOT: Manual snapshot trigger for testing
- [ ] Add command-line flag help text

---

### 3.2 Better Visualizations
**Status:** 🔲 Not Started

**Tasks:**
- [ ] Add sparkline for commit rate over time
- [ ] Show peer connectivity status
- [ ] Highlight current leader in peer list
- [ ] Add log entry detail view (expandable)
- [ ] Command history navigation with arrow keys

---

### 3.3 Network Partition Testing
**Status:** 🔲 Not Started

**Tasks:**
- [ ] Add `--drop-rate` flag to simulate packet loss
- [ ] Add `--partition` flag to isolate nodes
- [ ] Visualize network connectivity in TUI
- [ ] Test split-brain scenarios

---

### 3.4 Persistent Storage
**Status:** 🔲 Not Started

**Tasks:**
- [ ] Implement disk-backed Storage (replace MemStorage)
- [ ] Add snapshot serialization with bincode
- [ ] Test crash recovery scenarios
- [ ] Add `--data-dir` flag for storage path

---

### 3.5 Multi-Machine Deployment
**Status:** 🔲 Not Started

**Tasks:**
- [ ] Replace LocalTransport with TCP transport
- [ ] Add peer authentication (optional)
- [ ] Update README with multi-machine setup guide
- [ ] Test across multiple physical machines

---

## Maintenance

**Keeping This Roadmap Updated:**
- Mark tasks complete with ✅ as they're finished
- Update status: 🔲 Not Started, 🔄 In Progress, ✅ Complete
- Add notes/learnings in "Decision Points" as you discover them
- Create new sections if design changes require it
- Commit roadmap updates with related code changes

**Related Documentation:**
- `CLAUDE.md` - Implementation guide for AI assistance
- `README.md` - User-facing quickstart and command reference
- Inline code comments - Document "why" decisions

---

## Notes & Learnings

**Add notes here as you implement:**
- What worked well?
- What was harder than expected?
- What would you do differently?
- Common pitfalls to warn future contributors about

---

Last Updated: 2025-11-15
