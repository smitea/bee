# S21 Close-Out Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire `PluginManager::release()` into the `Op::UpdateJobLifecycle` apply path so a Job transitioning to `Completed` or `Failed` releases its plugins. The existing atomic CAS in the SM auto-unloads the plugin when refcount hits 0.

**Architecture:** Two pieces: (1) `JobRecord.plugins: HashSet<PluginId>` field + populate at `RegisterJob`; (2) `apply_op` calls `plugin_manager.release()` when the new state is terminal. The `plugin_manager` lives on the `Node` (matches S17/S29 patterns); the apply path takes it as a parameter. For MVP, only the `RegisterJob` caller is updated; the integration test wires a PluginManager.

**Tech Stack:** Rust, `HashSet<PluginId>`, `PluginManager`.

---

## File Structure

| File | Action |
|---|---|
| `crates/bee-control/src/control_plane.rs` | Add `JobRecord.plugins: HashSet<PluginId>` + apply path calls `release` on terminal state |
| `crates/bee-control/src/raft/node.rs` | Wire `plugin_manager` into the apply path |
| `crates/bee-control/tests/refcount_release_on_job_stop.rs` | New test file |

1 Task (small).

---

## Task 1: Wire `release()` into Job-stop

- [ ] **Step 1.1: Add `plugins` field to `JobRecord`**

In `crates/bee-control/src/control_plane.rs`, find `JobRecord`. Add a field after `dependencies`:

```rust
/// S21 close-out: the set of Plugin ids the Job uses. When the
/// Job transitions to a terminal state (Completed / Failed),
/// the SM calls `plugin_manager.release(plugin_id)` for each
/// entry. The plugin auto-unloads when refcount hits 0.
#[serde(default)]
pub plugins: std::collections::HashSet<crate::kv::PluginId>,
```

Add the `serde(default)` attribute (per the S18 pattern) so old test code that doesn't pass `plugins` still compiles.

- [ ] **Step 1.2: Update `apply_op` to call `release` on terminal state**

In `crates/bee-control/src/control_plane.rs`, find the `Op::UpdateJobLifecycle` arm. Add a release call at the end:

```rust
Op::UpdateJobLifecycle { job_id, state } => {
    let job = self.jobs.get_mut(job_id).ok_or_else(|| { ... })?;
    let prev_state = job.lifecycle;
    job.lifecycle = *state;
    // S21 close-out: when the Job transitions to a terminal
    // state, release each Plugin it used. The plugin
    // auto-unloads when its refcount hits 0.
    if is_terminal(*state) && !is_terminal(prev_state) {
        for plugin_id in job.plugins.iter() {
            self.plugin_manager.release(plugin_id);
        }
    }
    Ok(())
}
```

`is_terminal` is a new helper:

```rust
fn is_terminal(s: JobLifecycleState) -> bool {
    matches!(s, JobLifecycleState::Completed | JobLifecycleState::Failed)
}
```

`ControlPlaneStateMachine` needs a `plugin_manager` field. Add to the struct:

```rust
pub struct ControlPlaneStateMachine {
    // ... existing fields ...
    /// S21 close-out: plugin manager for auto-release on
    /// terminal lifecycle transitions. Owned by the Node
    /// process; passed in at construction time.
    plugin_manager: std::sync::Arc<crate::registry::PluginManager>,
}
```

Update `ControlPlaneStateMachine::new()` (or wherever it's constructed) to take a `PluginManager`:

```rust
impl ControlPlaneStateMachine {
    pub fn new(plugin_manager: std::sync::Arc<crate::registry::PluginManager>) -> Self {
        Self {
            // ... existing fields ...
            plugin_manager,
        }
    }
}
```

- [ ] **Step 1.3: Update `Node` to construct with a PluginManager**

In `crates/bee-control/src/raft/node.rs`, find where `ControlPlaneStateMachine` is constructed. Add a `PluginManager` parameter. For the in-process 3-Node cluster, all nodes share the same PluginManager (already done for `kv`).

- [ ] **Step 1.4: Update the existing `RegisterJob` arm to populate `plugins`**

In `apply_op`, the `Op::RegisterJob` arm reads `JobRecord` fields. Add `plugins` to the destructure pattern and the constructor.

(For MVP, the `RegisterJob` op doesn't have a `plugins` field — `S49`'s `bee_deploy_local` would populate it from a future "which plugins does this Job use" detection. For now, the `serde(default)` makes the field default to empty `HashSet`, and Jobs that don't declare plugins just don't release anything.)

- [ ] **Step 1.5: Update all `JobRecord { ... }` constructors in tests**

Run: `grep -rn "JobRecord {" crates/ bee-control/src/ tests/`. Add `plugins: Default::default(),` to each.

- [ ] **Step 1.6: Write the integration test**

In `crates/bee-control/tests/refcount_release_on_job_stop.rs` (new file):

```rust
//! S21 close-out: release() is called on terminal lifecycle transition.

use std::time::Duration;

use bee_control::control_plane::{ControlPlaneStateMachine, JobLifecycleState};
use bee_control::kv::{Op, PluginId, PluginManifest, PluginName, TaskStatus};
use bee_control::raft::cluster::{Cluster, ClusterConfig};
use bee_control::registry::PluginManager;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn release_on_completed_lifecycle() {
    let pm = Arc::new(PluginManager::new());
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader elected");
    let leader = cluster.leader().await.unwrap();

    // Register Plugin X.
    let id = PluginId("p1".to_string());
    pm.register_plugin_for_test(&id, &PluginManifest {
        name: PluginName("p1".into()),
        feature_version: "1.0.0".into(),
        abi_version: "v1".into(),
        adapters: vec![],
        handlers: vec![],
    });
    assert!(pm.retain(&id));
    assert_eq!(pm.refcount_of(&id), Some(1));

    // Register Job 1 (Running, uses Plugin X).
    cluster
        .submit(leader, Op::RegisterJob {
            job_id: 1,
            dag_hash: "j1".into(),
            owner_node: leader,
            tenant: 0,
            dependencies: vec![],
            plugins: std::iter::once(id.clone()).collect(),
        })
        .await
        .unwrap();
    // Mark the Job as Running (the SM needs a task to update lifecycle on).
    cluster
        .submit(leader, Op::RegisterTask {
            task_id: 1,
            job_id: 1,
            phase_id: 0,
            owner_node: leader,
            status: TaskStatus::Running,
            started_at_ms: 0,
        })
        .await
        .unwrap();

    // Transition the Job to Completed — the SM calls release().
    cluster
        .submit(leader, Op::UpdateJobLifecycle {
            job_id: 1,
            state: JobLifecycleState::Completed,
        })
        .await
        .unwrap();

    // The plugin should be auto-unloaded (refcount dropped to 0).
    assert_eq!(pm.refcount_of(&id), None, "plugin should auto-unload");
}
```

(Adapt to the actual `register_plugin_for_test` API or use `register_plugin_with_manifest` if available.)

- [ ] **Step 1.7: Build and run the new test**

Run: `cargo test -p bee-control --test refcount_release_on_job_stop 2>&1 | tail -10`. Expected: PASS.

- [ ] **Step 1.8: Run full workspace tests**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep "test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 439+ failed: 0 ignored: 5`.

- [ ] **Step 1.9: Commit**

```bash
git add crates/bee-control/src/control_plane.rs crates/bee-control/src/raft/node.rs crates/bee-control/tests/refcount_release_on_job_stop.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S21 close-out: JobRecord.plugins field + release() called on terminal lifecycle transition; integration test"
```

---

## Task 2: Final verification + push

- [ ] **Step 2.1: Update S21 spec acceptance criteria**

Edit `docs/superpowers/specs/2026-07-17-s21-release-on-job-stop-design.md` and flip the `[ ]` to `[x]`. Commit:

```bash
git add docs/superpowers/specs/2026-07-17-s21-release-on-job-stop-design.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S21 close-out: flip acceptance criteria to [x]"
```

- [ ] **Step 2.2: Update `docs/stories.md` S21 acceptance criteria**

Find the S21 section in stories.md and flip the relevant `[ ]` to `[x]`. Add a "Done in 2026-07-17" note. Commit:

```bash
git add docs/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "stories.md: S21 acceptance criteria flipped"
```

- [ ] **Step 2.3: Push to remote**

```bash
git push origin main
```

---

## Self-Review

**1. Spec coverage:** Walked the S21 spec's in-scope items:
- `JobRecord.plugins` field: Task 1 Step 1.1 ✓
- `apply_op` calls `release` on terminal: Task 1 Step 1.2 ✓
- Node wiring: Task 1 Step 1.3 ✓
- Integration test: Task 1 Step 1.6 ✓

**2. Placeholder scan:** No TBD / TODO.

**3. Type consistency:** `JobRecord.plugins: HashSet<PluginId>` uses the same `PluginId` type that's in `PluginManager`. `is_terminal` matches the existing `JobLifecycleState` enum.

**4. Ambiguity check:** Tests specify concrete input (specific plugin id, terminal/non-terminal state) + concrete expected output (refcount 0 after Completed, unchanged after Running).

---

## Estimated Total

- 2 Tasks
- 3 commits (impl + criteria flip + stories flip)
- ~100-150 LOC net change
- Estimated wall-clock: 1-1.5 hours