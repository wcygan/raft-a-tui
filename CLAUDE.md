# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**raft-a-tui** is an educational interactive playground for learning the Raft consensus algorithm. It combines:
- **raft-rs** (TiKV's Raft implementation) for distributed consensus
- **ratatui** for terminal-based UI visualization

The project is in early development. The foundational KV store, command parsing, and protobuf infrastructure exist. Raft integration and the TUI are not yet implemented (main.rs is a stub).

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

## Future Work (Not Yet Implemented)

Based on code comments and README:
- Raft integration (RawNode, elections, log replication, Ready messages)
- TUI rendering (ratatui event loop, split panes, color-coded logs)
- Network layer (peer communication)
- STATUS command needs Raft term/role/leader info
- CAMPAIGN command should trigger `RawNode::campaign()`
- Snapshot serialization (bincode dependency present but unused)
