# S49 — `bee deploy` + `bee jobs wait` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `bee deploy <sql_file>` (local mode) + `bee jobs wait --job <id> --until done` (local mode) subcommands. These unlock `scripts/demo-perf.sh` (S45) and the S33.1 multi-node demo.

**Architecture:** Two new helper functions in `bee/src/main.rs`. `bee_deploy_local` reads the SQL file, calls `extract_phase_dag`, allocates JobId + TaskIds by scanning the local CP's `list_jobs()` / `list_tasks()`, submits `RegisterJob` + N×`RegisterTask` ops via `Cluster`'s leader. `bee_jobs_wait` polls the local CP every 200ms for the Job's lifecycle state until terminal or timeout.

**Tech Stack:** Rust, tokio (sleep + interval), `extract_phase_dag` (already in `crates/bee-dsl-sql/src/dag.rs`).

---

## File Structure

| File | Action | Purpose |
|---|---|---|
| `bee/src/main.rs` | Modify | Add `bee_deploy_local` + `bee_jobs_wait_local` + CLI dispatch |

1 Task (small).

---

## Task 1: `bee deploy` (local) + `bee jobs wait` (local)

**Files:**
- Modify: `bee/src/main.rs` (add 2 helper functions + CLI dispatch)

- [ ] **Step 1.1: Read `extract_phase_dag` + `ControlPlane` API**

Run:

```bash
grep -n "pub fn extract_phase_dag\|pub struct PhaseDag\|pub struct Phase" crates/bee-dsl-sql/src/dag.rs | head -10
grep -n "pub fn list_jobs\|pub fn list_tasks\|pub fn apply_op\|pub fn get_job" crates/bee-control/src/control_plane.rs | head -10
grep -n "pub fn submit\|pub fn nodes\|pub fn is_alive\|pub fn leader_id" crates/bee-control/src/raft/cluster.rs | head -10
```

Note the exact signatures. The plan below assumes:
- `extract_phase_dag(sql: &str) -> Result<PhaseDag, String>`
- `PhaseDag` has `dag_hash: String` and `phases: Vec<Phase>`
- `Phase` has `phase_id: u32` (1-indexed within the DAG)
- `ControlPlane::list_jobs() -> Vec<JobRecord>`
- `ControlPlane::list_tasks() -> Vec<TaskRecord>`
- `ControlPlane::apply_op(&Op)` (via the leader handle; see `run_jobs_cli` for the pattern)
- `Cluster::leader_id()` + `Cluster::nodes()` give access to nodes

- [ ] **Step 1.2: Write the failing test (RED)**

In `bee/src/main.rs`, find the existing `#[cfg(test)] mod tests` (or create one). Add:

```rust
#[tokio::test(flavor = "current_thread")]
async fn extract_phase_dag_simple_select_produces_one_phase() {
    // Sanity check that the S33.5.3 extract_phase_dag is reachable.
    use bee_dsl_sql::dag::extract_phase_dag;
    let sql = "SELECT n FROM generate_series(1, 3) AS t(n);";
    let dag = extract_phase_dag(sql).expect("simple SELECT must parse");
    assert_eq!(dag.phases.len(), 1, "got: {dag:?}");
    assert_eq!(dag.phases[0].phase_id, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn extract_phase_dag_rejects_empty_sql() {
    use bee_dsl_sql::dag::extract_phase_dag;
    let result = extract_phase_dag("");
    assert!(result.is_err());
}
```

- [ ] **Step 1.3: Run the tests (verify they pass — the library already supports this)**

Run: `cargo test -p bee --bin bee extract_phase_dag 2>&1 | tail -5`. Expected: 2 tests pass.

If `bee` is a binary crate, the tests live in `bee/src/main.rs` and the test invocation is different. If the path is awkward, skip these tests — they're just sanity checks for the integration; the real acceptance is at the CLI level.

- [ ] **Step 1.4: Implement `bee_deploy_local`**

In `bee/src/main.rs`, add this function (near the other CLI helper functions like `run_jobs_cli`):

```rust
/// S49: `bee deploy <sql_file>` (local mode). Reads the SQL,
/// extracts the DAG, registers a Job + N Tasks in the local
/// ControlPlane. Returns the new JobId.
///
/// On failure: returns Err with a human-readable message; the
/// CLI prints it and exits non-zero.
async fn bee_deploy_local(cluster: &Cluster, sql_path: &str) -> Result<u32, String> {
    use bee_control::control_plane::{ControlPlaneStateMachine, JobLifecycleState};
    use bee_control::raft::cluster::ClusterConfig;
    use bee_dsl_sql::dag::extract_phase_dag;
    use bee_plugin_sdk::compute_plugin_id;

    // 1. Read the SQL file.
    let sql_text = std::fs::read_to_string(sql_path)
        .map_err(|e| format!("read {sql_path}: {e}"))?;

    // 2. Extract the DAG.
    let dag = extract_phase_dag(&sql_text)
        .map_err(|e| format!("extract_phase_dag: {e}"))?;

    // 3. Allocate IDs by scanning existing Jobs + Tasks.
    let leader_id = cluster
        .leader_id()
        .await
        .ok_or_else(|| "no leader elected".to_string())?;
    let leader_handle = cluster
        .nodes()
        .into_iter()
        .find(|(id, _)| *id == leader_id)
        .map(|(_, h)| h)
        .ok_or_else(|| format!("leader node {leader_id} handle not found"))?;
    let cp = leader_handle.cp.lock().await;

    let next_job_id = cp
        .list_jobs()
        .iter()
        .map(|j| j.job_id)
        .max()
        .map(|m| m + 1)
        .unwrap_or(1);
    let next_task_id = cp
        .list_tasks()
        .iter()
        .map(|t| t.task_id)
        .max()
        .map(|m| m + 1)
        .unwrap_or(1);

    // 4. Submit RegisterJob.
    cp.apply_op(&bee_control::kv::Op::RegisterJob {
        job_id: next_job_id,
        dag_hash: dag.dag_hash.clone(),
        owner_node: leader_id,
        tenant: 0,
    })
    .map_err(|e| format!("RegisterJob: {e}"))?;

    // 5. Submit N× RegisterTask. phase_id is 1-indexed per PhaseDag.
    for (i, phase) in dag.phases.iter().enumerate() {
        let task_id = next_task_id + i as u32;
        cp.apply_op(&bee_control::kv::Op::RegisterTask {
            task_id,
            job_id: next_job_id,
            phase_id: phase.phase_id,
            owner_node: leader_id,
            status: bee_control::kv::TaskStatus::Pending,
        })
        .map_err(|e| format!("RegisterTask[{i}]: {e}"))?;
    }

    Ok(next_job_id)
}
```

(Adapt the exact import paths if they differ — check `bee/src/main.rs:45-52` for the existing imports.)

- [ ] **Step 1.5: Wire `deploy` into the main dispatch (local mode)**

In `bee/src/main.rs`, find the `Some("deploy") =>` branch (around line 1023). Currently it sends an admin RPC. Refactor it to:

- If `--connect <addr>` is in args (remote mode): keep the existing admin RPC path
- Else (local mode): create a local `Cluster::new(ClusterConfig::default())`, wait for leader, call `bee_deploy_local(&cluster, sql_path)`

Concretely, wrap the existing `Some("deploy") =>` arm's logic in an `if args.contains("--connect")` check, and add an `else` branch that does the local path:

```rust
Some("deploy") => {
    let sql_path = args.get(2).ok_or_else(|| "deploy requires <sql_file>".to_string())?;
    if args.iter().any(|a| a == "--connect") {
        // Remote mode (existing AdminRequest::Deploy path).
        let addr = /* parse --connect <addr> from args */;
        // ... existing admin client code ...
    } else {
        // Local mode (S49).
        let cluster = Cluster::new(ClusterConfig::default()).await;
        cluster
            .wait_for_leader(Duration::from_secs(3))
            .await
            .ok_or_else(|| "no leader elected".to_string())?;
        match bee_deploy_local(&cluster, sql_path).await {
            Ok(job_id) => {
                println!("deployed as job {job_id}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}: deploy failed: {}", PKG_NAME, e);
                ExitCode::from(1)
            }
        }
    }
}
```

(Adjust the local mode to match the function's return type / `ExitCode` shape of the surrounding `match`.)

- [ ] **Step 1.6: Implement `bee_jobs_wait_local`**

In `bee/src/main.rs`, add this helper function:

```rust
/// S49: `bee jobs wait --job <id> --until done` (local mode).
/// Polls the local ControlPlane for the Job's lifecycle state
/// every 200ms. Returns when the Job reaches a terminal state
/// (Completed / Failed / Revoked) or when the timeout expires.
async fn bee_jobs_wait_local(
    cluster: &Cluster,
    job_id: u32,
    timeout_secs: u64,
) -> Result<&'static str, String> {
    use bee_control::control_plane::JobLifecycleState;

    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(timeout_secs);
    let mut iterations = 0u64;
    loop {
        // Read from any alive node's CP (consistent within a single
        // 3-node in-process cluster since they share the same SM).
        for (_id, handle) in cluster.nodes() {
            if cluster.is_alive(_id) {
                let cp = handle.cp.lock().await;
                if let Some(job) = cp.get_job(job_id) {
                    let state = &job.lifecycle;
                    if matches!(
                        state,
                        JobLifecycleState::Completed
                            | JobLifecycleState::Failed
                            | JobLifecycleState::Revoked
                    ) {
                        return Ok(match state {
                            JobLifecycleState::Completed => "completed",
                            JobLifecycleState::Failed => "failed",
                            JobLifecycleState::Revoked => "revoked",
                            _ => unreachable!(),
                        });
                    }
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout after {timeout_secs}s waiting for job {job_id} to reach a terminal state"
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        iterations += 1;
    }
}
```

(Adapt to use the actual `ControlPlane::get_job` API — check the existing `control_plane.rs` for the right method name; it may be `get_job_via_apply` or similar.)

- [ ] **Step 1.7: Wire `wait` into `run_jobs_cli`**

In `bee/src/main.rs`, find `run_jobs_cli`. Add a new arm in its inner `match subcommand`:

```rust
Some("wait") => {
    let id: u32 = job_id_arg
        .ok_or_else(|| "jobs wait requires <job_id>".to_string())?
        .parse()
        .map_err(|e| format!("invalid job_id: {e}"))?;
    let timeout_secs: u64 = args
        .iter()
        .position(|a| a == "--timeout-secs")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(300); // 5 minutes default
    match bee_jobs_wait_local(&cluster, id, timeout_secs).await {
        Ok(state) => {
            println!("job {id} reached {state}");
            return Ok(());
        }
        Err(e) => return Err(e),
    }
}
```

(Adjust the parameter passing — `run_jobs_cli` currently takes `subcommand` and `job_id_arg` but not the full `args` list. May need to update the signature.)

- [ ] **Step 1.8: Build the bee binary**

Run: `cargo build -p bee 2>&1 | tail -5`. Expected: clean build. Fix any signature mismatches.

- [ ] **Step 1.9: Manual end-to-end smoke test**

```bash
cargo run -p bee -- deploy examples/performance/prime_sieve.sql 2>&1 | tail -3
cargo run -p bee -- jobs list 2>&1 | tail -10
cargo run -p bee -- jobs inspect 1 2>&1 | tail -10
```

Expected: deploy prints `deployed as job 1`; `jobs list` shows the new job; `jobs inspect 1` shows the DAG header + tasks.

Then:

```bash
cargo run -p bee -- jobs wait --job 1 --until done --timeout-secs 5 2>&1 | tail -3
```

Expected: times out after 5 seconds with `timeout after 5s waiting for job 1...` (since no worker is running the job, it stays Pending — that's the MVP contract per spec).

- [ ] **Step 1.10: Run full workspace tests for regression check**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 429 failed: 0` (no new tests in S49; the existing 429 baseline preserved).

- [ ] **Step 1.11: Commit**

```bash
git add bee/src/main.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S49: bee deploy (local) + bee jobs wait (local) — unlock demo-perf.sh and S33.1 multi-node demo"
```

---

## Task 2: Final verification + push

- [ ] **Step 2.1: Update S49 spec acceptance criteria**

Edit `docs/superpowers/specs/2026-07-17-s49-bee-deploy-wait-design.md` and flip the `[ ]` to `[x]` for each criterion that the implementation actually satisfies. Some may stay `[ ]` (e.g., "scripts/demo-perf.sh end-to-end" if the script itself needs updates — that's a follow-up; S49 only adds the subcommands).

Commit:

```bash
git add docs/superpowers/specs/2026-07-17-s49-bee-deploy-wait-design.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S49: flip acceptance criteria to [x]"
```

- [ ] **Step 2.2: Add S49 to `docs/stories.md` + flip its criteria**

Find the next free S number after the current highest (S49 is the new one). Edit `docs/stories.md` to insert a `### S49` entry mirroring the spec. Flip criteria to `[x]` after.

Commit:

```bash
git add docs/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "stories.md: add S49 story entry"
```

- [ ] **Step 2.3: Push to remote**

```bash
git push origin main
```

---

## Self-Review

**1. Spec coverage:** Walked the S49 spec's in-scope items:
- `bee deploy <sql_file>` (local): Task 1 Steps 1.4–1.5 ✓
- `bee jobs wait`: Task 1 Steps 1.6–1.7 ✓
- Demo smoke test: Task 1 Step 1.9 ✓

**2. Placeholder scan:** Searched for "TBD" / "TODO" — only one (the S49.x follow-up for real worker execution, which is intentionally out of scope).

**3. Type consistency:**
- `extract_phase_dag(sql: &str) -> Result<PhaseDag, String>` (S33.5.3 deliverable; consistent across the plan).
- `Phase::phase_id: u32` (1-indexed within DAG).
- `ControlPlane::list_jobs()` / `list_tasks()` return `Vec<JobRecord>` / `Vec<TaskRecord>` with `job_id` / `task_id` fields (used for max-scan ID allocation).

**4. Ambiguity check:** The plan's code samples show concrete signatures. If the actual API differs slightly (e.g., `get_job` is named differently), the engineer adapts — the plan's intent is unambiguous.

---

## Estimated Total

- 2 Tasks
- 4-6 commits (Task 1 = 1, Task 2 = 3)
- ~100-150 LOC net change (mostly in `bee/src/main.rs`)
- Estimated wall-clock: 1-1.5 hours