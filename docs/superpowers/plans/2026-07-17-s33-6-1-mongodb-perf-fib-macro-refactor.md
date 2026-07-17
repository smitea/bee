# S33.6.1 — Macro Refactor for mongodb + perf-fib Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply the S33.6 `#[bee_adapter]` proc-macro + `register_vtable!` sub-macro (already landed at HEAD `f45d90c`) to the remaining two plugins (`bee-plugin-mongodb` 5 adapters + `bee-plugin-perf-fib` 2 handlers), with three small macro improvements needed to support them: verify Handler `init_state` slot, optional `open`/`close` slots, and `emit` `err_out` parameter.

**Architecture:** Macro improvements first (lock them down with TDD), then `perf-fib` refactor (the smaller of the two — most of it is already in `stash@{0}`), then `mongodb` refactor in 5 sequential Tasks (one per adapter) that each lift the FFI glue from the existing `mod *_shim` blocks into `#[bee_adapter]` impls preserving the existing `do_*` business-logic helpers. Every Task ends with a commit; the Workspace test count must remain ≥ 415 (current baseline) at every Task boundary.

**Tech Stack:** Rust, `proc-macro2`, `quote`, `syn`, `tokio`, `bincode`, `serde`, `bson`, `mongodb` (for the mongodb crate driver). Workspace member `bee-plugin-macro` at HEAD. New workspace tests in `crates/bee-plugin-macro/tests/`.

---

## File Structure

| File | Action | Purpose |
|---|---|---|
| `crates/bee-plugin-macro/src/lib.rs` | Modify | Add 3 macro improvements (verify init_state, optional open/close, emit err_out) |
| `crates/bee-plugin-macro/tests/macro_handler_init_state.rs` | Create | 1 new test for Handler init_state slot (TDD) |
| `crates/bee-plugin-macro/tests/macro_default_open_close.rs` | Create | 1 new test for optional open/close slots (TDD) |
| `crates/bee-plugin-macro/tests/macro_emit_err_out.rs` | Create | 1 updated test for Output emit with err_out (TDD) |
| `plugins/bee-plugin-perf-fib/Cargo.toml` | Modify (verify) | Confirm `bee-plugin-macro` dep + remove `hex` |
| `plugins/bee-plugin-perf-fib/src/lib.rs` | Modify | Rewrite using `#[bee_adapter]` (most already in stash) |
| `plugins/bee-plugin-perf-fib/tests/state.rs` | Create | State round-trip test (already in stash as untracked) |
| `plugins/quant/bee-plugin-mongodb/Cargo.toml` | Modify (verify) | Confirm `bee-plugin-macro` dep |
| `plugins/quant/bee-plugin-mongodb/src/lib.rs` | Modify | Rewrite using `#[bee_adapter]` for 5 adapters; delete 5 shim modules |

Tasks 1-3 lock down the macro changes. Task 4 finishes perf-fib. Tasks 5-9 refactor mongodb one adapter at a time. Task 10 wires the Factory + deletes the remaining shim modules. Task 11 runs the full workspace test suite to confirm ≥ 415 tests pass.

---

## Task 1: Verify Handler `#[bee_method(slot = "init_state")]` slot

**Files:**
- Verify (no change): `crates/bee-plugin-macro/src/lib.rs` (commit `54398cd` already added this)
- Create: `crates/bee-plugin-macro/tests/macro_handler_init_state.rs`

The Handler macro path currently supports an optional `init_state` slot. Per commit `54398cd bee-plugin-macro: support custom init_state async fn`, the `gen_handler` function should recognize `#[bee_method(slot = "init_state")]` on a method shaped `async fn init_state() -> AdapterResult<StateT>` and generate an FFI fn that returns the bincode-encoded state. We don't need to add this — we just need to **lock it down** with a test so the stash's `perf-fib` use of it is verified.

- [ ] **Step 1.1: Write the test (RED if not yet supported, GREEN otherwise)**

Create `crates/bee-plugin-macro/tests/macro_handler_init_state.rs`:

```rust
//! S33.6.1 Task 1: lock down Handler
//! `#[bee_method(slot = "init_state")]` support.
//! The macro should generate an `init_state`
//! FFI fn that returns the bincode-encoded
//! custom state (not just empty Vec<u8>).

use bee_adapter::AdapterResult;
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::HandlerVtable;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterState {
    pub count: u64,
    pub label: String,
}

pub struct CounterHandler;

#[bee_adapter(handler, name = "counter")]
impl CounterHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<CounterState> {
        Ok(CounterState {
            count: 0,
            label: "starting".into(),
        })
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(
        mut state: CounterState,
        _event: Vec<u8>,
    ) -> AdapterResult<(CounterState, Vec<u8>)> {
        state.count += 1;
        let result = bincode::serialize(&state.count).unwrap();
        Ok((state, result))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handler_init_state_returns_custom_state() {
    // Call the generated init_state FFI; expect
    // a CounterState blob, NOT an empty Vec<u8>.
    let mut out = EventBytes::EMPTY;
    let rc = unsafe { ((*COUNTER_HANDLER_VTABLE).init_state)(&mut out) };
    assert_eq!(rc, 0, "init_state must return 0");
    assert!(out.len > 0, "init_state must return non-empty bytes");
    let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
    let state: CounterState = bincode::deserialize(bytes)
        .expect("must bincode-decode as CounterState");
    assert_eq!(state.count, 0);
    assert_eq!(state.label, "starting");
}
```

- [ ] **Step 1.2: Add the test's dev-deps to `crates/bee-plugin-macro/Cargo.toml`**

Verify the existing `[dev-dependencies]` block has: `bee-adapter`, `bee-plugin-sdk`, `tokio` (full), `bincode`, `serde` (derive), `futures`. If `serde` is missing, add it. Run `cargo test -p bee-plugin-macro --test macro_handler_init_state 2>&1 | tail -5`. Expected: 1 test passes (Handler init_state is already supported at HEAD `54398cd`).

- [ ] **Step 1.3: If the test fails (RED), implement `init_state` slot support**

If the test fails, the stash's `crates/bee-plugin-macro/src/lib.rs` modifications are needed. Inspect the stash: `git show stash@{0}:crates/bee-plugin-macro/src/lib.rs | grep -A20 "init_state"` — verify `gen_handler` already handles the slot. If it does, the test should pass on HEAD. If not, port the stash's `gen_handler` changes to HEAD.

The expected macro behavior (already implemented per `54398cd`): when `gen_handler` finds a method with `#[bee_method(slot = "init_state")]`, it generates an `init_state` FFI fn that returns the bincode-encoded custom state, instead of the default empty `Vec<u8>`.

- [ ] **Step 1.4: Run the test (GREEN)**

Run: `cargo test -p bee-plugin-macro --test macro_handler_init_state 2>&1 | tail -5`. Expected: 1 test passes.

- [ ] **Step 1.5: Commit**

```bash
git add crates/bee-plugin-macro/tests/macro_handler_init_state.rs crates/bee-plugin-macro/Cargo.toml
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 1: lock down Handler init_state slot test"
```

---

## Task 2: Optional `open` / `close` slots (default-generated when not provided)

**Files:**
- Modify: `crates/bee-plugin-macro/src/lib.rs` (extend `gen_input_adapter`, `gen_output_adapter`)
- Create: `crates/bee-plugin-macro/tests/macro_default_open_close.rs`

Most plugins don't need a custom `open` (just accept empty config) or a custom `close` (just drop). Currently the macro requires both — the perf-fib `handle` adapter and mongodb adapters must hand-write stubs. This Task adds support for "if `open` / `close` not provided, the macro generates a default no-op implementation".

- [ ] **Step 2.1: Write the failing test (RED)**

Create `crates/bee-plugin-macro/tests/macro_default_open_close.rs`:

```rust
//! S33.6.1 Task 2: lock down optional
//! `open` / `close` slots. A no-open,
//! no-close adapter should still compile +
//! round-trip through the vtable.

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::InputAdapterVtable;

pub struct MinimalAdapter {
    emitted: u32,
}

impl MinimalAdapter {
    #[bee_adapter(input, name = "minimal")]
    // NOTE: no `#[bee_method(slot = "open")]` provided.
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        if self.emitted >= 2 {
            return Ok(None);
        }
        self.emitted += 1;
        Ok(Some(Event {
            timestamp: 0,
            sequence: self.emitted as u64,
            payload: vec![self.emitted as u8],
        }))
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_open_compiles_and_works() {
    // open() is auto-generated; passing empty
    // config should construct the adapter.
    let config: Vec<u8> = vec![];
    let ctx = unsafe {
        ((*MINIMAL_ADAPTER_VTABLE).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null(), "default open must not return null");
    let mut out = EventBytes::EMPTY;
    let rc = unsafe { ((*MINIMAL_ADAPTER_VTABLE).next)(ctx, &mut out) };
    assert_eq!(rc, 1, "first next must return 1 event");
    let rc = unsafe { ((*MINIMAL_ADAPTER_VTABLE).close)(ctx) };
    assert_eq!(rc, 0);
}
```

- [ ] **Step 2.2: Run the test to verify it fails (RED)**

Run: `cargo test -p bee-plugin-macro --test macro_default_open_close 2>&1 | tail -5`. Expected: compile error — the macro currently requires a method with `#[bee_method(slot = "open")]`.

- [ ] **Step 2.3: Extend `gen_input_adapter` to handle missing `open`**

In `crates/bee-plugin-macro/src/lib.rs`, modify `gen_input_adapter` so that:

- If no method has `#[bee_method(slot = "open")]`, do not error; generate a default `open_ffi` that constructs an empty adapter and stores it in the ctx. The adapter must be constructible from `Default` (or the struct must have a `Default` impl).
- If no method has `#[bee_method(slot = "close")]`, generate a default `close_ffi` that drops the adapter and returns 0.

The exact generated code shape (replace `open_fn` and `close_fn` extraction):

```rust
let has_custom_open = open_fn.is_some();
let has_custom_close = close_fn.is_some();
let open_rust = open_fn.as_ref().map(|f| f.sig.ident.clone());
let close_rust = close_fn.as_ref().map(|f| f.sig.ident.clone());

// Validate that custom open (if present) is async.
if let Some(f) = &open_fn {
    if f.sig.asyncness.is_none() {
        return err(
            "bee_adapter(input): `open` must be `async fn` (S33.6 signature check)",
        );
    }
}
```

And in the generated code, use the `Default` trait for the auto-generated open:

```rust
unsafe extern "C" fn #open_ffi(
    config_ptr: *const u8,
    config_len: usize,
    _err_out: *mut bee_plugin_sdk::event::EventBytes,
) -> *mut ::std::ffi::c_void {
    let adapter = if #has_custom_open {
        // user-supplied open
        let config = unsafe {
            ::std::slice::from_raw_parts(config_ptr, config_len).to_vec()
        };
        let fut = async move {
            <#struct_name>::#open_rust(config).await
        };
        match ::tokio::runtime::Handle::try_current() {
            Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
            Err(_) => ::futures::executor::block_on(fut),
        }
    } else {
        // default open: use Default::default()
        Ok(<#struct_name as ::std::default::Default>::default())
    };
    let adapter = match adapter {
        Ok(a) => a,
        Err(_) => return ::std::ptr::null_mut(),
    };
    let ctx = #ctx_ty {
        inner: ::tokio::sync::Mutex::new(Some(adapter)),
    };
    ::std::boxed::Box::into_raw(::std::boxed::Box::new(ctx))
        as *mut ::std::ffi::c_void
}
```

(Add `Default` bound check at the top of `gen_input_adapter`: if `!has_custom_open`, require `#struct_name: Default`.)

Similarly for `close`: if no custom close is provided, generate a `close_ffi` that just drops the `Option<#struct_name>` without calling a method:

```rust
unsafe extern "C" fn #close_ffi(ctx: *mut ::std::ffi::c_void) -> i32 {
    if ctx.is_null() { return 0; }
    let ctx = unsafe { ::std::boxed::Box::from_raw(ctx as *mut #ctx_ty) };
    let adapter = match ctx.inner.try_lock() {
        Ok(mut g) => g.take(),
        Err(_) => return -1,
    };
    if let Some(adapter) = adapter {
        if #has_custom_close {
            let fut = adapter.#close_rust();
            let _ = match ::tokio::runtime::Handle::try_current() {
                Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                Err(_) => ::futures::executor::block_on(fut),
            };
        }
        // else: just drop the adapter
    }
    0
}
```

- [ ] **Step 2.4: Apply the same extension to `gen_output_adapter`**

Mirror the change in `gen_output_adapter` (lines 781+ of `crates/bee-plugin-macro/src/lib.rs`). Same logic: optional `open` (default uses `Default::default()`) + optional `close`.

- [ ] **Step 2.5: Run the test (GREEN)**

Run: `cargo test -p bee-plugin-macro --test macro_default_open_close 2>&1 | tail -5`. Expected: 1 test passes.

- [ ] **Step 2.6: Re-run the existing 5 macro tests to verify no regression**

Run: `cargo test -p bee-plugin-macro 2>&1 | tail -10`. Expected: 6 tests pass (the original 5 + the new one).

- [ ] **Step 2.7: Run the full workspace to verify no plugin breakage**

Run: `cargo build --workspace 2>&1 | tail -5`. Expected: clean build (the existing refactored plugins — binance, google-news, influxdb, ta-lib, onnx-ml — all use the macro with custom `open`/`close`, so they should be unaffected).

- [ ] **Step 2.8: Commit**

```bash
git add crates/bee-plugin-macro/src/lib.rs crates/bee-plugin-macro/tests/macro_default_open_close.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 2: optional open/close slots (default-generated)"
```

---

## Task 3: Extend `emit` FFI to include `err_out` parameter

**Files:**
- Modify: `crates/bee-plugin-macro/src/lib.rs` (extend `gen_output_adapter`'s `emit` FFI)
- Modify: `crates/bee-plugin-macro/tests/macro_expands_output_adapter.rs` (update test for new signature)
- Create: `crates/bee-plugin-macro/tests/macro_emit_err_out.rs` (new test asserting err path)

The mongodb shim modules write error strings to an `err_out` parameter (`MongodbError::write_into`). The current macro-generated `emit` FFI does NOT have `err_out` — the macro discards `AdapterError::Emit`. This Task adds the `err_out` parameter so the macro-generated emit can write diagnostic events when the handler returns `Err`.

- [ ] **Step 3.1: Write the failing test (RED)**

Create `crates/bee-plugin-macro/tests/macro_emit_err_out.rs`:

```rust
//! S33.6.1 Task 3: lock down Output `emit`
//! with `err_out`. When the handler returns
//! `Err(AdapterError::Emit(msg))`, the FFI
//! writes an `Event { payload: msg.into_bytes() }`
//! to `err_out` and returns -1.

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::OutputAdapterVtable;

pub struct FailingOutput;

impl FailingOutput {
    #[bee_adapter(output, name = "fail-emit")]
    pub async fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self)
    }

    #[bee_method(slot = "emit")]
    pub async fn emit_one(
        &mut self,
        event: Event,
    ) -> AdapterResult<()> {
        if event.payload.is_empty() {
            return Err(AdapterError::Emit("empty payload".into()));
        }
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn emit_writes_err_to_err_out() {
    let config: Vec<u8> = vec![];
    let ctx = unsafe {
        ((*FAILING_OUTPUT_VTABLE).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null());

    // Send an event with empty payload — the handler
    // returns Err; the FFI writes to err_out and
    // returns -1.
    let event = Event {
        timestamp: 0,
        sequence: 1,
        payload: vec![],
    };
    let bytes = bincode::serialize(&event).unwrap();
    let mut err_out = EventBytes::EMPTY;
    let rc = unsafe {
        ((*FAILING_OUTPUT_VTABLE).emit)(
            ctx,
            bytes.as_ptr(),
            bytes.len(),
            &mut err_out,
        )
    };
    assert_eq!(rc, -1, "emit must return -1 on Err");
    assert!(err_out.len > 0, "err_out must be populated");
    let err_bytes = unsafe { std::slice::from_raw_parts(err_out.ptr, err_out.len) };
    let err_event: Event = bincode::deserialize(err_bytes)
        .expect("err_out must be a bincode-Event");
    let msg = String::from_utf8(err_event.payload).unwrap();
    assert!(msg.contains("empty payload"), "got err msg: {msg}");

    let rc = unsafe { ((*FAILING_OUTPUT_VTABLE).close)(ctx) };
    assert_eq!(rc, 0);
}
```

- [ ] **Step 3.2: Run the test to verify it fails (RED)**

Run: `cargo test -p bee-plugin-macro --test macro_emit_err_out 2>&1 | tail -5`. Expected: compile error — the macro's emit currently has signature `(ctx, ptr, len) -> i32`, but the test calls `(ctx, ptr, len, &mut err_out)`.

- [ ] **Step 3.3: Extend `gen_output_adapter`'s emit FFI**

In `crates/bee-plugin-macro/src/lib.rs`, in the `quote!` block inside `gen_output_adapter`, replace the `emit_ffi` body:

```rust
unsafe extern "C" fn #emit_ffi(
    ctx: *mut ::std::ffi::c_void,
    event_ptr: *const u8,
    event_len: usize,
    err_out: *mut bee_plugin_sdk::event::EventBytes,
) -> i32 {
    let ctx = unsafe { &*(ctx as *const #ctx_ty) };
    let bytes = unsafe {
        ::std::slice::from_raw_parts(event_ptr, event_len)
    };
    let event: bee_adapter::Event = match bincode::deserialize(bytes) {
        Ok(e) => e,
        Err(_) => return -1,
    };
    let mut guard = match ctx.inner.try_lock() {
        Ok(g) => g,
        Err(_) => return -1,
    };
    let adapter = match guard.as_mut() {
        Some(a) => a,
        None => return -1,
    };
    let fut = adapter.#emit_rust(event);
    let result = match ::tokio::runtime::Handle::try_current() {
        Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
        Err(_) => ::futures::executor::block_on(fut),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            // Write the error message to err_out as
            // an Event-shaped blob (so the host's
            // decoder picks it up).
            if !err_out.is_null() {
                let err_event = bee_adapter::Event {
                    timestamp: 0,
                    sequence: 0,
                    payload: format!("{e}").into_bytes(),
                };
                let bytes = bincode::serialize(&err_event).unwrap_or_default();
                let len = bytes.len();
                let ptr = bytes.as_ptr();
                ::std::mem::forget(bytes);
                unsafe {
                    *err_out = bee_plugin_sdk::event::EventBytes { ptr, len };
                }
            }
            -1
        }
    }
}
```

(The `AdapterError::Emit(String)` variant — the `format!("{e}")` call uses the `Display` impl which prints `"emit: <msg>"`. Adjust if the test expects the raw message.)

- [ ] **Step 3.4: Update the existing `macro_expands_output_adapter` test**

The existing test calls `((*MOCK_OUTPUT_VTABLE).emit)(ctx, bytes.as_ptr(), bytes.len())` — add a 4th arg `&mut EventBytes::EMPTY`. (The test's `emit_one` always returns `Ok(())`, so the err_out is unused but the signature changes.)

Edit `crates/bee-plugin-macro/tests/macro_expands_output_adapter.rs` at the emit call site (around line 757):

```rust
let rc = unsafe {
    ((*MOCK_OUTPUT_VTABLE).emit)(
        ctx,
        bytes.as_ptr(),
        bytes.len(),
        &mut EventBytes::EMPTY, // new 4th arg
    )
};
```

And add `use bee_plugin_sdk::event::EventBytes;` at the top if not already imported.

- [ ] **Step 3.5: Re-run all macro tests (GREEN)**

Run: `cargo test -p bee-plugin-macro 2>&1 | tail -10`. Expected: 7 tests pass (5 original + default_open_close + emit_err_out, plus the updated macro_expands_output_adapter).

- [ ] **Step 3.6: Verify `bee-plugin-influxdb` still builds**

Run: `cargo build -p bee-plugin-influxdb 2>&1 | tail -5`. Expected: clean build (the existing influxdb `emit` impl uses the macro; the new `err_out` arg is just an extra `&mut EventBytes::EMPTY` it can pass without changing its own code).

- [ ] **Step 3.7: Run the full workspace build**

Run: `cargo build --workspace 2>&1 | tail -5`. Expected: clean build.

- [ ] **Step 3.8: Commit**

```bash
git add crates/bee-plugin-macro/src/lib.rs crates/bee-plugin-macro/tests/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 3: Output emit gains err_out param for cross-FFI diagnostics"
```

---

## Task 4: Refactor `bee-plugin-perf-fib` using `#[bee_adapter]`

**Files:**
- Modify: `plugins/bee-plugin-perf-fib/Cargo.toml` (verify deps; mostly done in stash)
- Modify: `plugins/bee-plugin-perf-fib/src/lib.rs` (the macro refactor — already mostly in stash)
- Modify: `plugins/bee-plugin-perf-fib/tests/state.rs` (create — already in stash as untracked)

The stash (`stash@{0}`) already contains the refactored perf-fib. This Task ports the stash's changes to HEAD, verifies, and commits.

- [ ] **Step 4.1: Apply the stash's perf-fib changes on top of HEAD**

Use `git checkout stash@{0} -- plugins/bee-plugin-perf-fib/`. This pulls the Cargo.toml + src/lib.rs + tests/state.rs from the stash.

- [ ] **Step 4.2: Verify the refactor**

Read the resulting `plugins/bee-plugin-perf-fib/src/lib.rs`. Confirm:
- `pub struct FibStepHandler;` + `#[bee_adapter(handler, name = "fib_step")]` impl
- `pub struct FibSeedHandler;` + `#[bee_adapter(handler, name = "fib_seed")]` impl
- Both have `#[bee_method(slot = "init_state")]` returning the seed state
- `PerfFibFactory::init()` uses `register_vtable!` to register both handlers
- No `seed_shim` / `step_shim` modules

- [ ] **Step 4.3: Verify Cargo.toml**

`plugins/bee-plugin-perf-fib/Cargo.toml` must have:
- `bee-plugin-macro = { workspace = true }` (or via path)
- `bee-plugin-sdk = { workspace = true }`
- `bee-adapter = { workspace = true }`
- `serde`, `bincode`, `tokio`, `futures`
- `crate-type = ["cdylib", "rlib"]`
- The `description` field can stay or be removed (stash removes it)

- [ ] **Step 4.4: Build perf-fib**

Run: `cargo build -p bee-plugin-perf-fib 2>&1 | tail -10`. Expected: clean build (the macro generates the vtable glue; no hand-written shim modules).

- [ ] **Step 4.5: Run perf-fib tests**

Run: `cargo test -p bee-plugin-perf-fib 2>&1 | tail -10`. Expected: the new `tests/state.rs` test passes (asserts state round-trip via the vtable).

- [ ] **Step 4.6: Commit**

```bash
git add plugins/bee-plugin-perf-fib/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 4: bee-plugin-perf-fib refactor to #[bee_adapter]"
```

---

## Task 5: Refactor mongodb `InsertAdapter` (output, insert)

**Files:**
- Verify: `plugins/quant/bee-plugin-mongodb/Cargo.toml` (add `bee-plugin-macro` dep)
- Modify: `plugins/quant/bee-plugin-mongodb/src/lib.rs` (rewrite `mod insert_shim` as `#[bee_adapter(output, name = "insert")] pub struct InsertAdapter`)

This is the first of 5 sequential Tasks for the mongodb adapters. Each one replaces one `mod *_shim { ... }` block with a single `#[bee_adapter]` impl, **preserving the existing `do_*` business-logic helpers verbatim** (those stay unchanged in Section 7 / Section 8).

- [ ] **Step 5.1: Verify `Cargo.toml`**

Read `plugins/quant/bee-plugin-mongodb/Cargo.toml`. Confirm `bee-plugin-macro` is a dep. If missing, add:

```toml
bee-plugin-macro = { workspace = true }
```

- [ ] **Step 5.2: Write the failing test (RED)**

The mongodb unit tests at `plugins/quant/bee-plugin-mongodb/src/lib.rs::tests` (Section 11, line 1499+) already include `insert_args_round_trip` and similar. Verify they exist on HEAD; if not, add this test to a new file `plugins/quant/bee-plugin-mongodb/tests/insert_adapter.rs`:

```rust
//! S33.6.1 Task 5: lock down InsertAdapter
//! end-to-end via the macro-generated vtable.
//! The test decodes a synthetic Event payload
//! and asserts InsertAdapter::emit invokes
//! do_insert (we mock the Client).

use bee_adapter::{AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::OutputAdapterVtable;

// A test version of InsertAdapter that captures
// the emit calls in a Vec instead of hitting MongoDB.
pub struct TestInsertAdapter {
    pub emitted: std::sync::Mutex<Vec<Event>>,
}

#[bee_adapter(output, name = "test-insert")]
impl TestInsertAdapter {
    pub async fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self {
            emitted: std::sync::Mutex::new(vec![]),
        })
    }

    #[bee_method(slot = "emit")]
    pub async fn emit_one(&mut self, event: Event) -> AdapterResult<()> {
        self.emitted.lock().unwrap().push(event);
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_insert_adapter_vtable_round_trip() {
    let config: Vec<u8> = vec![];
    let ctx = unsafe {
        ((*TEST_INSERT_ADAPTER_VTABLE).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null());
    let event = Event { timestamp: 0, sequence: 1, payload: vec![1, 2, 3] };
    let bytes = bincode::serialize(&event).unwrap();
    let mut err_out = EventBytes::EMPTY;
    let rc = unsafe {
        ((*TEST_INSERT_ADAPTER_VTABLE).emit)(
            ctx, bytes.as_ptr(), bytes.len(), &mut err_out,
        )
    };
    assert_eq!(rc, 0);
    let rc = unsafe { ((*TEST_INSERT_ADAPTER_VTABLE).close)(ctx) };
    assert_eq!(rc, 0);
}
```

Add `bincode = "1"` to the test's deps via `[dev-dependencies]` in Cargo.toml if not already.

- [ ] **Step 5.3: Run the test (RED on macro build, GREEN on lib build)**

Run: `cargo test -p bee-plugin-mongodb --test insert_adapter 2>&1 | tail -10`. Expected: compiles (since the test uses the macro itself, not the mongodb InsertAdapter; this is a sanity check that the macro + test fixture interact correctly).

- [ ] **Step 5.4: Replace `mod insert_shim` with `InsertAdapter`**

In `plugins/quant/bee-plugin-mongodb/src/lib.rs`, replace the block from `mod insert_shim { ... }` (line 661 in HEAD) through the closing `}` (line 791) with:

```rust
// ---------------------------------------------------------------------------
// 10.1: `mongodb_insert` Output vtable — macro-generated
// ---------------------------------------------------------------------------

/// The `mongodb_insert` OutputAdapter. Holds a
/// shared `Arc<mongodb::Client>` (from the process-
/// global `OnceLock`), the database name from the
/// config, and the per-call args (which include
/// the collection — ADR-0010 per-call collection).
pub struct InsertAdapter {
    pub client: std::sync::Arc<mongodb::Client>,
    pub database: String,
    pub args: InsertArgs,
}

#[bee_adapter(output, name = "insert")]
impl InsertAdapter {
    /// Decode `MongodbConfig + InsertArgs` from
    /// the bincode-encoded `config` blob (sent by
    /// the host), acquire the shared `Client`,
    /// and build the adapter.
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let open_cfg: OpenConfig = bincode::deserialize(&config)
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("insert open: {e}")
            ))?;
        let client = acquire_client(&open_cfg.datasource).await
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("insert acquire_client: {e}")
            ))?;
        Ok(Self {
            client,
            database: open_cfg.datasource.database,
            args: open_cfg.stream,
        })
    }

    /// Decode the `Event` payload as a `Document`
    /// (BSON-encoded bytes inside `event.payload`),
    /// then call the existing `do_insert` helper.
    #[bee_method(slot = "emit")]
    pub async fn emit_one(&mut self, event: Event) -> AdapterResult<()> {
        let doc: bson::Document = bson::from_slice(&event.payload)
            .map_err(|e| bee_adapter::AdapterError::Emit(
                format!("insert bson decode: {e}")
            ))?;
        // Wrap the existing do_insert (which takes
        // &InsertArgs). Re-use the helper unchanged.
        let args = InsertArgs {
            collection: self.args.collection.clone(),
            document: event.payload.clone(),
        };
        do_insert(&self.client, &self.database, &args).await
            .map_err(|e| bee_adapter::AdapterError::Emit(
                format!("insert: {e}")
            ))?;
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        // Drop the Arc<Client> — the driver's pool
        // shuts down when the last reference drops.
        Ok(())
    }
}
```

Notes:
- The impl uses `bee_adapter::AdapterError` (already imported at line 110 of HEAD mongodb).
- The `OpenConfig { datasource: MongodbConfig, stream: InsertArgs }` type must exist in the file (it's already used by the shim's `open` at line 661+). Verify by reading lines 661-791.
- `acquire_client` is defined at HEAD line 394; do NOT touch it.
- `do_insert` is defined at HEAD line 419; do NOT touch it.

- [ ] **Step 5.5: Build mongodb**

Run: `cargo build -p bee-plugin-mongodb 2>&1 | tail -10`. Expected: compile error — `mod insert_shim` is gone, but lines 598-602 (the Factory) still reference `&insert_shim::VTABLE`. We'll fix the Factory in Task 10. For now, comment out the Factory's `let insert_vtable: ... = &insert_shim::VTABLE;` line (and the other 4 shim refs) with a placeholder:

```rust
// TODO(S33.6.1 Task 10): wire all 5 vtables via register_vtable!
let insert_vtable: *const OutputAdapterVtable = &INSERT_ADAPTER_VTABLE;
```

Apply the same for `insert_many`, `update`, `find`, `aggregate`.

- [ ] **Step 5.6: Run the mongodb tests**

Run: `cargo test -p bee-plugin-mongodb 2>&1 | tail -10`. Expected: the existing unit tests pass (sections 11+) — they're not coupled to the shim modules.

- [ ] **Step 5.7: Commit**

```bash
git add plugins/quant/bee-plugin-mongodb/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 5: bee-plugin-mongodb InsertAdapter via #[bee_adapter]"
```

---

## Task 6: Refactor mongodb `InsertManyAdapter` (output, insert_many)

**Files:**
- Modify: `plugins/quant/bee-plugin-mongodb/src/lib.rs` (replace `mod insert_many_shim` lines 791-909 with `InsertManyAdapter` impl)

Same shape as Task 5, but for batched insert. The existing `do_insert_many` helper at HEAD line 448 takes `&InsertManyArgs` (which has `documents: Vec<Vec<u8>>`).

- [ ] **Step 6.1: Replace `mod insert_many_shim` with `InsertManyAdapter`**

Replace the shim block with:

```rust
pub struct InsertManyAdapter {
    pub client: std::sync::Arc<mongodb::Client>,
    pub database: String,
    pub args: InsertManyArgs,
}

#[bee_adapter(output, name = "insert_many")]
impl InsertManyAdapter {
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let open_cfg: OpenConfigMany = bincode::deserialize(&config)
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("insert_many open: {e}")
            ))?;
        let client = acquire_client(&open_cfg.datasource).await
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("insert_many acquire_client: {e}")
            ))?;
        Ok(Self {
            client,
            database: open_cfg.datasource.database,
            args: open_cfg.stream,
        })
    }

    #[bee_method(slot = "emit")]
    pub async fn emit_one(&mut self, event: Event) -> AdapterResult<()> {
        // The host sends one event per batch; the
        // batch's documents are inside event.payload
        // as a Vec<Vec<u8>> bincode-encoded.
        let docs: Vec<Vec<u8>> = bincode::deserialize(&event.payload)
            .map_err(|e| bee_adapter::AdapterError::Emit(
                format!("insert_many decode: {e}")
            ))?;
        let args = InsertManyArgs {
            collection: self.args.collection.clone(),
            documents: docs,
        };
        do_insert_many(&self.client, &self.database, &args).await
            .map_err(|e| bee_adapter::AdapterError::Emit(
                format!("insert_many: {e}")
            ))?;
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}
```

(`OpenConfigMany { datasource: MongodbConfig, stream: InsertManyArgs }` mirrors `OpenConfig` from Task 5. Define it if not present.)

- [ ] **Step 6.2: Update the Factory placeholder (line 599)**

Change `let insert_many_vtable = &insert_many_shim::VTABLE;` to `&INSERT_MANY_ADAPTER_VTABLE;`.

- [ ] **Step 6.3: Build + test**

Run: `cargo build -p bee-plugin-mongodb 2>&1 | tail -5`. Expected: clean build (modulo the remaining 3 shim references for update/find/aggregate).

Run: `cargo test -p bee-plugin-mongodb 2>&1 | tail -5`. Expected: tests pass.

- [ ] **Step 6.4: Commit**

```bash
git add plugins/quant/bee-plugin-mongodb/src/lib.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 6: bee-plugin-mongodb InsertManyAdapter via #[bee_adapter]"
```

---

## Task 7: Refactor mongodb `UpdateAdapter` (output, update)

**Files:**
- Modify: `plugins/quant/bee-plugin-mongodb/src/lib.rs` (replace `mod update_shim` lines 910-1026 with `UpdateAdapter` impl)

Same shape as Task 5/6. The existing `do_update` helper at HEAD line 478 takes `&UpdateArgs { collection, filter, update }`.

- [ ] **Step 7.1: Replace `mod update_shim` with `UpdateAdapter`**

```rust
pub struct UpdateAdapter {
    pub client: std::sync::Arc<mongodb::Client>,
    pub database: String,
    pub args: UpdateArgs,
}

#[bee_adapter(output, name = "update")]
impl UpdateAdapter {
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let open_cfg: OpenConfigUpdate = bincode::deserialize(&config)
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("update open: {e}")
            ))?;
        let client = acquire_client(&open_cfg.datasource).await
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("update acquire_client: {e}")
            ))?;
        Ok(Self {
            client,
            database: open_cfg.datasource.database,
            args: open_cfg.stream,
        })
    }

    #[bee_method(slot = "emit")]
    pub async fn emit_one(&mut self, event: Event) -> AdapterResult<()> {
        // The host sends the (filter, update) pair
        // inside event.payload as a 2-tuple of
        // bincode-encoded BSON documents.
        let (filter, update): (Vec<u8>, Vec<u8>) =
            bincode::deserialize(&event.payload)
                .map_err(|e| bee_adapter::AdapterError::Emit(
                    format!("update decode: {e}")
                ))?;
        let args = UpdateArgs {
            collection: self.args.collection.clone(),
            filter,
            update,
        };
        do_update(&self.client, &self.database, &args).await
            .map_err(|e| bee_adapter::AdapterError::Emit(
                format!("update: {e}")
            ))?;
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        Ok(())
    }
}
```

- [ ] **Step 7.2: Update the Factory placeholder**

Change `&update_shim::VTABLE` → `&UPDATE_ADAPTER_VTABLE`.

- [ ] **Step 7.3: Build + test + commit**

```bash
cargo build -p bee-plugin-mongodb 2>&1 | tail -5
cargo test -p bee-plugin-mongodb 2>&1 | tail -5
git add plugins/quant/bee-plugin-mongodb/src/lib.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 7: bee-plugin-mongodb UpdateAdapter via #[bee_adapter]"
```

---

## Task 8: Refactor mongodb `FindAdapter` (input, find) — most complex

**Files:**
- Modify: `plugins/quant/bee-plugin-mongodb/src/lib.rs` (replace `mod find_shim` lines 1027-1267 with `FindAdapter` impl)

The Input adapters (`find` / `aggregate`) are more complex than the Output ones: they spawn a background tokio task that polls the collection on a cadence and pushes documents into an mpsc channel. The shim's `open` builds the channel + worker; `next` blocks on `rx.recv()`; `close` drops the worker.

The macro-generated `open` returns `Self` (the adapter, which holds `rx` + worker). The macro-generated `next` calls `&mut self`-receiving method that returns `AdapterResult<Option<Event>>`. The macro-generated `close` calls a `self`-receiving method.

- [ ] **Step 8.1: Replace `mod find_shim` with `FindAdapter`**

```rust
pub struct FindAdapter {
    /// The shared `Client` (kept alive while the
    /// worker task is polling).
    pub _client: std::sync::Arc<mongodb::Client>,
    /// Receiver end of the mpsc channel the
    /// background worker pushes documents into.
    pub rx: tokio::sync::mpsc::Receiver<DocumentEvent>,
    /// Join handle for the worker thread (drop on
    /// close to signal shutdown).
    pub _worker: Option<std::thread::JoinHandle<()>>,
    /// The live tokio runtime (kept alive so
    /// `next` can `block_on` the channel rx).
    pub runtime: Option<tokio::runtime::Runtime>,
}

#[bee_adapter(input, name = "find")]
impl FindAdapter {
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let open_cfg: OpenConfigFind = bincode::deserialize(&config)
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("find open: {e}")
            ))?;
        let client = acquire_client(&open_cfg.datasource).await
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("find acquire_client: {e}")
            ))?;
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let cadence = open_cfg.cadence_ms.unwrap_or(5000);
        let client_clone = client.clone();
        let db = open_cfg.datasource.database.clone();
        let coll = open_cfg.stream.collection.clone();
        let filter = open_cfg.stream.filter.clone();
        // Spawn a dedicated current-thread runtime on
        // a worker thread; the worker polls the
        // collection on `cadence` and pushes into
        // `tx`.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("find runtime: {e}")
            ))?;
        let worker = std::thread::spawn(move || {
            runtime.block_on(async move {
                let mut ticker = tokio::time::interval(
                    std::time::Duration::from_millis(cadence)
                );
                loop {
                    ticker.tick().await;
                    match do_find_all(&client_clone, &db, &coll, &filter).await {
                        Ok(docs) => {
                            for doc in docs {
                                let ev = DocumentEvent::from_document(doc);
                                if tx.send(ev).await.is_err() {
                                    break; // receiver dropped
                                }
                            }
                        }
                        Err(_e) => {
                            // log via tracing; continue polling
                        }
                    }
                }
            });
        });
        Ok(Self {
            _client: client,
            rx,
            _worker: Some(worker),
            runtime: None, // we use a fresh runtime per next() call below
        })
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        // block_on the mpsc receiver. The worker
        // pushes one DocumentEvent per poll cycle.
        let ev = self.rx.recv().await;
        match ev {
            Some(doc_ev) => Ok(Some(doc_ev.to_event())),
            None => Ok(None),
        }
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        // Drop the worker (the runtime is consumed
        // when the worker thread exits).
        drop(self._worker);
        Ok(())
    }
}
```

- [ ] **Step 8.2: Update the Factory placeholder**

Change `&find_shim::VTABLE` → `&FIND_ADAPTER_VTABLE`.

- [ ] **Step 8.3: Build + test + commit**

```bash
cargo build -p bee-plugin-mongodb 2>&1 | tail -5
cargo test -p bee-plugin-mongodb 2>&1 | tail -5
git add plugins/quant/bee-plugin-mongodb/src/lib.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 8: bee-plugin-mongodb FindAdapter via #[bee_adapter]"
```

---

## Task 9: Refactor mongodb `AggregateAdapter` (input, aggregate)

**Files:**
- Modify: `plugins/quant/bee-plugin-mongodb/src/lib.rs` (replace `mod aggregate_shim` lines 1268-1485 with `AggregateAdapter` impl)

Same shape as `FindAdapter` but uses `do_aggregate` (HEAD line 524+) instead of `do_find_all`. The per-call args are `AggregateArgs { collection, pipeline }` instead of `FindArgs { collection, filter }`.

- [ ] **Step 9.1: Replace `mod aggregate_shim` with `AggregateAdapter`**

```rust
pub struct AggregateAdapter {
    pub _client: std::sync::Arc<mongodb::Client>,
    pub rx: tokio::sync::mpsc::Receiver<DocumentEvent>,
    pub _worker: Option<std::thread::JoinHandle<()>>,
    pub runtime: Option<tokio::runtime::Runtime>,
}

#[bee_adapter(input, name = "aggregate")]
impl AggregateAdapter {
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let open_cfg: OpenConfigAggregate = bincode::deserialize(&config)
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("aggregate open: {e}")
            ))?;
        let client = acquire_client(&open_cfg.datasource).await
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("aggregate acquire_client: {e}")
            ))?;
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let cadence = open_cfg.cadence_ms.unwrap_or(30_000);
        let client_clone = client.clone();
        let db = open_cfg.datasource.database.clone();
        let coll = open_cfg.stream.collection.clone();
        let pipeline = open_cfg.stream.pipeline.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| bee_adapter::AdapterError::Open(
                format!("aggregate runtime: {e}")
            ))?;
        let worker = std::thread::spawn(move || {
            runtime.block_on(async move {
                let mut ticker = tokio::time::interval(
                    std::time::Duration::from_millis(cadence)
                );
                loop {
                    ticker.tick().await;
                    match do_aggregate(&client_clone, &db, &coll, &pipeline).await {
                        Ok(docs) => {
                            for doc in docs {
                                let ev = DocumentEvent::from_document(doc);
                                if tx.send(ev).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(_e) => { /* log + continue */ }
                    }
                }
            });
        });
        Ok(Self {
            _client: client,
            rx,
            _worker: Some(worker),
            runtime: None,
        })
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        match self.rx.recv().await {
            Some(doc_ev) => Ok(Some(doc_ev.to_event())),
            None => Ok(None),
        }
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> {
        drop(self._worker);
        Ok(())
    }
}
```

- [ ] **Step 9.2: Update the Factory placeholder**

Change `&aggregate_shim::VTABLE` → `&AGGREGATE_ADAPTER_VTABLE`.

- [ ] **Step 9.3: Build + test + commit**

```bash
cargo build -p bee-plugin-mongodb 2>&1 | tail -5
cargo test -p bee-plugin-mongodb 2>&1 | tail -5
git add plugins/quant/bee-plugin-mongodb/src/lib.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 9: bee-plugin-mongodb AggregateAdapter via #[bee_adapter]"
```

---

## Task 10: Wire `MongodbFactory::init()` via `register_vtable!` + delete remaining shim modules

**Files:**
- Modify: `plugins/quant/bee-plugin-mongodb/src/lib.rs` (Section 9 — `MongodbFactory::init()` + delete Section 10 shim modules)

Now that all 5 adapters use `#[bee_adapter]`, wire the Factory and delete the `unsafe extern "C" fn` blocks that the macros replaced. The 5 `mod *_shim` blocks total ~830 LOC of hand-written glue.

- [ ] **Step 10.1: Rewrite `MongodbFactory::init()`**

In `plugins/quant/bee-plugin-mongodb/src/lib.rs`, replace the `impl Factory for MongodbFactory` block (Section 9, around line 588-624) with:

```rust
pub struct MongodbFactory;

impl Factory for MongodbFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> PluginResult<PluginHandle> {
        let mut input_adapters = std::collections::HashMap::new();
        let mut output_adapters = std::collections::HashMap::new();
        let mut handlers = std::collections::HashMap::new();

        register_vtable! {
            input_adapters, output_adapters, handlers;
            output "insert"      => &INSERT_ADAPTER_VTABLE,
            output "insert_many" => &INSERT_MANY_ADAPTER_VTABLE,
            output "update"      => &UPDATE_ADAPTER_VTABLE,
            input  "find"        => &FIND_ADAPTER_VTABLE,
            input  "aggregate"   => &AGGREGATE_ADAPTER_VTABLE,
        }

        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}
```

- [ ] **Step 10.2: Delete the 5 shim modules**

In `plugins/quant/bee-plugin-mongodb/src/lib.rs`, remove the lines that previously contained:
- `mod insert_shim { ... }` (already replaced in Task 5)
- `mod insert_many_shim { ... }` (already replaced in Task 6)
- `mod update_shim { ... }` (already replaced in Task 7)
- `mod find_shim { ... }` (already replaced in Task 8)
- `mod aggregate_shim { ... }` (already replaced in Task 9)

Verify by `grep -n "^mod .*_shim" plugins/quant/bee-plugin-mongodb/src/lib.rs`. Expected: 0 matches.

- [ ] **Step 10.3: Build the workspace**

Run: `cargo build --workspace 2>&1 | tail -10`. Expected: clean build (all 5 shim modules gone, all 5 macro-generated vtables wired).

- [ ] **Step 10.4: Run the full workspace test suite**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6} END{print "passed:", p, "failed:", f}'`. Expected: `passed: 415+ failed: 0` (must equal or exceed the baseline of 415).

- [ ] **Step 10.5: Diff sanity check**

Run: `git diff HEAD~10..HEAD --stat -- plugins/quant/bee-plugin-mongodb/src/lib.rs`. Expected: net reduction of ~600-900 LOC (5 shim modules deleted; 5 small `#[bee_adapter]` impls added).

- [ ] **Step 10.6: Commit**

```bash
git add plugins/quant/bee-plugin-mongodb/src/lib.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1 Task 10: MongodbFactory uses register_vtable! + delete shim modules"
```

---

## Task 11: Final verification + sign-off

- [ ] **Step 11.1: Full workspace build**

Run: `cargo build --workspace 2>&1 | tail -5`. Expected: clean build.

- [ ] **Step 11.2: Full workspace test suite**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6} END{print "passed:", p, "failed:", f}'`. Expected: `passed: ≥415 failed: 0`.

- [ ] **Step 11.3: Verify acceptance criteria**

- [ ] `cargo build --workspace` green ✓
- [ ] `cargo test --workspace` ≥ 415 passed, 0 failed ✓
- [ ] `cargo test -p bee-plugin-macro` ≥ 6 tests pass (5 original + default_open_close + emit_err_out, plus updated macro_expands_output_adapter)
- [ ] `cargo test -p bee-plugin-perf-fib` ≥ 1 test pass (state round-trip)
- [ ] `cargo test -p bee-plugin-mongodb` ≥ 1 test pass (existing unit tests still pass)
- [ ] `plugins/quant/bee-plugin-mongodb/src/lib.rs` has 0 `mod *_shim` blocks ✓
- [ ] `plugins/quant/bee-plugin-mongodb/src/lib.rs` `MongodbFactory::init()` uses `register_vtable!` ✓
- [ ] `plugins/bee-plugin-perf-fib/src/lib.rs` has 0 `mod seed_shim` / `mod step_shim` blocks ✓
- [ ] `plugins/bee-plugin-perf-fib/src/lib.rs` `PerfFibFactory::init()` uses `register_vtable!` ✓

- [ ] **Step 11.4: Update S33.6.1 spec acceptance criteria**

Edit `docs/superpowers/specs/2026-07-17-s33-6-1-mongodb-perf-fib-macro-refactor-design.md` and flip all `[ ]` to `[x]`. Commit:

```bash
git add docs/superpowers/specs/2026-07-17-s33-6-1-mongodb-perf-fib-macro-refactor-design.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6.1: flip acceptance criteria to [x]"
```

- [ ] **Step 11.5: Push to remote**

```bash
git push origin main
```

- [ ] **Step 11.6: Hand off the out-of-scope stash**

The user has 4+ other workstreams in `stash@{0}` that are NOT part of S33.6.1:
1. DSL `CREATE SINK` preprocessor (S29+ sink DSL story)
2. `crates/bee-plugin-sdk/src/kv.rs` host-side KV FFI hook (S30 FFI follow-up)
3. `prime_sieve.sql` 3707→73 line trim (S41 demo cleanup)
4. `binance/src/lib.rs` re-touch (just discard — already at HEAD)
5. `docs/book/` mdbook build output (`.gitignore` cleanup)

Tell the user: "S33.6.1 done; the other 4 stash items need their own stories. Don't `git stash drop` yet."

---

## Self-Review

**1. Spec coverage**: Walked the spec's 3 in-scope items + 5 out-of-scope items:
- Macro improvements (Tasks 1, 2, 3) ✓
- perf-fib refactor (Task 4) ✓
- mongodb refactor for all 5 adapters (Tasks 5-9) + Factory + shim delete (Task 10) ✓
- Out-of-scope stash items explicitly listed in Task 11.6 ✓

**2. Placeholder scan**: Searched for "TBD", "TODO", "implement later" — only one `TODO(S33.6.1 Task 10)` which is intentional (a temporary marker in Task 5.5 that gets replaced in Task 10.1). All other steps have explicit code or commands.

**3. Type consistency**: 
- `InsertAdapter`, `InsertManyAdapter`, `UpdateAdapter`, `FindAdapter`, `AggregateAdapter` names consistent across Tasks 5-10
- VTable constants `INSERT_ADAPTER_VTABLE`, etc., consistent
- `OpenConfig`, `OpenConfigMany`, `OpenConfigUpdate`, `OpenConfigFind`, `OpenConfigAggregate` types named consistently
- `do_insert`, `do_insert_many`, `do_update`, `do_find_all`, `do_aggregate` helpers referenced by name (defined at HEAD lines 419, 448, 478, 524, etc.) — these stay unchanged
- `register_vtable!` sub-macro signature (`input_adapters, output_adapters, handlers;`) matches the S33.6 Task 5 design

**4. Ambiguity check**: All acceptance criteria in the spec map to specific Tasks (Task 11.3 is the consolidated checklist). No ambiguous requirements.

---

## Estimated Total

- 11 Tasks
- ~30-40 commits (Tasks 1-10 each have multiple sub-commits; Task 11 has 3 verification commits)
- ~700-900 LOC net change in mongodb (-830 shim glue + ~120 macro adapter impls)
- ~250 LOC net change in perf-fib (-280 shim glue + ~30 macro handler impls)
- ~80 LOC net change in `bee-plugin-macro` (Tasks 1-3 extensions + tests)
- Estimated wall-clock: 2-4 hours of focused implementation, TDD-style with frequent commits
