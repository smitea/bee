# S18 — Cross-Pipeline Edge SQL Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `CREATE VIEW v AS SELECT ... FROM <job_id>.output` SQL syntax. At deploy time, extract cross-Pipeline dependencies, populate `JobRecord.dependencies`, and trigger the existing `WaitingForUpstream → Running` lifecycle transition when the upstream Job is `Running`.

**Architecture:** Three pieces: (1) `Op::RegisterJob` gets a new `dependencies: Vec<DependencyRecord>` field (additive); (2) the `preprocess_sql_v2` pipeline gets a `detect_cross_pipeline_deps` pass that walks the SQL for `<u32>.output` references and emits `DependencyRecord`s; (3) `bee_deploy_local` (S49) passes the deps through. The existing `evaluate_job_state` + `job_dependencies_satisfied` logic handles the lifecycle transition.

**Tech Stack:** Rust, regex (existing dep; used by the S42 preprocessor).

---

## File Structure

| File | Action |
|---|---|
| `crates/bee-control/src/kv.rs` | Modify: add `dependencies: Vec<DependencyRecord>` to `Op::RegisterJob` |
| `crates/bee-dsl-sql/src/preprocess.rs` | Modify: add `detect_cross_pipeline_deps` + integrate into `preprocess_sql_v2` |
| `crates/bee-dsl-sql/src/lib.rs` | Re-export the new helper |
| `bee/src/main.rs` | Modify: `bee_deploy_local` passes `dependencies` to `RegisterJob` |
| `crates/bee-control/tests/cross_pipeline.rs` | New test file |

1 Task (medium).

---

## Task 1: Cross-pipeline dep detection + integration

- [ ] **Step 1.1: Add `dependencies` to `Op::RegisterJob`**

In `crates/bee-control/src/kv.rs`, find `Op::RegisterJob`:

```rust
RegisterJob {
    job_id: u32,
    dag_hash: String,
    owner_node: u32,
    /// S29: tenant namespace (`u16`; 0 = global per ADR-0010).
    /// MVP: struct field only; ACL check is 1.x.
    tenant: u16,
},
```

Change to:

```rust
RegisterJob {
    job_id: u32,
    dag_hash: String,
    owner_node: u32,
    /// S29: tenant namespace (`u16`; 0 = global per ADR-0010).
    /// MVP: struct field only; ACL check is 1.x.
    tenant: u16,
    /// S18: cross-Pipeline edges. A Job B with
    /// `CREATE VIEW v AS SELECT ... FROM a.output` (where
    /// `a` is JobId 1) gets `DependencyRecord { upstream_job: 1, stream: "output" }`.
    /// The orchestrator (`evaluate_job_state`) holds B in
    /// `WaitingForUpstream` until upstream Job 1 is `Running`.
    #[serde(default)]
    dependencies: Vec<DependencyRecord>,
},
```

(The `#[serde(default)]` attribute makes this additive — old test code that constructs `Op::RegisterJob { job_id, dag_hash, owner_node, tenant }` without `dependencies` will fail to compile. The plan handles that in Step 1.3.)

- [ ] **Step 1.2: Find all `Op::RegisterJob { ... }` constructors and add `dependencies: Vec::new()`**

Run: `grep -rn "Op::RegisterJob {" crates/ bee/`. For each match, add `dependencies: Vec::new(),` (or the actual list if the caller has computed deps). Likely places:
- `crates/bee-control/src/control_plane.rs` — inside the SM's apply path
- `crates/bee-control/src/deployer.rs` — the Deployer's existing call site (may not exist; check)
- `bee/src/main.rs` — `bee_deploy_local` (S49) — this is where we'll add the dep computation
- `crates/bee-control/tests/*.rs` — several test files; add `dependencies: Vec::new()` to each
- `crates/bee-control/src/lib.rs` — possibly the `ControlPlaneStateMachine::apply_op` test mocks

- [ ] **Step 1.3: Add `detect_cross_pipeline_deps` to `crates/bee-dsl-sql/src/preprocess.rs`**

Find the end of `preprocess_sql_v2` and add a new helper. The function walks the SQL for `<u32>.output` patterns (where `<u32>` is a numeric literal, the JobId of the upstream Job).

```rust
/// S18: scan a SQL text for cross-Pipeline edge references of
/// the form `<job_id>.output` (where `<job_id>` is a numeric
/// literal — the JobId of the upstream Job). Returns one
/// `DependencyRecord` per distinct upstream JobId. The MVP
/// only recognizes the `output` stream; richer stream names
/// (e.g. `binance.btc_5min`) are a S18.x follow-up.
pub fn detect_cross_pipeline_deps(sql: &str) -> Vec<crate::DependencyRecord> {
    use std::collections::BTreeSet;
    // The MVP scanner: a tiny byte-level pass that matches
    // a digit run followed by `.output`. Anything more elaborate
    // (e.g. ignoring `.output` inside string literals or
    // comments) is a S18.x follow-up.
    let bytes = sql.as_bytes();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut out: Vec<crate::DependencyRecord> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Skip ASCII alphabetic / underscore characters (we
        // only want numeric tokens; `binance.output` should
        // not match).
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let n: u32 = std::str::from_utf8(&bytes[start..i])
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            // Skip whitespace + `.output`
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j..j + 7) == Some(b".output") {
                if seen.insert(n) {
                    out.push(crate::DependencyRecord {
                        upstream_job: n,
                        stream: "output".to_string(),
                    });
                }
                i = j + 7;
                continue;
            }
        } else {
            i += 1;
        }
    }
    out
}
```

(Adapt the `crate::DependencyRecord` import path; may be `bee_types::DependencyRecord` or `bee_control::DependencyRecord`.)

- [ ] **Step 1.4: Re-export the new helper**

In `crates/bee-dsl-sql/src/lib.rs`, find the existing `preprocess` re-exports and add:

```rust
pub use preprocess::detect_cross_pipeline_deps;
```

- [ ] **Step 1.5: Build to verify the helper compiles**

Run: `cargo build -p bee-dsl-sql 2>&1 | tail -5`. Expected: clean build.

- [ ] **Step 1.6: Write the unit tests (RED)**

In `crates/bee-dsl-sql/src/preprocess.rs::tests` (or a new module), add:

```rust
#[test]
fn detect_cross_pipeline_deps_recognises_integer_output() {
    let sql = "CREATE VIEW v AS SELECT * FROM 1.output;";
    let deps = detect_cross_pipeline_deps(sql);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].upstream_job, 1);
    assert_eq!(deps[0].stream, "output");
}

#[test]
fn detect_cross_pipeline_deps_dedupes() {
    let sql = "SELECT a.col, b.col FROM 1.output AS a, 1.output AS b;";
    let deps = detect_cross_pipeline_deps(sql);
    assert_eq!(deps.len(), 1, "duplicate 1.output should dedupe");
}

#[test]
fn detect_cross_pipeline_deps_handles_multiple_upstreams() {
    let sql = "SELECT * FROM 1.output JOIN 2.output ON true;";
    let deps = detect_cross_pipeline_deps(sql);
    assert_eq!(deps.len(), 2);
    assert_eq!(deps[0].upstream_job, 1);
    assert_eq!(deps[1].upstream_job, 2);
}

#[test]
fn detect_cross_pipeline_deps_ignores_aliases() {
    // `my_alias` is not numeric — must not match.
    let sql = "SELECT * FROM my_alias;";
    let deps = detect_cross_pipeline_deps(sql);
    assert_eq!(deps.len(), 0);
}

#[test]
fn detect_cross_pipeline_deps_recognises_bare_reference_no_create_view() {
    // The MVP doesn't require `CREATE VIEW`; a bare
    // `FROM <n>.output` works too (the runtime is the
    // orchestrator, not the preprocessor).
    let sql = "SELECT * FROM 7.output;";
    let deps = detect_cross_pipeline_deps(sql);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].upstream_job, 7);
}
```

- [ ] **Step 1.7: Run the tests (GREEN)**

Run: `cargo test -p bee-dsl-sql --lib detect_cross_pipeline 2>&1 | tail -10`. Expected: 5 tests pass.

- [ ] **Step 1.8: Modify `bee_deploy_local` to call the new helper + pass deps**

In `bee/src/main.rs`, find `bee_deploy_local`. After the existing `extract_phase_dag(...)` call (Step 2), add:

```rust
// 2c. S18: detect cross-Pipeline edges (`FROM <job_id>.output`)
//     in the preprocessed SQL. Pass them to RegisterJob so
//     the orchestrator can drive the dependent Job's
//     lifecycle to `WaitingForUpstream` until the upstream
//     Job is `Running`.
let dependencies = bee_dsl_sql::preprocess::detect_cross_pipeline_deps(
    &preprocessed_sql,
);
```

Then update the `RegisterJob` op call (Step 5):

```rust
cp.apply_op(&bee_control::kv::Op::RegisterJob {
    job_id: next_job_id,
    dag_hash: dag.dag_hash.clone(),
    owner_node: leader_id,
    tenant: 0,
    dependencies: dependencies.clone(),
})
.map_err(|e| format!("RegisterJob: {e}"))?;
```

- [ ] **Step 1.9: Add the integration test**

Create `crates/bee-control/tests/cross_pipeline.rs`:

```rust
//! S18: cross-Pipeline edge metadata + lifecycle transition.

use std::time::Duration;

use bee_control::cluster::Cluster;
use bee_control::cluster::ClusterConfig;
use bee_control::control_plane::JobLifecycleState;
use bee_control::deployer::{Deployer, DeployerConfig, Edge, HandlerKind, Pipeline, TaskSpec};
use bee_control::kv::Op;
use bee_control::raft::cluster::ClusterConfig as RaftConfig;

fn started(id: u32) -> TaskSpec {
    TaskSpec {
        task_id: id,
        phase_id: 0,
        handler_kind: HandlerKind::Started { tag: format!("T{id}") },
        cpu_millicores: 100,
        mem_mb: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_pipeline_dep_blocks_lifecycle_until_upstream_runs() {
    // 1. Spin up a 3-Node in-process cluster.
    let cluster = Cluster::new(ClusterConfig::default()).await;
    cluster
        .wait_for_leader(Duration::from_secs(3))
        .await
        .expect("leader elected");
    let leader = cluster.leader().await.unwrap();

    // 2. Register Job A (upstream) + Job B (downstream that
    //    references A via `7.output`).
    //    We pass JobIds manually so B can reference A's id.
    cluster
        .submit(leader, Op::RegisterJob { job_id: 7, dag_hash: "a".into(), owner_node: leader, tenant: 0, dependencies: vec![] })
        .await
        .unwrap();
    cluster
        .submit(leader, Op::RegisterJob {
            job_id: 9,
            dag_hash: "b".into(),
            owner_node: leader,
            tenant: 0,
            dependencies: vec![bee_control::kv::DependencyRecord {
                upstream_job: 7,
                stream: "output".into(),
            }],
        })
        .await
        .unwrap();

    // 3. Job B should be in `WaitingForUpstream` (A is still
    //    `Pending`).
    for (_, handle) in cluster.nodes() {
        if !cluster.is_alive(handle.id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        if let Some(b) = cp.jobs.get(&9) {
            assert_eq!(b.lifecycle, JobLifecycleState::WaitingForUpstream,
                "B must wait for A");
            if let Some(a) = cp.jobs.get(&7) {
                assert_eq!(a.lifecycle, JobLifecycleState::Pending);
            }
        }
    }

    // 4. Mark A as Running. B should now transition to Running.
    cluster
        .submit(leader, Op::UpdateJobLifecycle {
            job_id: 7,
            state: JobLifecycleState::Running,
        })
        .await
        .unwrap();

    for (_, handle) in cluster.nodes() {
        if !cluster.is_alive(handle.id) {
            continue;
        }
        let cp = handle.cp.lock().await;
        if let Some(b) = cp.jobs.get(&9) {
            assert_eq!(b.lifecycle, JobLifecycleState::Running,
                "B should now be Running after A became Running");
        }
    }
}
```

(Adapt to the actual `Op::RegisterJob` and `Op::UpdateJobLifecycle` field shapes; my S18 step 1.1 added `dependencies: Vec<DependencyRecord>`. The `JobLifecycleState` and `DependencyRecord` are in `bee_control` crate.)

- [ ] **Step 1.10: Run the integration test**

Run: `cargo test -p bee-control --test cross_pipeline 2>&1 | tail -10`. Expected: PASS.

- [ ] **Step 1.11: Run full workspace tests for regression**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 431+ failed: 0 ignored: 5`.

- [ ] **Step 1.12: Commit**

```bash
git add crates/bee-control/src/kv.rs crates/bee-dsl-sql/src/preprocess.rs crates/bee-dsl-sql/src/lib.rs bee/src/main.rs crates/bee-control/tests/cross_pipeline.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S18: CREATE VIEW v AS SELECT ... FROM <job_id>.output SQL syntax; detect_cross_pipeline_deps helper; Op::RegisterJob gets dependencies field; integration test for WaitingForUpstream → Running lifecycle transition"
```

---

## Task 2: Final verification + push

- [ ] **Step 2.1: Update S18 spec acceptance criteria**

Edit `docs/superpowers/specs/2026-07-17-s18-cross-pipeline-edge-sql-design.md` and flip the `[ ]` to `[x]`. Commit:

```bash
git add docs/superpowers/specs/2026-07-17-s18-cross-pipeline-edge-sql-design.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S18: flip acceptance criteria to [x]"
```

- [ ] **Step 2.2: Update `docs/stories.md` S18 acceptance criteria**

Find the S18 section in stories.md and flip the relevant `[ ]` to `[x]`. Commit:

```bash
git add docs/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "stories.md: S18 acceptance criteria flipped"
```

- [ ] **Step 2.3: Push to remote**

```bash
git push origin main
```

---

## Self-Review

**1. Spec coverage:** Walked the S18 spec's in-scope items:
- `CREATE VIEW v AS SELECT ... FROM <job_id>.output` SQL syntax: Task 1 Step 1.3 ✓
- `JobRecord.dependencies` populated at deploy: Task 1 Step 1.8 ✓
- B's lifecycle `WaitingForUpstream → Running`: Task 1 Step 1.9 ✓
- Tests (unit + integration): Task 1 Steps 1.6, 1.9 ✓

**2. Placeholder scan:** No TBD / TODO.

**3. Type consistency:** `Op::RegisterJob { dependencies: Vec<DependencyRecord> }` — same `DependencyRecord` already used in `JobRecord.dependencies`. `detect_cross_pipeline_deps` returns `Vec<DependencyRecord>`. Direct match.

**4. Ambiguity check:** Each test specifies concrete input (specific JobIds + SQL text) + concrete expected output (deps list with specific upstream_job values). The integration test specifies the full lifecycle transition.

---

## Estimated Total

- 2 Tasks
- 3 commits (impl + criteria flip + stories flip)
- ~150-200 LOC net change (mostly `preprocess.rs` + the test files)
- Estimated wall-clock: 1-2 hours