# raft-a-tui Codebase Guide

## Overview

`raft-a-tui` is an educational interactive playground for learning the Raft consensus algorithm. It allows users to spin up multiple Raft nodes in separate terminal windows, forming a cluster. Each node features a Terminal UI (TUI) to visualize the state, logs, and Key-Value store.

The project is built using:
- **[raft-rs](https://github.com/tikv/raft-rs)**: The core Raft consensus logic (from TiKV).
- **[ratatui](https://ratatui.rs/)**: The Terminal UI library.
- **[sled](https://github.com/spacejam/sled)**: Embedded database for persistent storage.
- **[prost](https://github.com/tokio-rs/prost)**: Protocol Buffers for network messages.

## Architecture

The application is structured around a main event loop that drives the Raft state machine and updates the UI.

### Core Components

1.  **Main Application (`src/main.rs`)**:
    -   Entry point.
    -   Sets up the TUI, logger, and configuration.
    -   Spawns the `RaftDriver` thread.
    -   Runs the TUI event loop (`run_tui`), which consumes state updates from the Raft thread and handles user input.

2.  **Raft Driver (`src/raft_loop.rs`)**:
    -   The "engine" of the node.
    -   Runs in a separate thread.
    -   Implements the "Ready" loop pattern required by `raft-rs`.
    -   Handles:
        -   **Ticks**: Periodically ticks the Raft clock.
        -   **Messages**: Receives Raft messages from peers via `TcpTransport`.
        -   **Commands**: Receives user commands (PUT, CAMPAIGN) from the TUI.
        -   **Persistence**: Writes logs and state to `DiskStorage` (or memory).
        -   **Application**: Applies committed entries to the `Node` (KV State Machine).
        -   **Updates**: Sends `StateUpdate` events back to the TUI.

3.  **Raft Node Wrapper (`src/raft_node.rs`)**:
    -   A high-level wrapper around `raft::raw_node::RawNode`.
    -   Manages the proposal lifecycle (mapping proposals to callbacks).
    -   Provides helpers for checking leadership and getting state.

4.  **State Machine (`src/node.rs`)**:
    -   Represents the actual application state (a simple Key-Value store).
    -   Applies `Put` commands.
    -   Handles snapshots (serialization/deserialization of the map).
    -   Can also handle ephemeral "local" commands (GET, STATUS).

5.  **Transport (`src/tcp_transport.rs`, `src/network.rs`)**:
    -   Handles TCP communication between nodes.
    -   Uses length-prefixed Protobuf messages.
    -   Architecture:
        -   **Listener Thread**: Accepts connections and deserializes incoming messages.
        -   **Sender Thread**: Queues and sends messages to peers, handling retries.

6.  **Storage (`src/storage.rs`, `src/disk_storage.rs`)**:
    -   `RaftStorage`: An enum wrapper supporting both `Memory` and `Disk` backends.
    -   `DiskStorage`: A `sled`-based persistent storage implementation.
        -   **meta**: Stores HardState and ConfState.
        -   **entries**: Stores log entries.
        -   **snapshots**: Stores the latest snapshot.

## File Structure Map

| File | Responsibility |
| :--- | :--- |
| `src/main.rs` | Entry point, TUI rendering, input handling, and system wiring. |
| `src/raft_loop.rs` | The main event loop driving `raft-rs`, handling IO and updates. |
| `src/raft_node.rs` | Wrapper around `RawNode` to simplify API interaction. |
| `src/node.rs` | The KV State Machine implementation. |
| `src/storage.rs` | Abstraction for Raft storage (Memory vs Disk). |
| `src/disk_storage.rs` | Persistent storage implementation using `sled`. |
| `src/tcp_transport.rs` | TCP networking implementation. |
| `src/network.rs` | Transport traits and error definitions. |
| `src/commands.rs` | User command parsing logic (PUT, GET, etc.). |
| `src/cli.rs` | CLI argument parsing (node ID, peers list). |
| `proto/kv.proto` | Protocol Buffer definition for KV commands. |

## Development Workflow

### Building and Running

To run a 3-node cluster locally:

**Terminal 1:**
```bash
cargo run -- --id 1 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
```

**Terminal 2:**
```bash
cargo run -- --id 2 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
```

**Terminal 3:**
```bash
cargo run -- --id 3 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
```

### Commands
Inside the TUI, you can type:
-   `PUT <key> <value>`: Propose a write.
-   `GET <key>`: Read a value locally.
-   `CAMPAIGN`: Force the node to start an election.
-   `STATUS`: View detailed node status.

### Common Tasks
-   **Adding a new command**:
    1.  Update `UserCommand` in `src/commands.rs`.
    2.  Update parser in `src/commands.rs`.
    3.  Handle the command in `src/raft_loop.rs` (`handle_user_command`).
    4.  If it's a replicated command, update `proto/kv.proto` and `src/node.rs`.

### Important Notes
-   **Prost Version**: The project uses `prost` and `raft-rs`. Note that `raft-rs` depends on specific versions of `prost` (often 0.11 or similar legacy versions via re-exports), so care must be taken when serializing/deserializing messages manually in `tcp_transport.rs` and `disk_storage.rs`. The code currently explicitly uses `prost_011` traits in some places.
-   **Logging**: Logs are written to `node-<id>.log` files to avoid interfering with the TUI. Use `tail -f node-1.log` to debug.
