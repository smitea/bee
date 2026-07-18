# S27 — `bee jobs` / `bee jobs inspect` Close-Out (DAG visualization)

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S10 (deploy exists)
**ADRs:** none
**Status:** Draft (pending review)

## Why this story exists

`format_job_inspect` (in `crates/bee-control/src/jobs_view.rs:62`) already produces a per-Task table with a vertical tree prefix (`├─ Task N`), but the S27 acceptance criterion is "**DAG diagram**" — i.e., visualize the edges (dependencies) between Tasks, not just a flat list.

The current output:

```
Job 1
  job_id:       1
  phase_id:     1
  status:       running
  ...
    ├─ Task 1
    ├─ Task 2
    └─ Task 3
```

is a list, not a DAG. The `TaskRecord` has a `dependencies: Vec<u32>` field (per the S17 cross-Pipeline edge design), but `format_dag` doesn't read it.

This story closes the S27 gap by:
1. Adding a real DAG layout (uses `dependencies` to draw edges as ASCII connectors: `T1 → T2 → T3`)
2. Adding integration tests that lock down the layout for: linear chain, diamond, two independent chains
3. Verifying the color codes from `colorize_lifecycle` work correctly in `bee jobs` and `bee jobs inspect`

## What already exists at HEAD

- `crates/bee-control/src/jobs_view.rs::format_job_inspect` (line 62) — per-Task view + lifecycle
- `crates/bee-control/src/jobs_view.rs::format_dag` (line 110) — current vertical list
- `crates/bee-control/src/jobs_view.rs::colorize_lifecycle` — green / yellow / red color codes
- `crates/bee-control/src/jobs_view.rs::format_jobs` — list view with color codes
- `bee/src/main.rs::run_jobs_cli` (S27 dispatch) — `bee jobs` (list) + `bee jobs inspect <id>`

## What `TaskRecord::dependencies` looks like today

```rust
pub struct TaskRecord {
    pub task_id: u32,
    pub job_id: u32,
    pub phase_id: u32,
    pub owner_node: u32,
    pub status: TaskStatus,
    pub started_at_ms: u64,
    pub migrating_from_node: Option<u32>,
    pub dependencies: Vec<u32>, // <-- this is what `format_dag` should use
}
```

The MVP always stores `dependencies: vec![]` (all Tasks are independent). For S27's DAG layout to be non-trivial, the test must construct Tasks with explicit `dependencies`. S18 (cross-Pipeline edges) is the follow-up that actually populates this in production.

## Scope

### In scope

1. **Replace `format_dag` with a real DAG layout** that reads `TaskRecord::dependencies` and draws ASCII edges:
   - Linear chain `T1 → T2 → T3`: `T1 → T2 → T3`
   - Diamond `T1 → {T2, T3} → T4`: draw with branching connectors
   - Independent tasks: list them in a single row
2. **Add 3 unit tests** for the DAG layouts
3. **Add 1 integration smoke test** via `run_jobs_cli` that:
   - Creates a CP with a Job + 3 Tasks (linear chain)
   - Calls `format_job_inspect` and asserts the output contains the chain syntax
4. **Verify color codes**:
   - `bee jobs` (list) shows green for running, yellow for migrating, red for failed
   - `bee jobs inspect <id>` shows the same colors in the per-Task status line

### Out of scope (deferred)

- **S24 per-Phase metrics + `bee diagnostics` real values** — `format_task_diagnostics` shows placeholders today. Real metrics need a `PhaseMetrics` store in the ControlPlane (the runtime has the data; the CP doesn't). That's an S24.x follow-up.
- **S24 histogram buckets** — same as above
- **S24 CPU overhead** — needs benchmark harness
- **S18 cross-Pipeline edges** — populates `dependencies` in production; not part of S27

## File structure

| File | Action |
|---|---|
| `crates/bee-control/src/jobs_view.rs` | Modify (rewrite `format_dag` + add 3 tests + 1 integration test) |

1 Task (small).

## Acceptance criteria

- [x] `cargo build --workspace` green
- [x] `cargo test --workspace` ≥ 429 passed, 0 failed (achieved **432** — +3 S27 tests)
- [x] Linear chain (T1 → T2 → T3) renders with `├─` per Task + `│` between levels (locked down by `format_dag_linear_chain_draws_level_separators`)
- [x] Diamond (T1 → {T2, T3} → T4) renders with branching: T1 alone at L0, T2+T3 at L1, T4 at L2 (locked down by `format_dag_diamond_renders_both_branches`)
- [x] Independent tasks (no edges) render as a vertical tree with `├─` / `└─` prefixes (no `│` separator) (locked down by `format_dag_independent_tasks_listed_in_single_row`)
- [x] `bee jobs` + `bee jobs inspect <id>` color codes work (green / yellow / red) — covered by existing `bee_jobs_color_codes_for_different_lifecycles_s27_acceptance` test in `crates/bee-control/tests/jobs_view.rs`

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `format_dag` produces real DAG layout from `dependencies` | ✓ (S27) | N — `dependencies` is always empty in MVP; S18 populates it |
| Color codes work in `bee jobs` / `bee jobs inspect` | ✓ (S27 — verification) | N |
| S24 real metrics in `bee diagnostics` | — | N — S24.x follow-up (requires CP-side `PhaseMetrics` store) |

## Related work

- **S18** (cross-Pipeline edges) — populates `dependencies` in production; S27 is the read side
- **S24** (per-Phase metrics) — partially done; `PhaseMetrics` is plumbed through the runtime; the CP-side store + read API is a follow-up
- **S28** (bee diagnostics + bee cluster status) — already has placeholder lines; full wiring is S28 follow-up

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| S24 work in this story? | **No** — S24 needs runtime plumbing (CP-side `PhaseMetrics` store); too big for a quick win | S24.x follow-up |
| DAG layout style | **ASCII with `→` connectors** | Simple, no graphviz dep; sufficient for the 2-5 task range typical of demo pipelines |
| Per-Task status colors in DAG | **Yes** — color the status token (Running=green, etc.) inside each DAG node | Matches `format_jobs` already doing this for the list view |

If any of these decisions should change, the user can override during the spec review.