# Raft TUI (raft-a-tui)

An interactive playground for learning about the Raft consensus algorithm, powered by:

1. **[raft-rs](https://github.com/tikv/raft-rs)** for the consensus core and
2. **[ratatui](https://ratatui.rs/)** for a terminal UI.

This project lets you spin up multiple terminal windows, each running a Raft node with its own TUI. You can submit commands (like `PUT`, `GET`, `STATUS`, `KEYS`, `CAMPAIGN`), watch elections happen in real time, and see how log entries replicate across the cluster. It’s a hands-on way to understand how Raft behaves under normal conditions and during leadership changes.

---

## What You Can Do

* Start **three Raft nodes in separate terminals**, each with its own interactive UI.
* Issue commands via a REPL:

  * `PUT key value` — Replicate a write through the cluster.
  * `GET key` — Read from local state.
  * `KEYS` — Pretty-print the state machine.
  * `STATUS` — View node ID, term, role (leader/follower/candidate), leader ID.
  * `CAMPAIGN` — Force an election (great for exploring leader changes).
* Watch:

  * Log replication
  * Elections and term changes
  * Commit/apply cycles
  * Your own commands and system events displayed in separate color-coded sections

## Running the Example

Once built, open three terminals:

```bash
# terminal 1
cargo run -- --id 1 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

# terminal 2
cargo run -- --id 2 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003

# terminal 3
cargo run -- --id 3 --peers 1=127.0.0.1:6001,2=127.0.0.1:6002,3=127.0.0.1:6003
```

You’ll now have three interactive terminal UIs representing a Raft cluster.

Try things like:

```
PUT x hello
STATUS
CAMPAIGN
GET x
KEYS
```

---

## References & Further Reading

1. **Raft Algorithm** — [https://raft.github.io/](https://raft.github.io/)
2. **raft-rs** (TiKV’s Rust implementation) — [https://github.com/tikv/raft-rs](https://github.com/tikv/raft-rs)
3. **Implement Raft in Rust** — [https://tikv.org/blog/implement-raft-in-rust/](https://tikv.org/blog/implement-raft-in-rust/)
4. **TiKV Deep Dive** — [https://tikv.org/deep-dive/introduction/](https://tikv.org/deep-dive/introduction/)
5. **ratatui** — [https://ratatui.rs/](https://ratatui.rs/)
