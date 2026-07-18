# S18 — Cross-Pipeline Edge SQL Syntax + Dependency Tracking

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S15 (DataFusion executor wrapper)
**ADRs:** 0001 (P2P + Raft), 0003 (Producer Pipeline pattern)
**Status:** Draft (pending review)

## Why this story exists

Two Pipelines that should share data (e.g. "strategy A reads features computed by feature-engine B") have no way to declare the cross-Pipeline edge in SQL. The producer/consumer wiring today stops at "the Output Datasource Plugin is a Stream; subscribers can subscribe to it" — but the SQL has no syntax to declare "this Pipeline's input stream comes from that Pipeline".

This story adds the SQL grammar: `CREATE VIEW v_x AS SELECT ... FROM <job_id>.output`. At deploy time, the compiler:
1. Recognizes the `<job_id>.output` reference
2. Looks up the upstream Job
3. Registers the dependency (`Op::RegisterDependency` or a new `JobRecord.dependencies` entry)
4. Substitutes the reference with a placeholder subquery (same pattern S42 uses for `CREATE SINK`)

The control plane tracks the dependency in `JobRecord.dependencies` (the field already exists from S18's original design). The runtime will later use the dependency for in-process / cross-Node data channel wiring (S49.x follow-up).

## What already exists at HEAD

- `JobRecord.dependencies: Vec<DependencyRecord>` (from S18's original design; field is set by `Op::RegisterJob { dependencies: ... }`)
- `DependencyRecord { upstream_job: u32, stream: String }` (cross-Pipeline edge metadata)
- `JobLifecycleState::WaitingForUpstream` (state for Jobs that depend on an upstream Job)
- `evaluate_job_state(job_id) -> JobLifecycleState` (the orchestrator's state machine that drives the transition)
- `preprocess_sql_v2(sql) -> (Option<EmitTarget>, String)` (S42 preprocessor)
- `strip_create_source_and_view` (S42 / S33.5.3 — strips `CREATE SOURCE/VIEW` directives, substitutes references)
- `extract_phase_dag(sql) -> PhaseDag` (S33.5.3 — extracts top-level SELECTs as phases)
- The `TaskRecord.dependencies: Vec<u32>` field (S27 — intra-Job Task deps; S18 populates this for cross-Job Task deps)

## Scope

### In scope

1. **SQL grammar**: support `CREATE VIEW v AS SELECT ... FROM <job_id>.output` where `<job_id>` is a `u32` literal. The preprocessor recognizes this pattern.
2. **`detect_cross_pipeline_deps` helper** in `crates/bee-dsl-sql/src/preprocess.rs` (or `cross_pipeline.rs`): walks the SQL for `<job_id>.output` references. Returns `Vec<DependencyRecord> { upstream_job, stream }` for each match.
3. **Preprocessor integration**: the `Op::RegisterJob` accepts a `dependencies: Vec<DependencyRecord>` field. Update `bee_deploy_local` (S49) to compute cross-pipeline deps and pass them through.
4. **Reference substitution**: the `<job_id>.output` reference in the SQL body is replaced with a placeholder (e.g. `(SELECT * FROM <upstream_job_name> WHERE 1=1)` or similar) so DataFusion can parse the body. The actual cross-Pipeline data wiring is S49.x.
5. **Tests**:
   - `preprocess_sql_v2` recognizes `CREATE VIEW v AS SELECT * FROM 1.output` and emits a dep on JobId 1
   - `preprocess_sql_v2` recognizes the literal `2.output` reference and returns a dep
   - Negative test: `CREATE VIEW v AS SELECT * FROM my_alias` does NOT emit a dep (it's an alias, not a JobId)
6. **Integration test** in `crates/bee-control/tests/cross_pipeline.rs`:
   - Deploy 2 Jobs (A and B) on a single Node
   - B's SQL has `FROM a.output` (where a = 1, the JobId of A)
   - After deploy, B's `JobRecord.dependencies` contains `DependencyRecord { upstream_job: 1, stream: "output" }`
   - B's `evaluate_job_state` returns `WaitingForUpstream` (because A is still `Pending`)
   - Set A's lifecycle to `Running`; B transitions to `Running`

### Out of scope (deferred)

- **Cross-Node data channel wiring** (same Node → in-process; different Node → BRP subscription) — S49.x follow-up. S18 only adds the **dependency metadata** + the **state machine transition**; the actual data flow uses the metadata.
- **StreamIdentifier resolution** — the `stream: String` field of `DependencyRecord` is currently a placeholder; a future story (S49.x) populates it from the upstream's `CREATE SINK <stream>` or `EMIT INTO <stream>` directive.
- **Subscriber edge auto-creation** — when Job B depends on Job A, B's `JobMode` should become `Subscriber` (per S17's `job_mode()` derivation). This is already wired by the existing `evaluate_job_state` + the new `dependencies` field. No code change in S18.

## File structure

| File | Action |
|---|---|
| `crates/bee-dsl-sql/src/preprocess.rs` (or new `cross_pipeline.rs`) | Add `detect_cross_pipeline_deps` + integrate into the preprocessor pipeline |
| `crates/bee-dsl-sql/src/lib.rs` | Re-export the new helper |
| `bee/src/main.rs` | Modify `bee_deploy_local` to pass `dependencies` to `RegisterJob` |
| `crates/bee-control/tests/cross_pipeline.rs` | New test file |

1 Task (medium).

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` ≥ 431 passed, 0 failed
- [ ] `preprocess_sql_v2("CREATE VIEW v AS SELECT * FROM 1.output")` returns deps with `{ upstream_job: 1, stream: "output" }`
- [ ] `preprocess_sql_v2("SELECT * FROM 2.output")` recognizes the bare reference (no `CREATE VIEW`) and returns the dep
- [ ] `preprocess_sql_v2("SELECT * FROM my_alias")` does NOT recognize (alias, not a JobId)
- [ ] Deploy 2 Jobs on a single Node where B has `FROM a.output`; after deploy, B's `JobRecord.dependencies` contains the expected `DependencyRecord`
- [ ] After deploy, B's lifecycle state is `WaitingForUpstream` (because A is still `Pending`)
- [ ] Set A's lifecycle to `Running`; B's lifecycle state transitions to `Running` (the existing `evaluate_job_state` + `job_dependencies_satisfied` logic)

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `CREATE VIEW v AS SELECT ... FROM <job_id>.output` SQL syntax | ✓ (S18) | N |
| `JobRecord.dependencies` populated at deploy | ✓ (S18) | N |
| B's lifecycle transitions `WaitingForUpstream → Running` when A is `Running` | ✓ (S18) | N — uses the existing `evaluate_job_state` |
| Cross-Node data channel wiring | — | N — S49.x follow-up |
| `Subscriber` JobMode auto-classification | — | N — S49.x follow-up (S17 already derives the mode from deps; B becomes Subscriber automatically once deps are populated) |

## Related work

- **S17** (Producer detection) — done; B's `Subscriber` mode follows from B having a dep on A
- **S33.5.3** (Deploy full DSL runner) — done; `extract_phase_dag` parses top-level SELECTs
- **S27** (DAG visualization) — done; cross-Pipeline edges render as Task-to-Task connectors once S49.x wires the data flow
- **S49.x** (worker execution) — wires the actual cross-Pipeline data channel (in-process edge for same-Node, BRP subscription for different-Node)

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Where does `detect_cross_pipeline_deps` live? | **`crates/bee-dsl-sql/src/preprocess.rs`** | Co-located with the S42 preprocessor; both are pre-deploy SQL analysis |
| Reference substitution strategy | **Placeholder subquery** (replace `<job_id>.output` with `(SELECT * FROM <upstream_job_name>)`) | DataFusion can parse; runtime rewires to real data channel |
| `Op::RegisterJob` already has `dependencies`? | **Yes** (`Op::RegisterJob { job_id, dag_hash, owner_node, tenant }` currently — the spec's original design had `dependencies`; verify) | If missing, add the field |

If any of these decisions should change, the user can override during the spec review.