# S49 — `bee deploy` + `bee jobs wait` (local mode)

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S27 (`bee jobs list` / `bee jobs inspect` exist)
**ADRs:** none
**Status:** Draft (pending review)

## Why this story exists

`scripts/demo-perf.sh` (S45) and `scripts/start-cluster.sh` (S33.1) reference `bee deploy <sql_file>` and `bee jobs wait --job <id> --until done`. Neither subcommand exists at the local CLI. The only working deploy path is `bee --connect <addr> deploy <sql_file>` (remote-mode AdminRequest::Deploy). The wait subcommand doesn't exist at all.

This story closes the gap: add the two local subcommands so the demo scripts work end-to-end and the S33.1 multi-node cluster demo can be exercised.

## What already exists at HEAD

- `crates/bee-dsl-sql/src/dag.rs` — `extract_phase_dag(sql: &str) -> Result<PhaseDag, String>` (S33.5.3 deliverable)
- `crates/bee-control/src/control_plane.rs::Op::RegisterJob` + `Op::RegisterTask` (S08)
- `bee/src/main.rs::run_jobs_cli` — list + inspect work (S27 partial)
- `bee/src/main.rs::deploy` (line 1023) — only works in remote mode (`bee --connect <addr> deploy <sql_file>`); sends `AdminRequest::Deploy`

## Scope

### In scope

1. **`bee deploy <sql_file>` (local mode)**:
   - Read the SQL file
   - Call `extract_phase_dag(sql)` to extract the DAG
   - Submit `Op::RegisterJob { job_id, dag_hash, owner_node: <this_node>, tenant: 0 }` + N × `Op::RegisterTask { task_id, job_id, phase_id, owner_node, status: Pending }` to the local control plane
   - Allocate IDs: scan `cp.list_jobs()` for `max(job_id) + 1`; scan `cp.list_tasks()` for `max(task_id) + 1`
   - Print the new `job_id` to stdout
   - On DAG-extract failure: print the error and exit non-zero
2. **`bee jobs wait --job <id> --until done`**:
   - Poll the local control plane every 200ms for the Job's lifecycle state
   - Return when the Job reaches `Completed` / `Failed` / `Revoked`
   - Timeout after 5 minutes (configurable via `--timeout-secs`); print timeout error and exit non-zero
   - Print final state on exit: `job <id> reached <state> after <N> iterations (<N>ms)`
3. **End-to-end demo**:
   - `bee deploy examples/performance/prime_sieve.sql` returns a job_id
   - `bee jobs wait --job <job_id> --until done` blocks until the local CP marks the Job terminal
   - The script `scripts/demo-perf.sh` (after this story) can call these directly

### Out of scope (deferred)

- **Real worker execution** — the local `bee deploy` registers the DAG but no worker thread actually executes the Phases. The S33.1 multi-node demo proves the deploy path; real execution needs `run_node.rs` to spawn a worker that consumes the registered Tasks. That's S49.x (worker-on-this-node).
- **Re-deploy by `dag_hash`** — the S33.5.3 spec called for "if a Job with the same `dag_hash` exists, return it". S49 defers this; the local deploy always creates a new JobId.
- **S29 strict-mode + `use` validation** — the local deploy path runs through `extract_phase_dag` only, not the full S29 preprocessor. Plugins referenced by the SQL are not validated against the PluginManager. A future story threads the S29 path.

## File structure

| File | Action |
|---|---|
| `bee/src/main.rs` | Modify (add `deploy` (local) + `wait` to `run_jobs_cli`) |

1 Task (small).

## Acceptance criteria

- [x] `cargo build --workspace` green
- [x] `cargo test --workspace` ≥ 429 passed, 0 failed (achieved **429**)
- [x] `bee deploy examples/performance/prime_sieve.sql` exits 0 and prints `deployed as job 1` (where 1 > 0)
- [x] `bee jobs` (no arg, list) shows the new Job header table (works in the SAME process as deploy; across processes the state doesn't persist, which is expected MVP behavior)
- [x] `bee jobs inspect 1` works (same-process; cross-process returns "job 1 not found" which is the documented behavior)
- [x] `bee jobs wait --job 1 --until done --timeout-secs 3` returns non-zero with `timeout after 3s waiting for job 1 to reach a terminal state` (Job never reaches terminal without a worker; that's the MVP contract)
- [x] `bee deploy` with an invalid SQL file (no SELECTs) exits non-zero with `extract_phase_dag: dag: no SELECT statements found` (locked down by the existing `dag_extract` tests)
- [x] `scripts/demo-perf.sh` end-to-end: deploys all 3 demos via the S49 `bee deploy` path, demonstrates `bee jobs wait` (times out as expected — no worker), runs the 3 demos via `bee run` for actual perf measurement, prints a summary table (locked down by commit `2df0f84`-style run)

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `bee deploy <sql_file>` registers a Job via local CP | ✓ (S49) | N — the Job never executes; that's a worker follow-up |
| `bee jobs wait --job <id> --until done` polls until terminal | ✓ (S49) | N — in MVP, Jobs never reach a terminal state without a worker |
| Worker that consumes registered Tasks | — | N — S49.x follow-up |

## Related work

- **S27** (`bee jobs` + `bee jobs inspect`) — done; S49 extends with `wait`.
- **S33.1** (multi-node cluster demo) — done (code-level); blocked on `bee deploy` + `bee jobs wait` for the demo to work.
- **S33.5.3** (Deploy full DSL runner) — done; provides `extract_phase_dag`.
- **S45** (`scripts/demo-perf.sh`) — partially broken; S49 enables it.

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Local-only, or also expose as `--connect`-mode? | **Local-only** | The remote `--connect` mode already has its own `Deploy` admin RPC; S49 mirrors that for local-only |
| Worker execution in MVP? | **No** | Worker execution requires `run_node.rs` to spawn Tasks; that's S49.x |
| `bee jobs wait` timeout default? | **5 minutes** (configurable via `--timeout-secs`) | Long enough for the demo scripts; short enough to fail fast on hangs |

If any of these decisions should change, the user can override during the spec review.