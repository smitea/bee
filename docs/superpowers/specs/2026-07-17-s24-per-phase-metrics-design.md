# S24 — Per-Phase Metrics + `bee diagnostics` Real Numbers

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S15 (DataFusion executor wrapper — already done; S24 reuses the `run_with_metrics` plumbing)
**ADRs:** 0008 (scheduler)
**Status:** Draft (pending review)
**Source WIP:** `PhaseMetrics` already exists at `crates/bee-runtime/src/metrics.rs:130` (plumbed through `run_with_metrics` at `crates/bee-runtime/src/lib.rs:399`); the runtime records `events_processed_total`, `processing_latency` (histogram with 5 buckets: ≤1ms, ≤10ms, ≤100ms, ≤1s, ≤10s), `backpressure_wait_seconds_total` per Task.

## Why this story exists

`PhaseMetrics` is plumbed through the runtime (per-Task counters) but never exposed to the ControlPlane. `format_task_diagnostics` (in `crates/bee-control/src/diagnostics_view.rs:14`) shows placeholder lines ("requires Node admin RPC; S28 follow-up"). The S24 acceptance criterion "`bee diagnostics <TaskId>` prints all four metrics for the given Task" is unfulfilled.

This story closes the gap by:
1. Adding a `TaskMetrics` store (`HashMap<u32, Arc<PhaseMetrics>>`) on the `ControlPlaneStateMachine`
2. Worker code (or the test harness) populates the store as the Task runs
3. `format_task_diagnostics` reads from the store and renders the real values
4. `bee diagnostics <id>` displays them end-to-end

For MVP, the metrics store is **in-process** (same 3-Node in-process cluster shares one CP). The cross-Node wire format (each worker reports its Task's metrics to the leader over BRP) is a S49.x follow-up.

## What already exists at HEAD

- `PhaseMetrics` struct (`crates/bee-runtime/src/metrics.rs:130`)
- `Histogram` with 5 buckets + `record_latency` / `record_event_processed` / `record_backpressure_wait` methods
- `run_with_metrics(dag, input_rx, output_tx, metrics: Option<Arc<PhaseMetrics>>)` — runtime plumbs the metrics
- `format_task_diagnostics(cp, task_id)` (placeholder output)

## Scope

### In scope

1. **`ControlPlaneStateMachine::task_metrics: Mutex<HashMap<u32, Arc<PhaseMetrics>>>`** — stores metrics per Task. The SM can update it from `Op::RecordTaskMetrics { task_id, events_processed_total, latency_p50, latency_p99, backpressure_wait_seconds_total }`.
2. **`Op::RecordTaskMetrics { task_id, ... }`** — the wire-format op the worker uses to push metrics back to the leader.
3. **`format_task_diagnostics` reads from `task_metrics`** — replace the placeholder lines with real values formatted as "events: 1234", "p50: 5ms", "p99: 50ms", "backpressure: 1.2s".
4. **Worker integration** — `crates/bee-runtime/src/lib.rs::run_inner` calls a closure (passed in via `run_with_metrics`) that pushes `Op::RecordTaskMetrics` to the cluster after each batch. The cluster `submit`s the op to the leader, which updates the SM.

   For MVP: add a `metrics_flush_fn: Option<F>` parameter to `run_with_metrics` where `F: Fn(u32, &PhaseMetrics)`. The default is a no-op. The tests pass a closure that pushes the op.
5. **Integration test** (`crates/bee-control/tests/diagnostics_view.rs`): register a Task; simulate 5 events; assert `format_task_diagnostics` shows "events: 5" and the histogram buckets.
6. **`bee diagnostics <id>` smoke test** — runs `bee run` + `bee diagnostics` end-to-end.

### Out of scope (deferred)

- **Network metrics push (cross-Node)** — each worker reports metrics to the leader over BRP. MVP is in-process only. S49.x follow-up.
- **CPU usage** (`cpu_seconds_total` from cgroup / process stats) — the 4 metrics acceptance criterion is satisfied without it; can be added as a 5th metric later.
- **`< 1% CPU overhead`** — needs a benchmark harness; not a quick win.

## File structure

| File | Action |
|---|---|
| `crates/bee-control/src/kv.rs` | Add `Op::RecordTaskMetrics` variant + `TaskMetricsSnapshot` struct |
| `crates/bee-control/src/control_plane.rs` | Add `task_metrics` field + `Op::RecordTaskMetrics` apply arm + new `get_task_metrics(task_id)` accessor |
| `crates/bee-control/src/diagnostics_view.rs` | Replace placeholder lines in `format_task_diagnostics` with real values |
| `crates/bee-runtime/src/lib.rs` | Add `metrics_flush_fn` parameter to `run_with_metrics` |
| `crates/bee-control/tests/diagnostics_view.rs` | New test file |

1 Task (small).

## Acceptance criteria

- [x] `cargo build --workspace` green
- [x] `cargo test --workspace` ≥ 436 passed, 0 failed (achieved **438** — baseline 436 + 2 new S24 tests)
- [x] `format_task_diagnostics` with a Task that has recorded 5 events shows "events: 5" and the histogram buckets show the right counts (locked down by `format_diagnostics_renders_real_metrics` — verifies "events_processed_total: 5" and bucket array "1 / 2 / 2 / 0 / 0")
- [ ] `bee run` + `bee diagnostics <id>` end-to-end works (manual smoke test; in-process MVP, both commands run in the same process and can read the metrics)
- [x] Histogram buckets are documented (5 buckets: ≤1ms, ≤10ms, ≤100ms, ≤1s, ≤10s) — the `latency_bucket_counts: [u64; 5]` field matches `Histogram::bucket_counts()` exactly

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `format_task_diagnostics` shows real metrics | ✓ (S24) | N — in-process MVP |
| Worker pushes metrics via `Op::RecordTaskMetrics` | ✓ (S24) | N — in-process only; cross-Node is S49.x |
| Histogram buckets | ✓ (S24) | N |
| `< 1% CPU overhead` | — | N — needs benchmark harness |
| Network metrics push | — | N — S49.x |

## Related work

- **S44** (prime_sieve trim) — done; independent
- **S17** (Producer detection) — done; the `Op::RecordTaskMetrics` is similar in spirit (worker → leader)
- **S49.x** (worker execution + cross-Node) — the metrics push needs cross-Node wire for production

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| In-process vs network metrics push? | **In-process** (MVP) | S49.x adds the cross-Node wire |
| Where does the metrics flush happen? | **Worker → cluster.submit** (per task) | Matches S17's pattern |
| Format of displayed metrics? | **Human-readable** ("events: 1234", "p50: 5ms") | CSV / Prometheus format is a 1.x follow-up |

If any of these decisions should change, the user can override during the spec review.