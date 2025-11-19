# Raft-a-TUI Client-Server Plan

## Overview
We are transforming `raft-a-tui` from a local educational REPL into a distributed Key-Value database with a public gRPC API. This will allow external clients to interact with the Raft cluster over the network.

The system will follow a "Smart Client" architecture where the client is responsible for routing requests to the correct leader and handling failovers, avoiding the need for a centralized load balancer.

## Architecture

### 1. Protocol Definition (`proto/rpc.proto`)
We will define a gRPC service `KeyValueService` with:
- `Put(key, value)`
- `Get(key)`

### 2. "Smart Client" Load Balancing
To handle distributed consensus dynamics without a proxy:
- **Writes (PUT)**: Must go to the Leader.
    - If sent to a Follower, the server returns `FAILED_PRECONDITION` with metadata headers:
        - `x-raft-leader-id`: The ID of the current leader.
        - `x-raft-leader-addr`: The address of the current leader.
    - The client catches this error, updates its internal "Leader Cache", and retries the request against the new leader.
- **Reads (GET)**: Can go to any node (Eventual Consistency).
    - The client will randomly select a peer from its known list (excluding the known leader if desired, or including it).
    - This demonstrates "stale reads" from followers vs "strong reads" from the leader (if we choose to enforce leader-only reads for consistency later).

### 3. Server-Side Integration (The Bridge)
The current architecture is:
`TUI` -> `UserCommand` (Sync Channel) -> `RaftDriver` (Main Loop)

We will adapt this to support async gRPC:
1.  **gRPC Handler (Tokio)**: Receives `PutRequest`.
2.  **Bridge Channel**: Sends a `RpcCommand` to `RaftDriver` along with a `oneshot::Sender` for the response.
3.  **RaftDriver (Sync)**:
    - Accepts `RpcCommand`.
    - If `Put`: Proposes to Raft. When committed, triggers callback to `oneshot::Sender`.
    - If `Get`: Reads local state immediately and responds via `oneshot::Sender`.
4.  **gRPC Handler**: Awaits the `oneshot` signal and returns the gRPC response.

## Codebase Reorganization (Crates)
To keep things clean and avoid compiling the heavy TUI logic for the client, we will split the project into a workspace.

**Workspace Structure:**
- `raft-core` (Lib): The Raft logic, storage, and transport (shared).
- `raft-server` (Bin): The existing `raft-a-tui` server (renamed).
- `raft-client` (Bin): A new lightweight CLI/TUI client.
- `raft-proto` (Lib): The generated Prost/Tonic code.

*(For this iteration, we might keep it monolithic for simplicity if preferred, but a workspace is the ideal end state. We will start by keeping it monolithic but structuring modules to be separable).*

## Implementation Steps

1.  **Dependencies**: Add `tonic`, `tokio`, `prost` to `Cargo.toml`.
2.  **Proto**: Create `proto/rpc.proto`.
3.  **Core Logic**:
    - Update `UserCommand` to support response channels (`RpcCommand`).
    - Update `RaftDriver` to handle these commands and manage callbacks.
4.  **Server**:
    - Implement `KeyValueService` trait.
    - Spawn Tonic Server in a background thread in `main.rs`.
5.  **Client**:
    - Create a new binary (e.g., `src/bin/client.rs`) or a separate crate.
    - Implement the "Smart Client" retry logic.
    - Build a simple Ratatui interface for the client (input box + log output).
