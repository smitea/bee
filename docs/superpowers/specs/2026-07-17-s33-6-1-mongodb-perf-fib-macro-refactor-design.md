# S33.6.1 — Macro Refactor for Remaining Plugins (mongodb + perf-fib)

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S33.6 (the `#[bee_adapter]` proc-macro at HEAD `f45d90c` is done + 482 tests green)
**Status:** Draft (pending review)

## Why this story exists

S33.6 shipped the `#[bee_adapter]` proc-macro and `register_vtable!` sub-macro at HEAD `f45d90c` (commits `a392f37` … `d12cb01`). Five plugins have been refactored to use it (binance, google-news, influxdb, ta-lib, onnx-ml). Two plugins are still on hand-written FFI vtables:

| Plugin | Inputs | Outputs | Handlers | State at HEAD |
|---|---|---|---|---|
| `bee-plugin-perf-fib` | 0 | 0 | 2 (`fib_seed`, `fib_step`) | Hand-written `seed_shim` + `step_shim` modules (~280 LOC of FFI glue) |
| `bee-plugin-binance` | 1 (`subscribe`) | 0 | 0 | **Already refactored at `4929cd0`** — but the stash contains a re-touch that duplicates the same change. Out of scope. |
| `bee-plugin-mongodb` | 2 (`find`, `aggregate`) | 3 (`insert`, `insert_many`, `update`) | 0 | 5 hand-written shim modules (~830 LOC of FFI glue) |

S33.6.1 closes the gap by refactoring the remaining two plugins, bringing the whole workspace to one consistent plugin-authoring pattern. No wire-format changes — the vtable layout, `Event` bincode schema, and `Handler` state encoding are unchanged.

The user's working tree (`stash@{0}`) contains the in-flight start of this refactor (perf-fib almost complete; mongodb partial — 5 placeholder `#[bee_adapter]` impls written, but the Factory still references the deleted `*_shim` modules so it does not compile). S33.6.1 picks up from there.

## Scope

### In scope

1. **Complete `bee-plugin-mongodb` macro refactor** — 5 adapters:
   - `InsertAdapter` (output, `name = "insert"`) — `open` decodes `MongodbConfig + InsertArgs`, `emit` decodes `Event` → `Document`, calls `do_insert`
   - `InsertManyAdapter` (output, `name = "insert_many"`) — same shape, batched
   - `UpdateAdapter` (output, `name = "update"`) — same shape, calls `do_update`
   - `FindAdapter` (input, `name = "find"`) — `open` builds mpsc channel + spawns worker that calls `do_find` (existing tokio task pattern), `next` reads from channel
   - `AggregateAdapter` (input, `name = "aggregate"`) — same shape as `FindAdapter`
   - `MongodbFactory::init()` uses `register_vtable!` to register the 5 macro-generated vtables (deletes the 5 `mod *_shim` blocks + the 5 `unsafe extern "C" fn` blocks)
   - **Preserve all existing business logic**: `acquire_client`, `do_insert`, `do_insert_many`, `do_update`, `do_find`, `do_aggregate`, `MongodbError::write_into`, `MongodbConfig`, `InsertArgs`, etc.

2. **Complete `bee-plugin-perf-fib` macro refactor** — 2 handlers:
   - `FibStepHandler` (handler, `name = "fib_step"`) — uses `#[bee_method(slot = "init_state")]` to return the seed `FibState { prev2: 0, prev1: 1 }`
   - `FibSeedHandler` (handler, `name = "fib_seed"`) — uses `#[bee_method(slot = "init_state")]` to return `()`
   - `PerfFibFactory::init()` uses `register_vtable!`
   - Preserve the `plugin_manifest()` / `PerfFibFactory` shape
   - Drop the hand-written `seed_shim` / `step_shim` modules
   - Wire `bee-plugin-macro` into `Cargo.toml` (already done in stash — just verify)

3. **Macro improvements** — small additions needed to support the mongodb + perf-fib refactor (already partly in stash):
   - Handler `#[bee_method(slot = "init_state")]` support (commit `54398cd` already added; verify it's complete and exposed in the perf-fib test fixture)
   - Optional `open` slot (default generated when not provided — saves boilerplate for adapters that don't need config parsing). The stash already adds `has_custom_open` / `has_custom_close` support — finish it and lock down with a test
   - **No new wire-format changes** — these are ergonomics only

### Out of scope (noise to discard from the stash)

These are working-tree changes from other workstreams. They do NOT belong to S33.6.1; if needed they get their own stories:

- `plugins/quant/bee-plugin-binance/src/lib.rs` — **duplicate** of the `4929cd0` refactor (already at HEAD)
- `examples/performance/{fibonacci,multi_stream_analytics,prime_sieve}.sql` — `prime_sieve.sql` shrunk from 3707 → 73 lines (this is a separate "trim the demo to N phases" workstream)
- `scripts/demo-perf.sh` — same workstream as the SQL trim
- `crates/bee-dsl-sql/{lib,physical,preprocess}.rs` — adds `EmitTarget::Plugin(String)` for `CREATE SINK` syntax (a separate "S29+ Sink DSL" story)
- `crates/bee-plugin-sdk/src/lib.rs` + `crates/bee-plugin-sdk/src/kv.rs` — new `kv` module exposing host-side KV to plugins via FFI (a separate "S30 secret_store FFI" or "S29 KV FFI hook" story)
- `docs/book/` — `mdbook build` output (~60 files). Should be `.gitignore`d.

These will be left in `stash@{0}` for the user to file as separate stories.

## Approach

### Step 1 — finish the macro improvements (in scope)

Verify the stash's `crates/bee-plugin-macro/src/lib.rs` changes:
- `has_custom_open` / `has_custom_close` — if neither is provided, generate default `open` (returns `Ok(Self)` on empty config) / `close` (returns `Ok(())`) that the host calls. This saves ~10 lines per adapter that doesn't need config.
- Handler `init_state` slot — verified already committed at `54398cd bee-plugin-macro: support custom init_state async fn`. The stash's `perf-fib` use of `#[bee_method(slot = "init_state")]` is the lock-down.

**Test plan**: 1 new test `crates/bee-plugin-macro/tests/macro_default_open_close.rs` (asserts a no-`open` adapter compiles + round-trips through the vtable).

### Step 2 — refactor perf-fib

The stash already has the refactored `plugins/bee-plugin-perf-fib/src/lib.rs`. Verify:
- All 2 handlers use `#[bee_adapter(handler, name = "...")]`
- `register_vtable!` is used in `PerfFibFactory::init()`
- No `seed_shim` / `step_shim` modules
- The handler signature `handle(state: FibState, event: FibEvent) -> (FibState, i128)` works with the macro
- `Cargo.toml` wires `bee-plugin-macro` and removes `hex`, `description`, `lints.workspace`

**Test plan**: the new `plugins/bee-plugin-perf-fib/tests/state.rs` (untracked in stash) asserts state round-trip via the vtable. Run `cargo test -p bee-plugin-perf-fib` to confirm green.

### Step 3 — refactor mongodb

This is the bulk of S33.6.1. The stash has the 5 placeholder `#[bee_adapter]` impls at lines 640–757; the Factory at lines 588–602 still references the deleted `*_shim` modules.

The refactor approach:

1. **Lift the FFI bodies into the adapter impls.** Each adapter's `open`/`emit`/`next`/`close` body is the de-shimmed version of the corresponding `unsafe extern "C" fn` in HEAD's shim modules:
   - `InsertAdapter::open` decodes `MongodbConfig` + `InsertArgs`, calls `acquire_client`, returns `Self { ... }`
   - `InsertAdapter::emit(event)` decodes `Document` from `event.payload`, calls `do_insert(...)`, writes any `MongodbError::write_into` result into an `err_out` (the macro's emit slot doesn't currently have an `err_out` param — see "Macro extension needed" below)
   - `InsertAdapter::close` drops the `Client` clone and returns `Ok(())`
   - Similarly for `InsertManyAdapter`, `UpdateAdapter`
   - `FindAdapter::open` builds the mpsc channel, spawns the worker thread (existing pattern), returns `Self { rx, _worker, ... }`
   - `FindAdapter::next` blocks on the channel via `block_on`, returns `Ok(Some(event))` or `Ok(None)` on EOF
   - `FindAdapter::close` drops the worker, returns `Ok(())`
   - Similarly for `AggregateAdapter`

2. **Update `MongodbFactory::init()`** to use `register_vtable!`:
   ```rust
   register_vtable! {
       input_adapters, output_adapters, handlers;
       output "insert"      => &INSERT_ADAPTER_VTABLE,
       output "insert_many" => &INSERT_MANY_ADAPTER_VTABLE,
       output "update"      => &UPDATE_ADAPTER_VTABLE,
       input  "find"        => &FIND_ADAPTER_VTABLE,
       input  "aggregate"   => &AGGREGATE_ADAPTER_VTABLE,
   }
   ```

3. **Delete the 5 shim modules** (lines 661–1486 in HEAD): ~830 LOC of FFI glue gone.

### Macro extension needed (Step 3 dependency) — chosen: err_out param

**Chosen**: extend the macro's `gen_output_adapter` to emit `emit` with an `err_out` parameter.

- New signature: `(ctx, event_ptr, event_len, err_out: *mut EventBytes) -> i32`
- The handler's `Err(AdapterError::Emit(msg))` is converted to an `Event { payload: msg.into_bytes() }` written to `err_out`; the FFI returns `-1`.
- This matches the existing shim pattern at `plugins/quant/bee-plugin-mongodb/src/lib.rs` (HEAD `update_shim::emit` line ~1100).

**Impact**:
- This is a **breaking change** to the macro's emit ABI. The existing `macro_expands_output_adapter` test (Task 3 in `docs/superpowers/plans/2026-06-10-s33-6-plugin-macro-ergonomics.md`) must update its emit call to pass `&mut EventBytes::EMPTY` as the 4th arg.
- The existing refactored Output plugin `bee-plugin-influxdb` (which uses the macro) must rebuild and its test must be checked.

**Rejected alternative** (drop the err_out, log via `tracing` instead): simpler, no ABI break, but loses cross-FFI diagnostics that the mongodb shim currently relies on. We reject because the host CLI's error-message UX relies on the `EventBytes`-shaped err path.

### Test plan

**Unit / integration**:
1. `cargo build --workspace` green
2. `cargo test --workspace --no-fail-fast` — count ≥ 415 (current baseline)
3. `cargo test -p bee-plugin-macro` — the 5 existing tests + 1 new (default_open_close)
4. `cargo test -p bee-plugin-perf-fib` — new `tests/state.rs` (was untracked in stash)
5. `cargo test -p bee-plugin-mongodb` — `tests::insert_args_round_trip` + any existing tests still pass (the shim modules had unit tests; migrate them to the impl-block level)
6. The `S37` acceptance criterion "Connects to a real MongoDB instance" is HITL (not tested in CI) — same as before; no change

### Out-of-scope items left in stash

The following stay in `stash@{0}` and get their own stories later:
- DSL `CREATE SINK` preprocessor (S29+ sink DSL)
- `bee-plugin-sdk/src/kv.rs` host-side KV FFI hook (S30 follow-up)
- `prime_sieve.sql` trim (S41 demo cleanup)
- `binance/src/lib.rs` re-touch (already at HEAD, just discard)
- `docs/book/` build output (`.gitignore` cleanup)

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` ≥ 415 passed, 0 failed
- [ ] `cargo test -p bee-plugin-macro` — 6 tests pass (5 existing + new `default_open_close`)
- [ ] `cargo test -p bee-plugin-perf-fib` — new state-round-trip test passes
- [ ] `cargo test -p bee-plugin-mongodb` — existing unit tests still pass (migrated where needed)
- [ ] `plugins/quant/bee-plugin-mongodb/src/lib.rs` has zero `mod *_shim { ... }` blocks; the 5 adapters all use `#[bee_adapter(...)]`
- [ ] `plugins/quant/bee-plugin-mongodb/src/lib.rs` `MongodbFactory::init()` uses `register_vtable!`
- [ ] `plugins/bee-plugin-perf-fib/src/lib.rs` has zero `mod seed_shim` / `mod step_shim` blocks; both handlers use `#[bee_adapter(handler, ...)]`
- [ ] `plugins/bee-plugin-perf-fib/src/lib.rs` `PerfFibFactory::init()` uses `register_vtable!`
- [ ] `crates/bee-plugin-macro/src/lib.rs` supports Handler `init_state` slot (already at `54398cd`; verify)
- [ ] `crates/bee-plugin-macro/src/lib.rs` supports optional `open` / `close` slots (default generated when not provided)
- [ ] `crates/bee-plugin-macro/src/lib.rs` output emit has `err_out` parameter (for cross-FFI diagnostics)
- [ ] Diff size: `plugins/quant/bee-plugin-mongodb/src/lib.rs` shrinks by ~830 LOC (the 5 shim modules); perf-fib shrinks by ~280 LOC; macro grows by ~50 LOC
- [ ] Commits land as a small series (one per major step), not one mega-commit
- [ ] `git stash drop stash@{0}` only after the user confirms the out-of-scope items are filed separately

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `bee-plugin-mongodb` macro refactor (5 adapters) | ✓ (S33.6.1 + this story) | N — third-party plugin author trial |
| `bee-plugin-perf-fib` macro refactor (2 handlers) | ✓ (S33.6.1 + this story) | N |
| Handler `init_state` slot (from `54398cd`) | ✓ | N |
| Optional `open` / `close` slots | ✓ | N |
| Output `emit` `err_out` param | ✓ | N |
| Cross-FFI error path preserved (via `err_out`) | ✓ | N |

## Related work

- **S33.6** (the `#[bee_adapter]` proc-macro + `register_vtable!` sub-macro + 5 test fixtures + trybuild snapshot) — landed at HEAD `f45d90c`. This story extends S33.6 by applying the macro to the 2 remaining plugins.
- **S33.6.2 (proposed)** — apply the macro to the **3rd-party plugin examples** in `examples/` (a hypothetical `examples/binance-mock-plugin` etc.) as a usability check. Out of scope here.
- **S34** — the binance plugin (already refactored at `4929cd0`; the stash's binance re-touch is a duplicate that gets dropped).
- **S37** — the mongodb plugin spec; this story does not change the S37 contract or acceptance criteria (it only changes the internal FFI implementation).
- **S41** — the perf-fib plugin; this story does not change the S41 contract or acceptance criteria.
- **S30 follow-up** — the `crates/bee-plugin-sdk/src/kv.rs` work is a separate story (host-side KV FFI hook for plugins). Not part of S33.6.1.
