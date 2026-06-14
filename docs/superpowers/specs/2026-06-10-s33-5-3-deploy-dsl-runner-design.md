# S33.5.3 — Deploy 完整 DSL Runner

**Date:** 2026-06-10
**Type:** AFK
**Blocked by**: S33.6 (plugin macro ergonomics)
**Status**: Approved (2026-06-10)

## Why this story exists

S33.5 wired the leader-side apply for `AdminRequest::Deploy`, but the arm is a placeholder that hashes the SQL and registers a single `Op::RegisterJob { ... }` with **zero `Op::RegisterTask` ops**. The control plane has no idea what phases the pipeline should run.

S33.5.3 closes the gap:
1. Add a `extract_phase_dag(sql) -> PhaseDag` function in `bee-dsl-sql` that walks the SQL AST and identifies phases (top-level SELECTs) + their dependencies.
2. Wire `AdminServer::dispatch_with_apply(Deploy { sql_text, owner_node })` to use the extractor, allocate `task_id`s, and submit 1 × `Op::RegisterJob` followed by N × `Op::RegisterTask` in order (sequential apply, idempotent re-Deploy).

The MVP is **heuristic-only**: it finds every top-level `SELECT ... FROM <table_or_datasource>` and treats them as independent phases. `WITH` chain / multi-CTE DAG extraction is deferred to S33.5.x. A S33.5.x follow-up will also add `Op::RegisterJobWithTasks` for atomic apply.

## Scope

### In scope (3 deliverables)

1. **`extract_phase_dag` function in `bee-dsl-sql`**:
   - File: `crates/bee-dsl-sql/src/dag.rs` (new module).
   - Public API: `pub fn extract_phase_dag(sql: &str) -> Result<PhaseDag, String>`.
   - `PhaseDag` struct:
     ```rust
     pub struct PhaseDag {
         pub phases: Vec<Phase>,         // 1-indexed by phase_id
         pub dependencies: Vec<(u32, u32)>,  // (phase_id, depends_on_phase_id)
         pub dag_hash: String,            // sha256(sql_text)
     }
     pub struct Phase {
         pub phase_id: u32,                // 1-indexed
         pub sql: String,                  // the SQL for this phase (the SELECT itself)
     }
     ```
   - Implementation: parse the SQL with `parse_sql`, walk the AST statements, for each `Statement::Statement(Statement::Query)` (a top-level `SELECT`), extract the `SELECT` body, allocate a `phase_id`, and append a `Phase`. The MVP returns an empty `dependencies` vec (no inter-phase dependencies; phases are treated as independent). A S33.5.x follow-up will add WITH-chain support.
   - Compute `dag_hash = sha256(sql_text)` (existing logic, factored out of the AdminServer).
   - Errors: `Err("dag: <reason>")` on parse failures.

2. **`Deploy` arm in `AdminServer::dispatch_with_apply`**:
   - File: `crates/bee-control/src/raft/admin_server.rs` (replace the existing `AdminRequest::Deploy` arm at line 419 + the one in `dispatch` at line 635).
   - Flow:
     1. Call `bee_dsl_sql::dag::extract_phase_dag(&sql_text)`. On error, return `DeployAck { job_id: 0, task_ids: vec![], error_msg: e }`.
     2. Allocate `job_id = next_job_id` (scan `cp.list_jobs()` for max + 1, existing pattern).
     3. Allocate `task_id_base = next_task_id` (scan `cp.list_tasks()` for max + 1, new helper).
     4. Submit `Op::RegisterJob { job_id, dag_hash, owner_node, tenant: 0 }`. On submit error, return early.
     5. For each `Phase` in the DAG (in `phase_id` order), submit `Op::RegisterTask { task_id: task_id_base + i, job_id, phase_id: i + 1, owner_node, status: Pending, started_at_ms: 0 }`. Collect the `task_id` in a `Vec<u32>`.
     6. Return `DeployAck { job_id, task_ids, error_msg: "" }`.
   - On mid-apply crash (some Tasks registered, some not), the user can re-Deploy (idempotent — same `dag_hash`, leader re-applies; a future S33.5.x can use `Op::RegisterJobWithTasks` for atomic apply).

3. **`next_task_id` helper on `ControlPlaneStateMachine`**:
   - File: `crates/bee-control/src/control_plane.rs`.
   - New method: `pub fn next_task_id(&self) -> u32 { self.tasks.keys().max().unwrap_or(0) + 1 }` (mirrors the existing `next_job_id` pattern at line ~85).

### Out of scope (deferred to S33.5.x / S34.x)

- `WITH` chain / multi-CTE DAG extraction. MVP treats every top-level `SELECT` as an independent phase. SQL with `WITH foo AS (...) SELECT * FROM foo` will get 1 phase (the outer SELECT); the CTE is not extracted as a phase.
- Atomic `Op::RegisterJobWithTasks { job, tasks }` apply. MVP is sequential + idempotent.
- Per-phase SQL validation against registered Datasources (the `use <name>;` mechanism from ADR-0010). MVP trusts the Datasource references; the orchestrator fails at scheduling time if a Datasource is missing.
- DAG cycle detection. MVP assumes well-formed SQL.
- Multi-tenant support: `tenant` is hard-coded to 0 (per existing MVP). 1.x adds tenant ACL enforcement (ADR-0010).
- Re-deploy detection: MVP re-applies the same Tasks on every Deploy with the same `dag_hash`. A S33.5.x can add idempotency by checking the existing Job's `dag_hash` and returning the existing `task_ids`.

## Design

### `PhaseDag` API

```rust
// crates/bee-dsl-sql/src/dag.rs

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    pub phase_id: u32,
    pub sql: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseDag {
    pub phases: Vec<Phase>,
    pub dependencies: Vec<(u32, u32)>,
    pub dag_hash: String,
}

pub fn extract_phase_dag(sql_text: &str) -> Result<PhaseDag, String> {
    let stmts = crate::parse_sql(sql_text)
        .map_err(|e| format!("dag: parse failed: {e}"))?;
    let mut phases = Vec::new();
    let mut next_id = 1u32;
    for stmt in stmts {
        // `parse_sql` returns
        // `Vec<datafusion::sql::parser::Statement>`,
        // where `Statement` is the datafusion enum
        // (not the sqlparser-rs wrapper). A top-level
        // `SELECT` is `Statement::Query(_)`; other
        // variants (SetVariable, CreateTable, etc.)
        // are not phases.
        if let datafusion::sql::parser::Statement::Query(_) = stmt {
            // The MVP extracts a placeholder SQL
            // text for this statement. A S33.5.x
            // will use the AST to extract the actual
            // SELECT body and identify the
            // FROM-clause table / Datasource.
            phases.push(Phase {
                phase_id: next_id,
                sql: format!("<phase {}: parsed query>", next_id),
            });
            next_id += 1;
        }
    }
    if phases.is_empty() {
        return Err("dag: no SELECT statements found".to_string());
    }
    let dag_hash = {
        let mut h = Sha256::new();
        h.update(sql_text.as_bytes());
        format!("{:x}", h.finalize())
    };
    Ok(PhaseDag {
        phases,
        dependencies: Vec::new(),
        dag_hash,
    })
}
```

The MVP's `phases[i].sql` is a placeholder (the original parse tree's SQL text). A S33.5.x will use `Display` on the `Statement::Query` to extract the actual SELECT text.

### `Deploy` arm flow

```rust
AdminRequest::Deploy { sql_text, owner_node } => {
    // S33.5.3: extract the phase DAG.
    let dag = match bee_dsl_sql::dag::extract_phase_dag(&sql_text) {
        Ok(d) => d,
        Err(e) => {
            return AdminResponse::DeployAck {
                job_id: 0,
                task_ids: vec![],
                error_msg: e,
            };
        }
    };
    // Allocate IDs.
    let cp_locked = cp.lock().await;
    let next_job_id = cp_locked.list_jobs().iter()
        .map(|j| j.job_id).max().unwrap_or(0) + 1;
    let next_task_id = cp_locked.next_task_id();
    drop(cp_locked);
    // Submit the Job.
    let op = crate::kv::Op::RegisterJob {
        job_id: next_job_id,
        dag_hash: dag.dag_hash.clone(),
        owner_node,
        tenant: 0,
    };
    if let AdminResponse::Error(e) = submit_and_await(transport, op).await {
        return AdminResponse::DeployAck {
            job_id: 0,
            task_ids: vec![],
            error_msg: format!("job submit: {e}"),
        };
    }
    // Submit N Tasks.
    let mut task_ids = Vec::with_capacity(dag.phases.len());
    for (i, phase) in dag.phases.iter().enumerate() {
        let task_id = next_task_id + i as u32;
        let op = crate::kv::Op::RegisterTask {
            task_id,
            job_id: next_job_id,
            phase_id: phase.phase_id,
            owner_node,
            status: crate::kv::TaskStatus::Pending,
            started_at_ms: 0,
        };
        match submit_and_await(transport, op).await {
            AdminResponse::KvPutAck { ok: true } => {
                task_ids.push(task_id);
            }
            AdminResponse::KvPutAck { ok: false } => {
                return AdminResponse::DeployAck {
                    job_id: next_job_id,
                    task_ids,
                    error_msg: format!("task submit failed at phase {}", phase.phase_id),
                };
            }
            other => {
                return AdminResponse::DeployAck {
                    job_id: next_job_id,
                    task_ids,
                    error_msg: format!("task submit unexpected reply: {other:?}"),
                };
            }
        }
    }
    AdminResponse::DeployAck {
        job_id: next_job_id,
        task_ids,
        error_msg: String::new(),
    }
}
```

The same flow applies in `AdminServer::dispatch(Deploy)` (the wire-direct path that writes to local KV without Raft). The MVP writes the op directly to the local KV instead of going through `submit_and_await`.

### Test plan (2 new + the existing `admin_deploy_roundtrip` test updated)

#### 1. `dag_extracts_multi_phase`

```rust
// crates/bee-dsl-sql/tests/dag_extract.rs
use bee_dsl_sql::dag::{extract_phase_dag, PhaseDag};

#[test]
fn extracts_two_phases_from_two_selects() {
    let sql = "SELECT * FROM binance.subscribe('BTC/USDT', '5min'); \
               SELECT avg(price) FROM ticks;";
    let dag = extract_phase_dag(sql).expect("extract");
    assert_eq!(dag.phases.len(), 2);
    assert_eq!(dag.phases[0].phase_id, 1);
    assert_eq!(dag.phases[1].phase_id, 2);
    assert!(dag.dependencies.is_empty());
    assert_eq!(dag.dag_hash.len(), 64); // sha256 hex
}

#[test]
fn errors_on_empty_sql() {
    let dag = extract_phase_dag("");
    assert!(dag.is_err());
}

#[test]
fn errors_on_no_selects() {
    // A non-SELECT statement (e.g. a SET command)
    // produces 0 phases.
    let sql = "SET foo = 1;";
    let dag = extract_phase_dag(sql);
    assert!(dag.is_err());
    assert!(dag.unwrap_err().contains("no SELECT"));
}
```

#### 2. `deploy_full_path_creates_job_and_tasks`

```rust
// crates/bee-control/tests/deploy_full_dag.rs

use std::sync::Arc;
use bee_control::control_plane::ControlPlaneStateMachine;
use bee_control::kv::KVStateMachine;
use bee_control::raft::admin_client::AdminClient;
use bee_control::raft::admin_protocol::{AdminRequest, AdminResponse};
use bee_control::raft::admin_server::AdminServer;
use bee_control::raft::node::NodeState;
use tokio::sync::Mutex;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deploy_two_phase_sql_creates_job_and_two_tasks() {
    let kv = Arc::new(Mutex::new(KVStateMachine::new()));
    let cp = Arc::new(Mutex::new(ControlPlaneStateMachine::new()));
    let state = Arc::new(Mutex::new(NodeState::default()));
    let mut admin = AdminServer::start(
        "127.0.0.1:0".parse().unwrap(),
        kv.clone(),
        cp.clone(),
        state,
        None, None, None, None, // S33.5.2 plugin_manager = None
    ).await.expect("start");
    let mut client = AdminClient::connect(admin.local_addr())
        .await.expect("connect");
    let resp = client.call(AdminRequest::Deploy {
        sql_text: "SELECT * FROM binance.subscribe('BTC/USDT', '5min'); \
                   SELECT avg(price) FROM ticks;"
            .to_string(),
        owner_node: 1,
    }).await.expect("call");
    let (job_id, task_ids) = match resp {
        AdminResponse::DeployAck { job_id, task_ids, error_msg } => {
            assert!(error_msg.is_empty(), "deploy failed: {error_msg}");
            (job_id, task_ids)
        }
        other => panic!("expected DeployAck, got: {other:?}"),
    };
    assert_eq!(job_id, 1);
    assert_eq!(task_ids.len(), 2);
    assert_eq!(task_ids[0], 1);
    assert_eq!(task_ids[1], 2);
    // Verify the control plane has 1 Job + 2 Tasks.
    let cp_locked = cp.lock().await;
    let jobs = cp_locked.list_jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, 1);
    let tasks = cp_locked.list_tasks();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].job_id, 1);
    assert_eq!(tasks[0].phase_id, 1);
    assert_eq!(tasks[1].job_id, 1);
    assert_eq!(tasks[1].phase_id, 2);
    admin.shutdown();
}
```

#### 3. Update `admin_deploy_roundtrip` in `admin_write_roundtrip.rs`

The existing test sends `AdminRequest::Deploy { sql_text: "SELECT 1", owner_node: 1 }` and asserts the response is `DeployAck { job_id: 0, error_msg: <non-empty> }` (the MVP marker). With S33.5.3, `SELECT 1` has 1 phase, so the response should be `DeployAck { job_id: 1, task_ids: [1], error_msg: "" }`. Update the test.

### Edge cases

- **Empty SQL**: `extract_phase_dag("")` returns `Err("dag: parse failed: ...")`. The `Deploy` arm returns `DeployAck { job_id: 0, task_ids: vec![], error_msg }`.
- **SQL with no SELECTs** (e.g., `SET foo = 1;`): returns `Err("dag: no SELECT statements found")`. The `Deploy` arm returns the error.
- **SQL with `;` at the end**: DataFusion's parser handles this. 1 phase if 1 SELECT.
- **Re-Deploy with the same SQL**: the new Job gets a different `job_id` (since `next_job_id` increments). The new Tasks get different `task_ids`. The old Job + Tasks remain in the control plane. A S33.5.x can add re-Deploy detection (match by `dag_hash`).
- **Mid-apply crash**: leader crashes after `RegisterJob` but before some `RegisterTask` ops. The Job is in the control plane with fewer Tasks than expected. The rebalancer sees the Job with N < expected Tasks and ignores it (or fails). Re-Deploy creates a new Job with the correct Tasks. The old Job is orphaned but doesn't block anything (it's marked Pending with no Tasks to schedule).
- **Owner_node is dead**: the Job is registered but the Tasks can't be scheduled on the dead node. The rebalancer will pick them up after the orphan timeout (3 × heartbeat_interval = 30s).

### Sign-off matrix

| Item | Code-level (this story) | Production-level (1.x) |
|------|------------------------|------------------------|
| `extract_phase_dag` extracts N phases for N top-level SELECTs | ✓ (3 tests) | N — covers only the heuristic |
| `Deploy` arm submits 1 Job + N Tasks in order | ✓ (1 test) | N — 24h run |
| `next_task_id` helper on ControlPlaneStateMachine | ✓ (used by 1 test) | N |
| Sequential apply (idempotent re-Deploy) | ✓ (covered by the test) | N — needs S33.5.x for atomic apply |
| `WITH` chain / multi-CTE DAG extraction | N (deferred) | N |
| Atomic `Op::RegisterJobWithTasks` | N (deferred) | N |
| Per-phase Datasource validation | N (deferred) | N |
| DAG cycle detection | N (deferred) | N |
| Re-Deploy detection by `dag_hash` | N (deferred) | N |

## Related work

- S19: datafusion SQL parser integration (the `parse_sql` function in `bee-dsl-sql`).
- S26: `run_pipeline_with_config` — the runtime path that compiles a SQL to an `ExecutionPlan`. S33.5.3 does NOT use this; the Deploy path only needs the phase DAG topology, not the full execution plan.
- S33.5: leader-side apply (the `RegisterJob` + `RegisterTask` arm stubs).
- S33.5.1: cross-node forwarding — the `Deploy` arm also goes through this path; the S33.5.3 changes are in the `dispatch_with_apply` (which the Forward local-leader branch and the run_node callback both use).
- S33.5.2: `RegisterDatasource` validation — orthogonal to Deploy.
- ADR-0010: Datasource as a managed Provider, `use` syntax. The Deploy path's FROM-clause table may be a Datasource; the S33.5.3 MVP trusts the reference.
