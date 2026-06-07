# 0007: Simplified all-in-one Raft topology for MVP

The control plane (Job/Task ownership, StealTask arbitration) and the KV cluster (ADR-0004) both run on the same Raft cluster. We adopt the **simplified topology (A)** for MVP: every Bee Node is simultaneously a Worker, a Raft participant, and a KV node. The Raft cluster is the same as the data-plane mesh. Raft cluster size is configurable via `bee.cluster.raft_size` (default 3 for dev, 5 for production). This gives single-binary deployment (`bee cluster init --node 1,2,3`) and minimal operational complexity. To prevent worker load from interfering with Raft consensus, **control-plane RPCs and heartbeats run at high priority** and are scheduled on a dedicated channel that bypasses worker data-flow contention.

The progression to **B (dedicated control plane) in 1.x** is triggered by any of three quantified conditions: (1) Raft p99 consensus latency > 10ms sustained for 1 week, (2) Worker pool > 50 Nodes, (3) explicit user request for independent control-plane scaling. At that point the existing crate boundaries (`bee-control` vs `bee-runtime`) already allow process-level split without code refactor.

**C (tiered: control plane + KV nodes + worker pool)** is reserved for 2.x and only if scale demands it.

## Consequences

- Single-binary deployment simplifies MVP, demo, and small-team production. Operational story: `init` + `add-node` + `deploy` and you're done.
- Worker load contention with Raft consensus is mitigated but not eliminated. High-throughput worker tasks can still cause tail-latency spikes in consensus; the priority mechanism caps the damage but does not remove it. This is acceptable for MVP; the trigger conditions are designed to catch it before production pain.
- Raft cluster size is effectively the data-plane size. Adding 100 worker Nodes means 100 Raft participants, which has known scalability limits (~7 Nodes for healthy consensus, ~15 with careful tuning). The trigger conditions are designed to catch this before it bites.
- `bee-control` and `bee-runtime` are already separate crates so the 1.x split to a dedicated control plane is mostly a deployment change, not a code change.
- KV cluster inherits the same Node membership. 1.x may also split KV into its own Raft groups (TiKV-style) if KV write throughput outgrows the shared Raft group — independently of the A→B progression.
- Migration to B is a 1.x decision: code paths already exist in separate crates; only the deployment topology and the routing of RPCs change.
