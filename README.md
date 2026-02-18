# 🧭 Raft Consensus in Rust (with Fault Injection Simulator)

A from-scratch implementation of the Raft consensus algorithm in Rust, built with a deterministic multi-node simulation environment supporting network delays, message drops, and partitions.

This project focuses on **correctness under failures**, demonstrating how distributed systems maintain consistency despite unreliable networks.

---

## ✨ Highlights

- 🧠 Leader election (RequestVote RPC)
- 🔁 Log replication (AppendEntries RPC)
- 🧾 Structured log entries with command abstraction
- 🌐 Simulated distributed network using threads + channels
- ⚡ Fault injection:
  - Message drops
  - Random network delays
  - Network partitions
- 🧪 Designed for testing distributed system behavior under chaos

---

## 🏗️ Architecture

Each node runs as an independent thread with:
- Local Raft state (term, role, log)
- Message inbox (channel-based)
- Timers for election + heartbeat

A central **Network layer** simulates real-world failures:


Nodes (threads)
↓
Message passing (mpsc channels)
↓
Network layer
├── random delay
├── message drop
└── partition isolation



---

## 📡 Raft RPCs Implemented

### RequestVote
- Election initiation
- Term comparison
- Vote granting logic

### AppendEntries
- Heartbeats
- Log replication
- Leader commit propagation

---

## 🧪 Fault Injection (Key Feature)

The simulation includes a configurable chaos layer:

- `drop_rate`: probability of message loss
- `min_delay_ms / max_delay_ms`: network latency
- partition map: isolates nodes into groups

This allows testing scenarios like:
- Leader isolation
- Split-brain elections
- Delayed replication
- Recovery after partition healing

---

## ▶️ Running the Simulation

```bash
cargo run

You will observe:
 - Leader elections
 - Term changes
 - Log replication
 - Behavior under network instability

🧠 Design Goals
 - Understand Raft beyond theory
 - Explore behavior under realistic failures
 - Build intuition for:
   - safety vs liveness
   - timing sensitivity
   - distributed coordination

📈 Roadmap
v0.1 (current)
- Leader election
- Basic log replication
- Fault-injection network

v0.2
- Stable commit index handling
- More robust election timers

v0.3
- Log consistency guarantees
- Safety invariants testing

v0.4
- Persistence (WAL / disk-backed logs)
- Crash + recovery

v0.5
- Metrics + benchmarking (latency, throughput)
- Stress testing with randomized failures

🔗 Relation to My Work

This project complements my research in:
- Byzantine Fault Tolerance (pMVBA)
- Distributed systems correctness
- Fault-tolerant protocol design

While Raft targets crash fault tolerance (CFT), my research extends into Byzantine environments, bridging theory and real-world systems.

💡 Future Extensions
- Snapshotting / log compaction
- gRPC-based real network (replace simulation)
- Integration with a KV store (state machine)
- Comparison with BFT protocols

🛠️ Tech Stack
- Rust (concurrency, safety)
- std::thread + mpsc channels
- Custom simulation framework

📌 Why This Project

Distributed systems don't fail in ideal conditions —
they fail under partitions, delays, and partial failures.

This project is built to explore exactly that.
