# S12 — Work-Stealing Thief Loop (the missing production half)

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S11 (Heartbeat loop), S08 (CP SM)
**ADRs:** 0004 (KV cluster), 0001 (P2P + Raft)
**Status:** Draft (pending review)

## Why this story exists

The `Op::StealTask` SM transition in `crates/bee-control/src/control_plane.rs:184-206` is atomic and correct: when a thief's `StealTask` is applied, the SM checks `Orphaned` → atomically sets `Migrating` with `migrating_from_node` and `owner_node = thief_node` → no double-steal. The CP-level guarantee is in place.

What's MISSING: no production code **invokes** `Op::StealTask`. The acceptance criterion is "kill node 2, observe Work-Stealing, new owner resumes". Without a thief loop, no Node ever issues the StealTask op, so Orphaned Tasks stay Orphaned forever.

This story adds the **thief loop**: a background task in each Node that periodically scans the ControlPlane for Orphaned Tasks owned by other Nodes, and issues `StealTask` to take ownership. The SM does the rest (atomic check, transition to Migrating, Migrating → Running once the new owner resumes).

**KV Checkpoint** is the third S12 acceptance criterion ("no data loss: events between the original owner's last checkpoint and the crash are replayed"). It requires a checkpoint-write path in the runtime + a checkpoint-read on resume. **S12.x follow-up** — too big for this quick win.

## What already exists at HEAD

- `Op::StealTask { thief_node, task_id }` — SM transition is atomic; no two thieves can win
- `Op::MarkNodeOrphaned { node_id }` — bulk-marks all Tasks of a dead node as Orphaned (called by the S11 leader loop)
- `TaskRecord.migrating_from_node: Option<u32>` — set by the SM on successful StealTask
- `TaskStatus::Orphaned | Migrating | Running` — lifecycle states
- The leader's orphan-detection loop in `crates/bee-control/src/heartbeat.rs:detect_and_mark_orphans`

## Scope

### In scope

1. **`NodeThiefLoop`** — a background async task that runs once per Node. Every 1 second, it scans the ControlPlane for `TaskStatus::Orphaned` tasks whose `owner_node != self` and issues `Op::StealTask { thief_node: self }` for each.
2. **Integration with `Cluster`**: the thief loop starts when the Node joins the cluster and stops when the Node shuts down. It's a per-Node task, not cluster-wide.
3. **Backoff**: if a `StealTask` is rejected (because the Task was already taken by another thief), the loop continues; the next iteration re-scans.
4. **Integration test** (`crates/bee-control/tests/work_stealing.rs`): 3-Node in-process cluster, deploy a Job, "kill" node 2 (`shutdown_node(2)`), wait for the thief loop, assert that within 5 seconds the Job's Tasks are owned by node 1 or node 3 with status `Migrating` or `Running`.

### Out of scope (deferred)

- **KV Checkpoint** — S12 acceptance criterion #3. Requires runtime-level integration (`run_with_metrics` would checkpoint the `Arc<PhaseMetrics>` + state to KV at intervals; new owner reads the checkpoint on resume). Big story; S12.x follow-up.
- **Real worker thread on the new owner** — the SM transitions to `Migrating`, then needs a worker to call `dispatch_handler` to resume. The MVP has the bookkeeping but no actual resumption. S49.x follow-up (worker that consumes the Task from the CP and starts running it).
- **Network StealTask (cross-Node) wire protocol** — the local in-process cluster's SM works. Cross-Node StealTask over BRP is a S33.1 follow-up (the multi-node demo).

## File structure

| File | Action |
|---|---|
| `crates/bee-control/src/work_stealing.rs` | New module: `NodeThiefLoop::run` |
| `crates/bee-control/src/lib.rs` | Modify: re-export the new module |
| `crates/bee-control/tests/work_stealing.rs` | New test file: 3-Node cluster + kill-Node-2 + assert Migrating |

1 Task (small).

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` ≥ 432 passed, 0 failed
- [ ] Integration test: 3-Node in-process cluster, deploy a Job on node 1, `shutdown_node(2)`, the thief loop on node 1 (or 3) takes over the Tasks within 5 seconds
- [ ] Concurrent StealTask from two thieves: only one wins (covered by the existing atomic CAS at `control_plane.rs:184-206`); the integration test confirms the second thief gets no-op'd

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `NodeThiefLoop` runs in each Node and issues `StealTask` for Orphaned Tasks owned by other Nodes | ✓ (S12) | N — needs a real worker thread on the new owner to actually resume (S49.x) |
| Concurrent StealTask: one winner | ✓ (S12) — locked down by the existing SM atomic check | N |
| KV Checkpoint + replay | — | N — S12.x follow-up (too big) |
| Network StealTask wire | — | N — S33.1 follow-up (multi-node demo) |

## Related work

- **S11** (Heartbeat + 3× missed = Orphaned) — done; the leader's `detect_and_mark_orphans` calls `Op::MarkNodeOrphaned` which sets all the dead node's Tasks to `Orphaned`. The thief loop reads `Orphaned` Tasks and takes them.
- **S18** (cross-Pipeline edges) — populates `TaskRecord::dependencies` in production; orthogonal to S12.
- **S49** (bee deploy + bee jobs wait) — provides the deploy path that creates Tasks; the thief loop runs after the deploy.

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Where does `NodeThiefLoop` run? | **In each `Node` process** (per-Node task) | Matches the design — each Node scans for Orphaned Tasks and tries to take them |
| Polling interval | **1 second** | Quick enough to feel "live"; slow enough to not hammer the SM |
| `NodeThiefLoop` vs Cluster-level scan | **Per-Node** | Avoids single-point-of-failure; a dead leader doesn't block the loop |

If any of these decisions should change, the user can override during the spec review.