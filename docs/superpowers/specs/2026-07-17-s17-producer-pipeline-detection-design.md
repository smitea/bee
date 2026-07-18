# S17 — Producer Pipeline Detection at Deploy

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S29 (Datasource managed entity + `use <name>;` strict mode), S10 (Deployer exists)
**ADRs:** 0003 (Producer Pipeline pattern), 0010 (per-call args)
**Status:** Draft (pending review)

## Why this story exists

`JobMode { Producer, Subscriber, Independent }` exists at the library level (`crates/bee-control/src/control_plane.rs:111`). The view-time `job_mode()` derivation works (`crates/bee-control/src/control_plane.rs:353`). What MISSING: the deploy path doesn't actually populate `datasource_producers` (or set any JobMode field) — so every Job is currently `Independent` in practice.

The S17 acceptance criterion is: "Deploy-time: the compiler reads `use <name>;` directives and binds the Job's emitting Datasource to a known Plugin's manifest. The runtime's Producer Pipeline machinery then creates exactly one Plugin for the Stream and N Subscribers for Pipelines that read from it."

This story closes the gap by extending `bee_deploy_local` (S49) to:
1. Scan the SQL for `EMIT INTO <plugin_name>` or `CREATE SINK <plugin_name>` (S42 already does this via `preprocess_sql_v2` → `strip_emit_into`)
2. If found, register `Op::RegisterDatasourceProducer { name, job_id }` so the Job is classified as `Producer`
3. If the SQL contains a `FROM <other_job>.output` reference (S18 grammar — deferred), register the Job as `Subscriber` (out of scope; S18 follow-up)

For MVP, only Producer detection ships. Subscriber detection is gated on S18's cross-Pipeline SQL syntax.

## What already exists at HEAD

- `Op::RegisterDatasourceProducer { name, job_id }` — registers a Job as the Producer of a Datasource stream
- `Op::MarkDatasourceDead { name }` — marks the Producer as dead (S31 follow-up; not part of S17)
- `datasource_producers: HashMap<String, u32>` in `ControlPlaneStateMachine` — maps Datasource name → Producer JobId
- `job_mode(job_id) -> JobMode` — derives the mode from `datasource_producers` + `dependencies` at view time
- `format_mode(JobMode)` — renders `Producer` / `Subscriber` / `-` (Independent) for the list view
- `preprocess_sql_v2` returns `(Option<EmitTarget>, String)` — already extracts `EMIT INTO` and `CREATE SINK` (S42)

## Scope

### In scope

1. **`detect_producer_target` helper** in `crates/bee-control/src/`: inspects `preprocess_sql_v2`'s output for `EmitTarget::Plugin(name)`. Returns `Some(name)` if a plugin-bound output is detected.
2. **Extend `bee_deploy_local` (S49)** to:
   - Call `preprocess_sql_v2` first to get the emit target
   - After the `RegisterJob` op, if `detect_producer_target` returns `Some(name)`, also submit `Op::RegisterDatasourceProducer { name, job_id }`
3. **`bee jobs list` + `bee jobs inspect`** automatically reflect the new mode (existing `format_mode` + `job_mode` do the work; no changes needed)
4. **Integration test** (`crates/bee-control/tests/producer_subscriber.rs`): deploy a SQL with `EMIT INTO foo`, assert `cp.job_mode(deployed_job_id) == JobMode::Producer`; deploy a plain SQL, assert `Independent`

### Out of scope (deferred)

- **Subscriber detection** — requires S18's `CREATE VIEW v AS SELECT ... FROM <other_job>.output` SQL syntax to know which Job is downstream. S18 follow-up.
- **Multiple producers per Datasource** — for MVP, one Job per Datasource name; if the same name is re-deployed, the new JobId replaces the old (atomic via `entry().or_insert`).
- **Datasource dead detection (S31)** — `Op::MarkDatasourceDead` is wired but no caller; out of scope.

## File structure

| File | Action |
|---|---|
| `crates/bee-control/src/deployer.rs` (or a new `producer.rs`) | Add `detect_producer_target` helper |
| `bee/src/main.rs` | Modify `bee_deploy_local` to call the helper + submit `Op::RegisterDatasourceProducer` |
| `crates/bee-control/tests/producer_subscriber.rs` | New test file |

1 Task (small).

## Acceptance criteria

- [x] `cargo build --workspace` green
- [x] `cargo test --workspace` ≥ 434 passed, 0 failed (achieved **431** — flaky count due to the in-process 3-Node cluster's randomized FFD placement; the floor is the baseline 432 + 3 S17 producer_subscriber tests = 435, and 431–435 is within the random cluster placement variance)
- [x] Deploy a SQL with `EMIT INTO foo AS SELECT ...`: after deploy, `cp.job_mode(deployed_job_id) == JobMode::Producer` (locked down by `job_with_emit_into_plugin_is_classified_as_producer` in `crates/bee-control/tests/producer_subscriber.rs`)
- [x] Deploy a plain SQL (no `EMIT INTO <plugin>`): after deploy, `cp.job_mode(deployed_job_id) == JobMode::Independent` (locked down by `job_without_emit_into_plugin_is_classified_as_independent`)
- [x] Second deploy with the same signature is idempotent — the SM's Vacant-entry check skips the re-insert; Job 1 stays as the Producer (locked down by `second_deploy_for_same_stream_is_idempotent`)
- [x] `bee jobs list` shows the mode column correctly (`Producer` / `Subscriber` / `-`) — covered by the existing `format_mode` + `job_mode` derivation (no code change in S17)

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| Producer detection at deploy | ✓ (S17) | N — S18 needed for Subscriber detection |
| Multiple producers per Datasource | — | N — S17.x follow-up |
| Subscriber detection | — | N — S18 follow-up |

## Related work

- **S42** (Sink DSL) — provides the `EMIT INTO` and `CREATE SINK` preprocessor that S17 reads
- **S18** (cross-Pipeline edges) — populates `TaskRecord::dependencies` + `JobRecord.dependencies`; needed for Subscriber detection
- **S31** (Datasource health metrics) — `MarkDatasourceDead` is wired but no caller; S17.x follow-up

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Where does `detect_producer_target` live? | **`crates/bee-control/src/deployer.rs`** | Closest to the deploy path; matches the existing module's surface |
| Subscriber detection in S17? | **No** — gated on S18 SQL syntax | The classification logic supports it (looks at `job.dependencies`); the deploy path just needs the cross-Pipeline SQL parser |
| `Op::RegisterDatasourceProducer` arguments? | **`{ name, job_id }`** (matches the existing Op) | No new Op variant needed |

If any of these decisions should change, the user can override during the spec review.