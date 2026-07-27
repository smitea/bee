# S21 Close-Out — Wire `release()` Into the Job-Stop Path

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S11 (Heartbeat — done), S08 (CP SM — done)
**ADRs:** 0004 (KV cluster), 0001 (P2P + Raft)
**Status:** Draft (pending review)

## Why this story

`PluginManager::release(id)` (in `crates/bee-registry/src/lib.rs:290`) correctly decrements a Plugin's refcount and auto-unloads the Plugin when refcount hits 0. The library-side guarantee is solid and locked down by `two_versions_of_binance_run_independently_in_the_manager` (the S21 integration test).

What's missing: a **production caller**. When a Job transitions to a terminal state (`Completed` / `Failed` / `Revoked`), the Plugins it used should be released so the runtime unloads them once no other Job references them.

This story closes the S21 acceptance criterion by:
1. Adding a `JobRecord.plugins: HashSet<PluginId>` field (the set of Plugin ids the Job uses — populated at `RegisterJob` time)
2. Wiring `plugin_manager.release(plugin_id)` into the SM's `Op::UpdateJobLifecycle` apply path when the new state is terminal
3. Adding an integration test that demonstrates a Plugin auto-unloads when its last Job stops

For MVP, the `plugin_manager` lives on the `Node` process (not on the CP SM directly — the SM doesn't own plugin state). The leader's orchestrator watches for terminal Job transitions and issues the `release`. In the in-process 3-Node cluster, the same `PluginManager` is shared across all nodes (the leader's view is the source of truth).

## What already exists at HEAD

- `PluginManager::release(id)` (atomic refcount decrement + auto-unload on 0)
- `JobRecord.lifecycle: JobLifecycleState` (with `Completed` / `Failed` variants)
- `Op::UpdateJobLifecycle { job_id, state }` — transitions a Job's lifecycle
- S21 integration test `two_versions_of_binance_run_independently_in_the_manager` (library semantics locked down)

## Scope

### In scope

1. **`JobRecord.plugins: HashSet<PluginId>`** field — the set of Plugin ids the Job uses. Populated at `RegisterJob` time (caller passes it; the S49 `bee_deploy_local` path sets it to `vec![]` for MVP since the deploy path doesn't yet know which plugins a Job uses; the field is additive).
2. **`Op::UpdateJobLifecycle` apply path**: when `state` is `Completed` or `Failed`, call `plugin_manager.release(plugin_id)` for each `plugins` entry. The `plugin_manager` is a new parameter on `apply_op` (or a method on the SM that the orchestrator invokes).
3. **Node-side wiring** (`crates/bee-control/src/raft/node.rs`): when a terminal `Op::UpdateJobLifecycle` is applied, the node calls `plugin_manager.release(...)` for each plugin in `JobRecord.plugins`.
4. **Integration test**: register Plugin X → register Job that uses X → Job transitions to `Completed` → `PluginManager::refcount_of(X) == None` (auto-unloaded).

### Out of scope (deferred)

- **Per-task plugins** — `TaskRecord` doesn't have a `plugins` field. For MVP, the Job-level list is enough; the per-task variant is S21.x.
- **S29 ACL** — `JobRecord.tenant` is set but not checked. S29 follow-up.
- **Cross-Node Plugin state** — in production, each Node has its own PluginManager. The leader's view is the source of truth, but followers need to mirror. Cross-Node is a S49.x follow-up.

## File structure

| File | Action |
|---|---|
| `crates/bee-control/src/control_plane.rs` | Add `JobRecord.plugins` + `apply_op` calls `release` on terminal state |
| `crates/bee-control/src/raft/node.rs` | Pass `plugin_manager` to the apply path |
| `crates/bee-control/tests/refcount_release_on_job_stop.rs` | New test file |

1 Task (small).

## Acceptance criteria

- [x] `cargo build --workspace` green
- [x] `cargo test --workspace` ≥ 438 passed, 0 failed
- [x] Integration test: register Plugin X → register Job with `plugins: {X}` → transition Job to `Completed` → `plugin_manager.refcount_of(X) == None` (auto-unloaded)
- [x] Same test, but transition to `Failed` — also releases
- [x] Same test, but transition to `Running` (non-terminal) — does NOT release (refcount stays at 1)

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `release()` called on terminal lifecycle transition | ✓ (S21 close-out) | N — in-process MVP; cross-Node plugin state is S49.x |
| Per-task plugins | — | N — S21.x |
| S29 ACL | — | N — S29 follow-up |

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Where does the `plugin_manager` reference live? | **On the Node, passed to the apply path** (matches S17/S29 patterns) | The SM doesn't own plugin state |
| Should we also release on `Revoked`? | **No** | S21 says terminal is `Completed | Failed`; `Revoked` is a S18 / S25 concept (work-stealing) — out of scope here |

If any of these decisions should change, the user can override during the spec review.