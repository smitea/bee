# 0004: Bee KV Cluster for shared Task State

The original Q4 MVP state model was per-Task private state stored in local WAL on the owning Node, requiring state serialization and BRP-channel transfer on Migrating. The user pushed for stronger semantics: state should be cluster-wide accessible for consistency and transaction support. We resolve this by giving each Task **private ownership** of its state, but storing that state in a **cluster-shared KV** that runs as a second logical state machine on the same Raft cluster as the Control Plane. Each Node is simultaneously a Worker, a Raft participant, and a KV node. The KV API is `get / put / cas / txn(ops)` over opaque bincode values — no range scan, no secondary index in MVP. This simplifies Migrating (new owner Node reads state from KV in one Raft read), simplifies Failover (new owner reads from KV + replays from saved offset), and provides linearizable reads/writes plus multi-key transactions for atomic state-and-offset snapshots.

## Consequences

- Every state op is a Raft consensus op (~1–5ms latency). Acceptable for the targeted quant use case (5-min K-lines); constrains ultra-low-latency operators. Mitigation: default in-memory hot cache + periodic sync to KV; Handler sees ~1µs latency in the common case.
- KV writes share Raft throughput with control plane ops. MVP accepts this; 1.x can split into multiple Raft groups (TiKV-style) if measurements show contention.
- State capacity = total disk of the Raft cluster. Default caps: 1 GB / Task, 7-day TTL after Job stops.
- Migrating latency drops to ~1–5ms (one Raft read) instead of serialize-over-BRP. New owner resumes from the latest KV checkpoint and replays upstream from the saved offset.
- Time-window state (MACD / RSI / ASOF history) is maintained by the Handler as an in-memory data structure and persisted as a single bincode blob. Typical quant windows (N < 10K) round-trip in microseconds; for larger N the user partitions across keys or moves to a dedicated time-series store.
- API: `kv.get(key)`, `kv.put(key, value)`, `kv.cas(key, expected, new)`, `kv.txn(ops)`. Keys are namespaced: `state/task/{TaskId}/{state_name}` for per-Task state, `state/checkpoint/{TaskId}` for atomic (state + saved offset) snapshots.
- The KV does not interpret value contents; it is intentionally generic so that Handler authors can build any stateful structure (ring buffers, sorted maps, custom aggregations) on top.
