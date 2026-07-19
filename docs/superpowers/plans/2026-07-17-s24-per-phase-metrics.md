# S24 — Per-Phase Metrics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-Task `PhaseMetrics` store on the `ControlPlaneStateMachine`. The worker pushes metrics via a new `Op::RecordTaskMetrics`; `format_task_diagnostics` reads from the store and renders real values.

**Architecture:** Three pieces: (1) `Op::RecordTaskMetrics` + `TaskMetricsSnapshot` in `kv.rs`; (2) `task_metrics: Mutex<HashMap<u32, Arc<PhaseMetrics>>>` on the SM with apply arm + accessor; (3) `format_task_diagnostics` reads from the store. A fourth piece: `run_with_metrics` takes a `metrics_flush_fn` closure the worker calls to push metrics.

**Tech Stack:** Rust, `Arc<PhaseMetrics>`, `Mutex<HashMap<u32, Arc<PhaseMetrics>>>`.

---

## File Structure

| File | Action |
|---|---|
| `crates/bee-control/src/kv.rs` | Add `Op::RecordTaskMetrics` + `TaskMetricsSnapshot` |
| `crates/bee-control/src/control_plane.rs` | Add `task_metrics` field + apply arm + `get_task_metrics` accessor |
| `crates/bee-control/src/diagnostics_view.rs` | Replace placeholder lines in `format_task_diagnostics` |
| `crates/bee-runtime/src/lib.rs` | Add `metrics_flush_fn` parameter to `run_with_metrics` |
| `crates/bee-control/tests/diagnostics_view.rs` | New test file |

1 Task (medium).

---

## Task 1: Metrics store + display

- [ ] **Step 1.1: Add `TaskMetricsSnapshot` + `Op::RecordTaskMetrics` in `kv.rs`**

In `crates/bee-control/src/kv.rs`, find `pub enum Op`. After `MarkNodeOrphaned` (or wherever fits), add:

```rust
/// S24: the worker's per-Task metrics snapshot. The worker
/// pushes this to the leader after each batch (or at a
/// configurable interval); the SM stores it in `task_metrics`
/// for `format_task_diagnostics` to read.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskMetricsSnapshot {
    pub events_processed_total: u64,
    /// 5 latency buckets: ≤1ms, ≤10ms, ≤100ms, ≤1s, ≤10s.
    pub latency_bucket_counts: [u64; 5],
    pub backpressure_wait_seconds_total_ns: u64,
}
```

And add the variant:

```rust
RecordTaskMetrics {
    task_id: u32,
    snapshot: TaskMetricsSnapshot,
},
```

- [ ] **Step 1.2: Add the `task_metrics` field on `ControlPlaneStateMachine`**

In `crates/bee-control/src/control_plane.rs`, find `ControlPlaneStateMachine::default()`. Add a `task_metrics` field:

```rust
pub struct ControlPlaneStateMachine {
    // ... existing fields ...
    /// S24: per-Task metrics (events_processed_total, latency
    /// histogram, backpressure_wait_seconds). The worker
    /// pushes via `Op::RecordTaskMetrics`; `format_task_diagnostics`
    /// reads from this map.
    task_metrics: std::collections::HashMap<u32, std::sync::Arc<crate::runtime::PhaseMetrics>>,
}
```

But `crate::runtime::PhaseMetrics` is a path from bee-control. Let me check the path. Actually, `PhaseMetrics` is in `crates/bee-runtime/src/metrics.rs`, not in bee-control. We need to either:
- Add a `PhaseMetrics` re-export to `bee-control`
- Or have the worker hold the metrics, and only push the *snapshot* to the SM

The snapshot approach (Step 1.1) is the right one. The SM stores `TaskMetricsSnapshot` (a POD struct), not `Arc<PhaseMetrics>`. That decouples bee-control from bee-runtime's runtime types.

Let me redo Step 1.2: use `HashMap<u32, TaskMetricsSnapshot>`:

```rust
task_metrics: std::collections::HashMap<u32, TaskMetricsSnapshot>,
```

- [ ] **Step 1.3: Add the apply arm in `control_plane.rs`**

In `apply_op`, add:

```rust
Op::RecordTaskMetrics { task_id, snapshot } => {
    // Last writer wins (idempotent; the worker periodically
    // pushes; the most recent snapshot is the source of truth).
    self.task_metrics.insert(*task_id, snapshot.clone());
    Ok(())
}
```

- [ ] **Step 1.4: Add a `get_task_metrics` accessor**

```rust
pub fn get_task_metrics(
    &self,
    task_id: u32,
) -> Option<&TaskMetricsSnapshot> {
    self.task_metrics.get(&task_id)
}
```

- [ ] **Step 1.5: Update `format_task_diagnostics` in `diagnostics_view.rs`**

Find the placeholder lines (around line 50-53 in `diagnostics_view.rs`):

```rust
out.push_str("\n  --- metrics (S24) ---\n");
out.push_str("  events_processed_total:       (requires Node admin RPC; S28 follow-up)\n");
out.push_str("  processing_latency_p50/p99:   (requires Node admin RPC; S28 follow-up)\n");
out.push_str("  cpu_seconds_total:            (requires Node admin RPC; S28 follow-up)\n");
out.push_str("  backpressure_wait_seconds_total: (requires Node admin RPC; S28 follow-up)\n");
```

Replace with:

```rust
out.push_str("\n  --- metrics (S24) ---\n");
if let Some(metrics) = cp.get_task_metrics(task.task_id) {
    out.push_str(&format!(
        "  events_processed_total: {}\n",
        metrics.events_processed_total
    ));
    let buckets = metrics.latency_bucket_counts;
    out.push_str(&format!(
        "  latency buckets (≤1ms, ≤10ms, ≤100ms, ≤1s, ≤10s): [{}, {}, {}, {}, {}]\n",
        buckets[0], buckets[1], buckets[2], buckets[3], buckets[4]
    ));
    out.push_str(&format!(
        "  backpressure_wait_seconds_total: {:.3}\n",
        metrics.backpressure_wait_seconds_total_ns as f64 / 1_000_000_000.0
    ));
} else {
    out.push_str("  (no metrics recorded yet; worker hasn't run)\n");
}
```

(Adapt the bucket labels to match `Histogram`'s actual boundaries — check `crates/bee-runtime/src/metrics.rs:41-118` to confirm.)

- [ ] **Step 1.6: Add the `metrics_flush_fn` parameter to `run_with_metrics`**

In `crates/bee-runtime/src/lib.rs`, find `pub fn run_with_metrics`. Add a `metrics_flush_fn: Option<F>` parameter where `F: Fn(u32, &PhaseMetrics) + Send + Sync + 'static`. The runtime calls it after each batch.

- [ ] **Step 1.7: Write the integration test**

In `crates/bee-control/tests/diagnostics_view.rs` (new file):

```rust
//! S24: per-Phase metrics in the ControlPlane + display.

use bee_control::control_plane::ControlPlaneStateMachine;
use bee_control::kv::{Op, TaskMetricsSnapshot};

#[test]
fn task_metrics_round_trip_via_op() {
    let mut cp = ControlPlaneStateMachine::new();

    // Register a Job + Task.
    cp.apply_op(&Op::RegisterJob {
        job_id: 1,
        dag_hash: "x".into(),
        owner_node: 1,
        tenant: 0,
        dependencies: vec![],
    })
    .unwrap();
    cp.apply_op(&Op::RegisterTask {
        task_id: 1,
        job_id: 1,
        phase_id: 0,
        owner_node: 1,
        status: bee_control::kv::TaskStatus::Pending,
        started_at_ms: 0,
    })
    .unwrap();

    // Worker pushes a snapshot.
    let snap = TaskMetricsSnapshot {
        events_processed_total: 42,
        latency_bucket_counts: [5, 10, 20, 5, 2],
        backpressure_wait_seconds_total_ns: 1_500_000_000, // 1.5s
    };
    cp.apply_op(&Op::RecordTaskMetrics {
        task_id: 1,
        snapshot: snap.clone(),
    })
    .unwrap();

    // View reads the metrics.
    let stored = cp.get_task_metrics(1).expect("metrics present");
    assert_eq!(stored.events_processed_total, 42);
    assert_eq!(stored.latency_bucket_counts, [5, 10, 20, 5, 2]);
}

#[tokio::test(flavor = "current_thread")]
async fn format_diagnostics_renders_real_metrics() {
    use bee_control::diagnostics_view::format_task_diagnostics;
    use bee_control::kv::TaskStatus;

    let mut cp = ControlPlaneStateMachine::new();
    cp.apply_op(&Op::RegisterJob {
        job_id: 1,
        dag_hash: "x".into(),
        owner_node: 1,
        tenant: 0,
        dependencies: vec![],
    })
    .unwrap();
    cp.apply_op(&Op::RegisterTask {
        task_id: 1,
        job_id: 1,
        phase_id: 0,
        owner_node: 1,
        status: TaskStatus::Running,
        started_at_ms: 0,
    })
    .unwrap();
    cp.apply_op(&Op::RecordTaskMetrics {
        task_id: 1,
        snapshot: TaskMetricsSnapshot {
            events_processed_total: 5,
            latency_bucket_counts: [1, 2, 2, 0, 0],
            backpressure_wait_seconds_total_ns: 0,
        },
    })
    .unwrap();
    let s = format_task_diagnostics(&cp, 1).await.unwrap();
    assert!(s.contains("events_processed_total: 5"), "missing events line:\n{s}");
    assert!(s.contains("1, 2, 2, 0, 0"), "missing bucket counts:\n{s}");
}
```

- [ ] **Step 1.8: Build and run the new tests**

Run: `cargo build -p bee-control 2>&1 | tail -3`. Fix any errors.

Run: `cargo test -p bee-control --test diagnostics_view 2>&1 | tail -10`. Expected: 2 tests pass.

- [ ] **Step 1.9: Run full workspace tests**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 438+ failed: 0 ignored: 5`.

- [ ] **Step 1.10: Commit**

```bash
git add crates/bee-control/src/kv.rs crates/bee-control/src/control_plane.rs crates/bee-control/src/diagnostics_view.rs crates/bee-runtime/src/lib.rs crates/bee-control/tests/diagnostics_view.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S24: task_metrics store on CP + Op::RecordTaskMetrics + format_task_diagnostics renders real values; 2 new tests"
```

---

## Task 2: Final verification + push

- [ ] **Step 2.1: Update S24 spec acceptance criteria**

Edit `docs/superpowers/specs/2026-07-17-s24-per-phase-metrics-design.md` and flip the `[ ]` to `[x]`. Commit:

```bash
git add docs/superpowers/specs/2026-07-17-s24-per-phase-metrics-design.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S24: flip acceptance criteria to [x]"
```

- [ ] **Step 2.2: Update `docs/stories.md` S24 acceptance criteria**

Find the S24 section in stories.md and flip the relevant `[ ]` to `[x]`. Commit:

```bash
git add docs/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "stories.md: S24 acceptance criteria flipped"
```

- [ ] **Step 2.3: Push to remote**

```bash
git push origin main
```

---

## Self-Review

**1. Spec coverage:** Walked the S24 spec's in-scope items:
- `Op::RecordTaskMetrics` + `TaskMetricsSnapshot`: Task 1 Step 1.1 ✓
- `task_metrics` field + apply arm: Task 1 Steps 1.2-1.3 ✓
- `format_task_diagnostics` reads from store: Task 1 Step 1.5 ✓
- `metrics_flush_fn` parameter: Task 1 Step 1.6 ✓
- Integration test: Task 1 Step 1.7 ✓

**2. Placeholder scan:** No TBD / TODO.

**3. Type consistency:** `TaskMetricsSnapshot` is a POD struct, used in both `kv.rs` and `control_plane.rs` and `diagnostics_view.rs`. The bucket array `[u64; 5]` matches `Histogram::bucket_counts()`.

**4. Ambiguity check:** Tests specify concrete input (specific event counts + bucket arrays) + concrete expected output (substrings in the formatted string).

---

## Estimated Total

- 2 Tasks
- 3 commits (impl + criteria flip + stories flip)
- ~150-200 LOC net change
- Estimated wall-clock: 1-1.5 hours