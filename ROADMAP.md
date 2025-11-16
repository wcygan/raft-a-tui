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

## ✅ Phase 1.6 - Crash Recovery (COMPLETE)

**Status:** ✅ Complete (2 commits: adced5b, b7a6cbc)
**Goal:** Fix node crash/restart panic and enable production-ready crash recovery

**Problem Solved:**
- Nodes no longer panic with `to_commit X is out of range [last_index 0]`
- New nodes can join existing clusters by receiving snapshots
- Crashed nodes recover state from disk on restart
- Applied comprehensive crash recovery testing (6 new tests)

**Implementation Summary:**
Implemented BOTH snapshot handling (prevents panic) AND persistent storage (enables crash recovery):

### 1.6.1 Snapshot Handling in Ready Loop ✅
**Status:** ✅ Complete (commit: adced5b)
**File:** `src/raft_loop.rs`
**Priority:** CRITICAL - Prevents node startup panic

**Tasks:**
- [x] Add snapshot handling in Ready processing (BEFORE entries append)
- [x] Extract snapshot data and restore state machine via `Node::restore_from_snapshot()`
- [x] Update applied_index from snapshot metadata
- [x] Add comprehensive logging for snapshot application
- [x] Test snapshot reception with mock RawNode
- [x] Test state machine restoration from snapshot bytes

**Implementation Pattern:**
```rust
// In Ready processing, FIRST check for snapshot:
if !ready.snapshot().is_empty() {
    let snapshot = ready.snapshot();
    storage.apply_snapshot(snapshot.clone())?;

    // Restore state machine from snapshot data
    let snapshot_data = snapshot.get_data();
    node.restore_from_snapshot(snapshot_data)?;

    // Update applied index
    applied_index = snapshot.get_metadata().index;
    storage.set_applied_index(applied_index)?;
}
```

**Decision Points:**
- Snapshot serialization format? (bincode for now - simple, fast)
- Should Node::restore_from_snapshot() clear existing state? (Yes - snapshot is authoritative)
- How to encode BTreeMap in snapshot? (bincode serialize directly)

---

### 1.6.2 Node Snapshot Methods ✅
**Status:** ✅ Complete (commit: adced5b)
**File:** `src/node.rs`
**Priority:** CRITICAL - Required for snapshot handling

**Tasks:**
- [x] Add `Node::create_snapshot() -> Vec<u8>` - serializes BTreeMap to bytes
- [x] Add `Node::restore_from_snapshot(data: &[u8]) -> Result<()>` - deserializes and replaces state
- [x] Add bincode dependency to Cargo.toml
- [x] Write tests for snapshot roundtrip (create → restore → verify state)
- [x] Test error handling for malformed snapshot data (9 basic + 14 edge case tests)

**Decision Points:**
- Should snapshot include metadata (version, checksum)? (Not yet - keep simple)
- Handle backwards compatibility? (Not yet - Phase 3 concern)

---

### 1.6.3 Persistent Storage Implementation ✅
**Status:** ✅ Complete (commit: adced5b)
**File:** `src/disk_storage.rs` (new, 424 lines)
**Priority:** HIGH - Enables crash recovery after snapshot handling works

**Tasks:**
- [x] Create `DiskStorage` struct wrapping sled database
- [x] Implement `Storage` trait for DiskStorage
- [x] Persist HardState (term, vote, commit) on every change
- [x] Persist log entries with append operations
- [x] Persist snapshots to disk
- [x] Persist applied_index separately
- [x] Add recovery logic: `DiskStorage::open(path) -> Result<Self>`
- [x] Add CLI flag: `--data-dir <path>` (default: `./data/node-{id}`)
- [x] Write tests for crash recovery scenarios (20 comprehensive tests)

**Storage Layout:**
```
data/node-1/
  ├── hardstate     # Current term, vote, commit (atomic writes)
  ├── entries/      # Log entries (indexed by log index)
  ├── snapshots/    # State machine snapshots (indexed by index)
  └── meta          # applied_index, cluster config
```

**Decision Points:**
- Storage backend? (sled - pure Rust, embedded, crash-safe)
- Snapshot compaction trigger? (Manual via SNAPSHOT command first, auto later)
- Log compaction strategy? (Keep all entries initially, add compaction in Phase 3)

---

### 1.6.4 Integration & Testing ✅
**Status:** ✅ Complete (commit: b7a6cbc)
**Files:** `src/main.rs`, `src/cli.rs`, `src/storage.rs`, `tests/crash_recovery_e2e.rs`
**Priority:** HIGH - Validates crash recovery works end-to-end

**Tasks:**
- [x] Add `--data-dir` CLI argument (optional, defaults to `./data/node-{id}`)
- [x] Wire DiskStorage into main.rs (refactored RaftStorage to enum: Memory | Disk)
- [x] Persistent by default, `:memory:` opt-in for testing
- [x] Test: Single node crash/restart with state recovery
- [x] Test: HardState persistence (term, vote, commit)
- [x] Test: Log entries persist across crashes
- [x] Test: applied_index tracking survives crashes
- [x] Test: Multiple crash/restart cycles
- [x] Test: In-memory mode doesn't persist (contrast test)

**Human Review Status:**
- ⏳ Manual testing pending (requires multi-node cluster setup)

**Implementation Outcome:**
- ✅ 122 tests passing (33 new crash recovery tests)
- ✅ Nodes can recover from crashes without data loss
- ✅ New nodes can join clusters via snapshot transfer
- ✅ Persistent storage with sled (embedded database)
- ✅ Clean separation of Memory vs. Disk storage
- ✅ Production-ready crash safety

**Key Learnings:**
- RaftStorage refactored to enum for clean Memory/Disk abstraction
- DiskStorage auto-initializes with ConfState on first use
- Snapshots use bincode for efficient BTreeMap serialization
- Applied index must be tracked separately from commit index
- HardState + log entries + snapshots all must persist for full recovery

---

## Phase 1: Raft Core Integration ✅

**Goal:** Get Raft consensus working with 3 nodes (no TUI yet - logs to files)
**Status:** ✅ Complete (Phases 1.1-1.5)

**Completed Components:**
- ✅ 1.1 Storage Layer (`src/storage.rs`) - RaftStorage wrapping MemStorage with applied_index tracking
- ✅ 1.2 Network Layer (`src/network.rs`) - LocalTransport with crossbeam channels for testing
- ✅ 1.3 Raft Node Wrapper (`src/raft_node.rs`) - RaftNode wrapping RawNode with callback tracking
- ✅ 1.4 Raft Ready Loop (`src/raft_loop.rs`) - 3-phase Ready loop with state updates
- ✅ 1.5 CLI Integration (`src/main.rs`, `src/cli.rs`, `src/tcp_transport.rs`) - TCP transport, clap CLI, 3-node tests

**Key Implementation Details:**
- Storage: Arc<Mutex<u64>> for applied_index, 7 storage tests
- Network: Transport trait with LocalTransport + TcpTransport implementations, retry logic, message framing
- RaftNode: UUID-based callback tracking, RaftState exposure, 11 unit tests
- Ready Loop: 3-phase pattern (input → ready → process), handles empty entries, 14 unit tests
- CLI: Clap parsing, TCP transport with prost 0.11 compatibility, file logging, 74 total tests

**Known Limitation (Addressed in Phase 1.6):**
- ⚠️ **Missing snapshot handling** - Nodes panic when joining existing cluster or restarting after downtime
- Using MemStorage only - No crash recovery yet

---

## Phase 2: TUI Visualization (DEPRIORITIZED)

**Goal:** Replace stdin with interactive TUI showing real-time Raft state
**Status:** 🔲 Deferred until Phase 1.6 complete

> **Note:** Phase 2 is postponed until crash recovery (Phase 1.6) is complete. The current panic bug makes the system unusable for anything beyond basic testing. Snapshot handling and persistent storage are prerequisites for a stable TUI experience.

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

## Phase 3: Polish & Advanced Features (DEPRIORITIZED)

**Goal:** Production-ready features and enhanced UX
**Status:** 🔲 Deferred until Phase 1.6 and Phase 2 complete

> **Note:** Phase 3.4 (Persistent Storage) has been pulled forward into Phase 1.6 due to critical crash recovery requirements.

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
**Status:** ⏫ Moved to Phase 1.6.3

> **This section has been promoted to Phase 1.6.3** due to critical crash recovery requirements. See Phase 1.6 for full implementation details.

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

### Phase 1.4 Learnings (Raft Ready Loop)

**What worked well:**
- Following the raft-rs documentation's Ready loop pattern exactly prevented bugs
- Using crossbeam's `select!` macro made multi-channel listening clean and straightforward
- Comprehensive logging from the start made debugging much easier
- Writing unit tests before integration tests helped catch API mismatches early
- Generic `impl Transport` parameter provides flexibility for both testing and production

**What was harder than expected:**
- Understanding raft-rs API differences between fields and methods (e.g., `ready.messages()` vs `ready.messages`)
- RaftStorage needed write methods added (`append`, `set_hardstate`, etc.) - not initially implemented in Phase 1.1
- LightReady has different API than Ready - need to check for both committed entries and messages
- Empty log entries can occur during leader election - must check `entry.data.is_empty()`
- `set_hardstate()` returns `()` not `Result` - different from other storage operations

**Common pitfalls to avoid:**
- ⚠️ **Order matters in Ready processing**: Must follow snapshot → entries → hardstate → messages → apply → advance
- ⚠️ **Don't forget LightReady**: After `advance()`, must process `light_rd.messages()` and `light_rd.take_committed_entries()`
- ⚠️ **Empty entries are valid**: Leader election creates empty entries - skip them in apply phase
- ⚠️ **Callbacks must be invoked**: When committed entries arrive, find and invoke callback via `take_callback()`
- ⚠️ **Applied index tracking is critical**: Must call `set_applied_index()` to prevent reapplication on restart
- ⚠️ **Error resilience**: step(), apply, and transport errors should be logged and continued, not abort the loop
- ⚠️ **ConfChange entries**: Must handle EntryType::EntryConfChange and EntryConfChangeV2 separately from Normal entries

**What would we do differently:**
- Consider adding write methods to RaftStorage in Phase 1.1 to avoid retrofitting
- Could extract committed entry application into a helper function (it's duplicated for Ready and LightReady)
- Might want to return applied key/value from `Node::apply_kv_command()` to avoid double-decoding

**Enhanced Test Coverage (Post-Review):**
- Added 5 additional tests to verify critical paths:
  1. Single-node commit flow (propose → commit → apply → StateUpdate emission)
  2. Multiple commands producing expected state updates
  3. Transport failure resilience (loop continues despite send errors)
  4. Malformed entry handling structure (defensive programming)
  5. Read-only commands bypass Raft (GET/KEYS/STATUS don't create log entries)
- Tests are timing-aware (single-node leader election is timing-dependent)
- Mock FailingTransport implementation tests error resilience
- Total test count: 14 raft_loop tests, 59 tests overall

---

### Phase 1.5 Learnings (CLI Integration & TCP Transport)

**What worked well:**
- TCP transport implementation with background threads cleanly separates concerns
- Generic `read_message<R: Read>` and `write_message<W: Write>` made testing much easier
- Clap's derive macros made CLI parsing trivial and type-safe
- Non-blocking crossterm event loop with 50ms timeout provided responsive UI
- Length-prefixed message framing (4-byte u32 + protobuf) is simple and robust
- Retry logic (3 attempts, 10ms delay) handles transient connection failures well
- Comprehensive error types (TransportError enum) made debugging straightforward

**What was harder than expected:**
- **Prost version mismatch**: raft-rs uses prost 0.11, project uses prost 0.14
  - Both versions can't coexist via traits without explicit disambiguation
  - Solution: Import prost 0.11 as `prost_011` package and use explicit trait imports
  - Required: `use prost_011::Message as ProstMessage011;` before calling `.encode()`/`.merge()`
- **Logging completely broken**: raft's default logger spammed thousands of debug messages to stdout
  - Made interactive terminal completely unusable
  - Solution: Remove "default-logger" feature, use slog file logging, change `println!` to `eprintln!`
  - **Critical UX lesson**: Always verify where logs go in interactive applications
- Borrow checker issues when sharing peers_map between transport and main
  - Solution: Dereference early: `let my_addr = *peers_map.get(&args.id)...`
- Crossterm's raw mode requires explicit cleanup on exit to avoid broken terminal state

**Common pitfalls to avoid:**
- ⚠️ **Prost trait ambiguity**: When using multiple prost versions, ALWAYS use explicit trait imports
- ⚠️ **Terminal state restoration**: MUST call `disable_raw_mode()` even on error paths
- ⚠️ **Logging strategy**: Interactive TUIs require file logging, not stdout/stderr
- ⚠️ **Channel buffer sizes**: Unbounded channels can cause memory issues under load
- ⚠️ **TCP connection failures**: Don't panic on failed connections - peers may not be started yet
- ⚠️ **Message size validation**: Always enforce max message size to prevent DoS
- ⚠️ **Thread naming**: Use `thread::Builder::new().name()` for better debugging
- ⚠️ **Error handling in threads**: Background threads need logging since they can't return errors

**What would we do differently:**
- Could use `tokio` for async I/O instead of blocking threads (more scalable)
- Might want to batch messages to reduce syscalls (raft supports `max_size_per_msg`)
- Could add connection pooling/reuse instead of connecting per-message
- Should document prost version compatibility prominently in CLAUDE.md
- Might extract message framing into a separate module for reuse

**Testing Insights:**
- LocalTransport for tests vs TcpTransport for main allows deterministic testing
- Generic I/O traits (Read/Write) enable testing with Cursor instead of real sockets
- #[ignore] on long-running consensus tests keeps `cargo test` fast
- 74 tests passing (73 run + 1 ignored) gives high confidence

**Ready for Manual Testing:**
To test 3-node cluster, run in 3 separate terminals:
```bash
# Terminal 1
cargo run -- --id 1 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

# Terminal 2
cargo run -- --id 2 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

# Terminal 3
cargo run -- --id 3 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
```

Logs will be in `node-1.log`, `node-2.log`, `node-3.log`.

---

### Phase 1.6 Discovery (Crash Recovery Critical Bug)

**Problem Identified:**
- Node startup panic: `to_commit X is out of range [last_index 0]`
- Occurs when node joins cluster late or restarts after downtime
- Root cause: Ready loop missing snapshot handling

**Why It Happens:**
1. Leader has committed entries 1-9
2. New node starts with empty log (last_index=0)
3. Leader sends heartbeat with commit_index=9 before sending entries
4. raft-rs validates commit_index <= last_index, fails (9 > 0)

**The Two-Pronged Solution:**

**1. Snapshot Handling (CRITICAL - Prevents Panic):**
- Add snapshot processing in Ready loop BEFORE entry append
- When node is too far behind, leader sends snapshot instead of entries
- Snapshot brings node up to date instantly (e.g., covers entries 1-1000)
- Required for: new nodes joining, nodes down for long time, log compaction

**2. Persistent Storage (HIGH - Enables Crash Recovery):**
- Persist HardState (term, vote, commit) to survive restarts
- Persist log entries to rebuild Raft log after crash
- Persist snapshots to disk for recovery
- Track applied_index separately to prevent duplicate application
- Without persistence: Every restart loses all state, must receive snapshot

**Key Insight:**
- Persistent storage alone won't fix the panic (new nodes start empty)
- Snapshot handling alone won't survive crashes (state lost on restart)
- BOTH are required for production-ready crash recovery

**Implementation Order:**
1. Node snapshot methods (create_snapshot, restore_from_snapshot)
2. Snapshot handling in Ready loop (prevents panic)
3. Disk-backed storage (enables crash recovery)
4. Integration testing (crash scenarios)

**Testing Strategy:**
- Unit tests: Snapshot serialization roundtrip
- Integration tests: Mock snapshot reception in Ready loop
- End-to-end tests: Kill node → restart → verify recovery
- Cluster tests: 3 nodes, kill follower/leader, verify cluster health

---

Last Updated: 2025-11-15 (Added Phase 1.6 - Crash Recovery)
