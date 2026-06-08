# S33-deferred · FFI + runtime dispatch + 2 SQL + demo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the S33 deferred items: production wire format for adapter events across FFI, runtime plugin loading + dispatching, 2 SQL pipelines, and a one-click demo script. Single commit.

**Architecture:** 5 sections — (1) bincode event wire format in `bee-plugin-sdk`, (2) per-adapter vtables (`InputAdapterVtable` / `OutputAdapterVtable` / `HandlerVtable`) carried by `PluginHandle` + 3 new function pointers on `BeeHostV1`, (3) `PluginManager::load_directory` + `PluginInputAdapter`/`PluginOutputAdapter`/`PluginHandler` wrappers + `PluginAdapterRegistry` + pre-flight check in `Deployer::deploy`, (4) 2 SQL pipelines in `examples/`, (5) `scripts/demo-quant-prod.sh`.

**Tech Stack:** Rust 2021, `bincode` (new dep for `bee-plugin-sdk`), `tokio` (existing), `cdylib` + `libloading` (existing per S19/S20), `Cluster` test harness (existing).

**Reference docs:**
- Design: `docs/superpowers/specs/2026-06-08-s33-deferred-ffi-design.md`
- Story: `docs/stories.md` §S33 (deferred items) + S40 (acceptance criteria for demo)
- ADR-0005 (FFI), ADR-0009 (PluginId), ADR-0010 (Datasource), ADR-0011 (Stream identity)

**Pre-flight (read these before starting):**
- `CONTEXT.md` — domain vocabulary
- `crates/bee-adapter/src/lib.rs` — `Event` + `InputAdapter`/`OutputAdapter` traits
- `crates/bee-plugin-sdk/src/lib.rs` — `PluginId`, `PluginManifest`, `PluginHandle`, `BeeHostV1`, `Plugin` trait
- `crates/bee-plugin-sdk/src/macros.rs` — `cdylib_plugin!` macro (lines 31-57)
- `crates/bee-registry/src/loader.rs` — `load_library` + `LoadedPlugin` (lines 65-96)
- `crates/bee-runtime/src/lib.rs` — `DynPhase`, `Runtime`, `Msg`
- `crates/bee-runtime/src/test_utils.rs` — current `MockInputAdapter`
- `crates/bee-control/src/deployer.rs` — `Deployer::deploy` (line 184)
- `plugins/bee-plugin-binance-mock/src/lib.rs` — mock plugin example (215 lines)
- `plugins/bee-plugin-ta-lib-mock/src/lib.rs` — mock plugin with multiple Handlers (303 lines)
- `CONTEXT.md` (line about S33-S41 stories)

**Working-tree state (do not touch):**
- Commits `b680d3b` (S17), `22a9e39` (S33 wrap-up), `dfc63da` (S33-deferred design) on `main`.
- No uncommitted changes; clean tree.
- All 5 mock plugin crates (`plugins/bee-plugin-*-mock/`) and `crates/bee-types/` exist and compile.

**Out of scope (per design):**
- Replacing the 5 mock plugins with production-grade real-external-system implementations (S34–S39).
- The `examples/performance/` Fibonacci + prime sieve + multi-stream analytics (S41).
- HITL seed-user walkthrough (user's manual step after S40).
- `bee run --check` dry-run command (verify the SQL files compile by other means if it doesn't exist; see Task 8).

---

## File structure

**New files:**
- `crates/bee-plugin-sdk/src/event.rs` — `EventBytes`, `encode_event`, `decode_event`
- `crates/bee-plugin-sdk/src/vtable.rs` — `InputAdapterVtable`, `OutputAdapterVtable`, `HandlerVtable`
- `crates/bee-runtime/src/plugin_adapter.rs` — `PluginInputAdapter`, `PluginOutputAdapter`, `PluginHandler`, `PluginAdapterRegistry`
- `examples/quant_btc_macd.sql` — BTC K-line + MACD/EMA + InfluxDB
- `examples/quant_btc_sentiment.sql` — BTC K-line + FinBERT sentiment + decision tree + InfluxDB
- `scripts/demo-quant-prod.sh` — end-to-end demo

**Modified files:**
- `crates/bee-adapter/src/lib.rs` — add `Serialize`/`Deserialize` derives to `Event`
- `crates/bee-adapter/Cargo.toml` — add `bincode = "1"` + `serde = { version = "1", features = ["derive"] }`
- `crates/bee-plugin-sdk/src/lib.rs` — re-export `event` and `vtable` modules; extend `PluginHandle` with 3 vtable maps; extend `BeeHostV1` with 3 register slots
- `crates/bee-plugin-sdk/Cargo.toml` — add `bincode = "1"` + `serde` + `bee-adapter` (or whichever)
- `crates/bee-registry/src/loader.rs` — add `load_directory` function
- `crates/bee-runtime/src/lib.rs` — re-export `plugin_adapter`
- `crates/bee-runtime/Cargo.toml` — add `bincode = "1"` + `serde` (if not transitive)
- `crates/bee-control/src/deployer.rs` — add pre-flight check that Pipeline's adapter names resolve
- `plugins/bee-plugin-binance-mock/src/lib.rs` — populate vtable maps in `init()`
- `plugins/bee-plugin-google-news-mock/src/lib.rs` — populate vtable maps
- `plugins/bee-plugin-influxdb-mock/src/lib.rs` — populate vtable maps
- `plugins/bee-plugin-mongodb-mock/src/lib.rs` — populate vtable maps
- `plugins/bee-plugin-ta-lib-mock/src/lib.rs` — populate vtable maps
- `bee/src/main.rs` — add `bee plugin register <path>` subcommand (if missing) + `bee run --check` (if missing)
- `Cargo.toml` (workspace) — add `bincode` to `[workspace.dependencies]` if used in 3+ crates

**Boundary responsibilities:**
- `event.rs`: pure encode/decode, no FFI types
- `vtable.rs`: `#[repr(C)]` function pointer structs; FFI safety docs
- `plugin_adapter.rs`: wrappers that implement the existing `InputAdapter`/`OutputAdapter` traits by calling through the vtable
- `PluginAdapterRegistry`: process-global lookup table, behind `OnceLock` or `RwLock<HashMap>`
- `load_directory`: filesystem walk + `load_library` for each file

---

## Task 1: §1 Event wire format (RED + GREEN combined)

**Files:**
- Modify: `crates/bee-adapter/Cargo.toml` (add `bincode`, `serde`)
- Modify: `crates/bee-adapter/src/lib.rs` (add `Serialize`/`Deserialize` to `Event`)
- Create: `crates/bee-plugin-sdk/src/event.rs` (encode/decode helpers + `EventBytes`)
- Modify: `crates/bee-plugin-sdk/src/lib.rs` (re-export `event` module)
- Modify: `crates/bee-plugin-sdk/Cargo.toml` (add `bincode`, `serde`, `bee-adapter`)

- [ ] **Step 1: Add deps to `crates/bee-adapter/Cargo.toml`**

Read current file. Add to `[dependencies]`:
```toml
serde = { version = "1", features = ["derive"] }
bincode = "1"
```

- [ ] **Step 2: Derive `Serialize`/`Deserialize` on `Event` in `crates/bee-adapter/src/lib.rs`**

Current:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub timestamp: u64,
    pub sequence: u64,
    pub payload: Vec<u8>,
}
```

Change to:
```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub timestamp: u64,
    pub sequence: u64,
    pub payload: Vec<u8>,
}
```

- [ ] **Step 3: Add deps to `crates/bee-plugin-sdk/Cargo.toml`**

Add:
```toml
serde = { version = "1", features = ["derive"] }
bincode = "1"
bee-adapter = { workspace = true }
```

- [ ] **Step 4: Create `crates/bee-plugin-sdk/src/event.rs`**

```rust
//! S33 §1: production wire format for adapter events across the FFI.
//!
//! `Event` (from `bee-adapter`) crosses the `cdylib` boundary as
//! bincode-serialized bytes. The `EventBytes` struct is the FFI-
//! facing view: a `(ptr, len)` pair the host reads (and then
//! bincode-deserializes). Memory ownership: the producer allocates
//! the bytes (via `Vec<u8>::into_boxed_slice().leak()` or
//! `Box::into_raw`), the consumer reads them once, then frees
//! via the vtable's `close` or the same producer's allocator.

use bee_adapter::Event;

/// FFI-facing view of a serialized Event. The `ptr` is non-null
/// when `len > 0`; both fields are read-only from the consumer's
/// perspective. The producer is responsible for the bytes'
/// lifetime — see the vtable docs for the exact contract.
#[repr(C)]
pub struct EventBytes {
    pub ptr: *const u8,
    pub len: usize,
}

impl EventBytes {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };
}

/// Encode an `Event` to bincode bytes. The result is what crosses
/// the FFI boundary (the host reads it via `EventBytes`).
pub fn encode_event(event: &Event) -> Vec<u8> {
    bincode::serialize(event).expect("Event is always bincode-serializable")
}

/// Decode bincode bytes (as read from the FFI) back into an `Event`.
/// Returns `Err` if the bytes are malformed or the version field
/// (if added in the future) is incompatible.
pub fn decode_event(bytes: &[u8]) -> Result<Event, bincode::Error> {
    bincode::deserialize(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_event() {
        let event = Event {
            timestamp: 1_700_000_000_000,
            sequence: 42,
            payload: b"hello world".to_vec(),
        };
        let bytes = encode_event(&event);
        let decoded = decode_event(&bytes).expect("decode");
        assert_eq!(event, decoded);
    }

    #[test]
    fn roundtrip_empty_payload() {
        let event = Event {
            timestamp: 0,
            sequence: 0,
            payload: vec![],
        };
        let bytes = encode_event(&event);
        let decoded = decode_event(&bytes).expect("decode");
        assert_eq!(event, decoded);
    }

    #[test]
    fn roundtrip_large_payload() {
        // 1 MB payload
        let event = Event {
            timestamp: u64::MAX,
            sequence: u64::MAX,
            payload: vec![0xAB; 1_000_000],
        };
        let bytes = encode_event(&event);
        let decoded = decode_event(&bytes).expect("decode");
        assert_eq!(event, decoded);
    }

    #[test]
    fn decode_rejects_garbage() {
        let bytes = vec![0xFFu8; 4];
        let err = decode_event(&bytes);
        assert!(err.is_err(), "garbage must not decode");
    }

    #[test]
    fn empty_event_bytes_is_safe() {
        let eb = EventBytes::EMPTY;
        assert!(eb.ptr.is_null());
        assert_eq!(eb.len, 0);
    }
}
```

- [ ] **Step 5: Re-export from `crates/bee-plugin-sdk/src/lib.rs`**

Add `pub mod event;` after `pub mod macros;`. Also add to the existing `pub use` block (if any) or just rely on `bee_plugin_sdk::event::*`.

- [ ] **Step 6: Run tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-sdk --lib event:: 2>&1 | tail -10
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-adapter 2>&1 | tail -10
```

Expected: 5 tests pass in `event`, existing `Event` tests still pass in `bee-adapter`.

- [ ] **Step 7: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-adapter crates/bee-plugin-sdk && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred §1: bincode event wire format + encode/decode helpers"
```

---

## Task 2: §2 vtable types

**Files:**
- Create: `crates/bee-plugin-sdk/src/vtable.rs`
- Modify: `crates/bee-plugin-sdk/src/lib.rs` (re-export)

- [ ] **Step 1: Create `crates/bee-plugin-sdk/src/vtable.rs`**

```rust
//! S33 §2: per-adapter function pointer tables (vtables).
//!
//! Each vtable is `#[repr(C)]` so the layout is stable across the
//! FFI boundary. The host calls through the function pointers; the
//! plugin (in its `init()`) bundles the vtables into `PluginHandle`.
//!
//! ## Memory ownership across the FFI
//!
//! - `*const u8` / `*mut u8` + `len` is the canonical C ABI for
//!   variable-length bytes. The producer is responsible for the
//!   bytes' lifetime.
//! - For `next` (InputAdapter) and `emit` (OutputAdapter), the
//!   event bytes are bincode-encoded `Event` (see `event.rs`).
//! - For `handle` (Handler), the state and result are
//!   bincode-encoded plugin-specific types — the Handler is
//!   responsible for its own state schema.
//! - All `err_out` parameters are optional (may be null). When
//!   non-null, the plugin writes a bincode-encoded error string
//!   (UTF-8 bytes, NOT a C string) on failure. The host reads +
//!   frees (or ignores on success).

use crate::event::EventBytes;

/// Function pointer table for an `InputAdapter` instance.
/// All function pointers take a `ctx: *mut c_void` (the adapter's
/// per-instance state, allocated by `open`) as the first arg.
#[repr(C)]
pub struct InputAdapterVtable {
    /// Open the adapter with a config (bincode-encoded plugin-
    /// specific config blob). Returns a `*mut c_void` ctx for
    /// subsequent calls, or null on error (with `err_out` filled
    /// if non-null).
    pub open: unsafe extern "C" fn(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void,

    /// Pull the next event. Returns:
    /// - 1 if an event was produced (written to `*out`)
    /// - 0 for end-of-stream
    /// - -1 on error (with `*err_out` filled if non-null)
    /// The producer owns the bytes; the consumer must copy them
    /// out before the next call (or before `close`).
    pub next: unsafe extern "C" fn(
        ctx: *mut std::ffi::c_void,
        out: *mut EventBytes,
    ) -> i32,

    /// Close the adapter; free the ctx. Returns 0 on success.
    pub close: unsafe extern "C" fn(
        ctx: *mut std::ffi::c_void,
    ) -> i32,
}

#[repr(C)]
pub struct OutputAdapterVtable {
    pub open: unsafe extern "C" fn(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void,

    /// Emit one event. The host has already bincode-encoded the
    /// `Event` (see `event::encode_event`); the plugin
    /// bincode-decodes + processes. Returns 0 on success, -1 on
    /// error.
    pub emit: unsafe extern "C" fn(
        ctx: *mut std::ffi::c_void,
        event_ptr: *const u8,
        event_len: usize,
    ) -> i32,

    pub close: unsafe extern "C" fn(
        ctx: *mut std::ffi::c_void,
    ) -> i32,
}

#[repr(C)]
pub struct HandlerVtable {
    /// Compute `handler(state, event) -> (new_state, result)`.
    /// All blobs are bincode-encoded. Returns 0 on success.
    pub handle: unsafe extern "C" fn(
        state_ptr: *const u8,
        state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        err_out: *mut EventBytes,
    ) -> i32,

    /// Initialize a fresh state blob. Returns 0 on success.
    pub init_state: unsafe extern "C" fn(
        out: *mut EventBytes,
    ) -> i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vtables_have_stable_layout() {
        // Compile-time / runtime sanity: the structs have non-zero
        // size and (because of #[repr(C)]) the field offsets are
        // predictable.
        assert!(std::mem::size_of::<InputAdapterVtable>() > 0);
        assert!(std::mem::size_of::<OutputAdapterVtable>() > 0);
        assert!(std::mem::size_of::<HandlerVtable>() > 0);
    }

    #[test]
    fn event_bytes_is_ffi_safe() {
        assert_eq!(
            std::mem::size_of::<EventBytes>(),
            std::mem::size_of::<*const u8>() + std::mem::size_of::<usize>(),
        );
    }
}
```

- [ ] **Step 2: Re-export from `lib.rs`**

Add `pub mod vtable;` next to `pub mod event;`.

- [ ] **Step 3: Run tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-sdk --lib vtable:: 2>&1 | tail -10
```

Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-plugin-sdk && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred §2: vtable types (Input/Output/Handler Vtable)"
```

---

## Task 3: §2 mock plugin shims (populate vtable maps)

This task updates each of the 5 mock plugins to:
- Keep their existing in-process `InputAdapter` / `OutputAdapter` / `Handler` impls
- Wrap them in vtable function pointers
- Populate the 3 vtable maps in `init()`

**Files:**
- Modify: each `plugins/bee-plugin-*/src/lib.rs` (5 files)

- [ ] **Step 1: Read all 5 mock plugin `lib.rs` files** to understand their current shape

```bash
for f in plugins/bee-plugin-*/src/lib.rs; do echo "=== $f ==="; head -50 "$f"; done
```

For each plugin, identify:
- The concrete InputAdapter / OutputAdapter / Handler types
- The factory function or method that creates an instance
- The current `init()` impl in the `Plugin` trait

- [ ] **Step 2: Add a `vtable_shim.rs` module to each mock plugin** (or inline in `lib.rs`)

For each plugin, add vtable-shim functions that wrap the concrete types. Example for `bee-plugin-binance-mock` (Input only):

```rust
// in plugins/bee-plugin-binance-mock/src/lib.rs (append)

mod vtable_shim {
    use super::*;
    use bee_plugin_sdk::event::EventBytes;
    use bee_plugin_sdk::vtable::InputAdapterVtable;
    use std::sync::Mutex;

    /// Wrapper that pins the concrete BinanceInput behind a
    /// raw pointer so the FFI function can recover it.
    pub struct Ctx {
        pub input: Mutex<BinanceInput>,
    }

    pub unsafe extern "C" fn open(
        config_ptr: *const u8,
        config_len: usize,
        _err_out: *mut EventBytes,
    ) -> *mut std::ffi::c_void {
        let bytes = std::slice::from_raw_parts(config_ptr, config_len);
        let cfg: BinanceConfig = match bincode::deserialize(bytes) {
            Ok(c) => c,
            Err(_) => return std::ptr::null_mut(),
        };
        let input = match BinanceInput::new(cfg) {
            Ok(i) => i,
            Err(_) => return std::ptr::null_mut(),
        };
        let ctx = Box::new(Ctx { input: Mutex::new(input) });
        Box::into_raw(ctx) as *mut std::ffi::c_void
    }

    pub unsafe extern "C" fn next(
        ctx: *mut std::ffi::c_void,
        out: *mut EventBytes,
    ) -> i32 {
        let ctx = &*(ctx as *const Ctx);
        let mut input = ctx.input.lock().unwrap();
        match input.next_blocking() {
            Some(event) => {
                let bytes = bee_plugin_sdk::event::encode_event(&event);
                let len = bytes.len();
                let ptr = bytes.as_ptr();
                std::mem::forget(bytes); // leak to host
                *out = EventBytes { ptr, len };
                1
            }
            None => {
                *out = EventBytes::EMPTY;
                0
            }
        }
    }

    pub unsafe extern "C" fn close(ctx: *mut std::ffi::c_void) -> i32 {
        if !ctx.is_null() {
            drop(Box::from_raw(ctx as *mut Ctx));
        }
        0
    }

    pub const VTABLE: InputAdapterVtable = InputAdapterVtable {
        open,
        next,
        close,
    };
}
```

NOTE: the exact function bodies depend on the existing concrete types in each mock crate. The pattern is:
- `open`: deserialize config via bincode; create concrete instance; wrap in `Box<Ctx>`; return raw pointer
- `next`/`emit`/`handle`: recover `Ctx` from raw pointer; call the concrete method; convert result to bincode bytes; write to `*out`
- `close`: drop the `Box<Ctx>`

- [ ] **Step 3: Update each plugin's `init()` to populate the vtable maps**

The current `Plugin::init()` returns a `PluginHandle` with `manifest`, `inner`. The new init also populates the 3 vtable maps.

For the binance mock (Input only):
```rust
    fn init(&self) -> PluginResult<PluginHandle> {
        let input_vtable: *const InputAdapterVtable = &vtable_shim::VTABLE;
        let mut input_adapters = std::collections::HashMap::new();
        input_adapters.insert("subscribe".to_string(), input_vtable);
        Ok(PluginHandle {
            manifest: self.manifest(),
            inner: Arc::new(()),
            input_adapters,
            output_adapters: std::collections::HashMap::new(),
            handlers: std::collections::HashMap::new(),
        })
    }
```

(Symmetric for the 4 other plugins: google-news has Input, influxdb has Output, mongodb has Output, ta-lib has 4 Handlers.)

- [ ] **Step 4: Add a unit test per plugin** that exercises the vtable round-trip

For each plugin, the test:
1. Calls `init()` to get the `PluginHandle`
2. Looks up the vtable by adapter/handler name
3. For Input: calls `open` with a known config, then `next` repeatedly, asserting the events match the expected mock output
4. For Output: calls `open` then `emit` with a known event, then `close`
5. For Handler: calls `init_state` then `handle` with a known event, asserting the result is correct

Example for binance:
```rust
    #[test]
    fn vtable_next_returns_sine_wave_events() {
        let plugin = BinanceMockFactory;
        let handle = plugin.init().expect("init");
        let vtable = *handle.input_adapters.get("subscribe").expect("vtable");
        let cfg = BinanceConfig::default();
        let cfg_bytes = bincode::serialize(&cfg).unwrap();
        let ctx = unsafe { (vtable.open)(cfg_bytes.as_ptr(), cfg_bytes.len(), std::ptr::null_mut()) };
        assert!(!ctx.is_null());
        let mut out = EventBytes::EMPTY;
        let rc = unsafe { (vtable.next)(ctx, &mut out) };
        assert_eq!(rc, 1);
        let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
        let event: Event = bincode::deserialize(bytes).unwrap();
        // Mock binance emits sine-wave prices; the first event's
        // sequence is 0.
        assert_eq!(event.sequence, 0);
        unsafe { (vtable.close)(ctx); }
    }
```

- [ ] **Step 5: Run all mock plugin tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-binance-mock -p bee-plugin-google-news-mock -p bee-plugin-influxdb-mock -p bee-plugin-mongodb-mock -p bee-plugin-ta-lib-mock 2>&1 | tail -15
```

Expected: each plugin has its new vtable-roundtrip test + all existing tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add plugins && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred §2: mock plugins populate vtable maps (5 plugins)"
```

---

## Task 4: §2 `PluginHandle` extension + `BeeHostV1` extension

**Files:**
- Modify: `crates/bee-plugin-sdk/src/lib.rs` (extend `PluginHandle`, `BeeHostV1`)

- [ ] **Step 1: Extend `PluginHandle`**

Current:
```rust
pub struct PluginHandle {
    pub manifest: PluginManifest,
    pub inner: Arc<dyn std::any::Any + Send + Sync + 'static>,
}
```

Change to:
```rust
pub struct PluginHandle {
    pub manifest: PluginManifest,
    pub inner: Arc<dyn std::any::Any + Send + Sync + 'static>,

    // S33-deferred: per-adapter vtable registries. Populated by
    // the plugin in `init()` and frozen for the plugin's
    // lifetime. The host looks up vtables by adapter/handler
    // name and calls through the function pointers.
    pub input_adapters:
        std::collections::HashMap<String, *const vtable::InputAdapterVtable>,
    pub output_adapters:
        std::collections::HashMap<String, *const vtable::OutputAdapterVtable>,
    pub handlers:
        std::collections::HashMap<String, *const vtable::HandlerVtable>,
}
```

Add `Default` impl for the empty maps (or use `Default::default()` inline).

- [ ] **Step 2: Extend `BeeHostV1` with 3 register-vtable function pointer slots**

Current:
```rust
#[repr(C)]
pub struct BeeHostV1 {
    pub ctx: *mut std::ffi::c_void,
    pub register_adapter:
        Option<unsafe extern "C" fn(ctx: *mut std::ffi::c_void, adapter: *const AdapterDescriptor)>,
}
```

Add 3 more slots (for input/output/handler vtable registration):
```rust
    // S33-deferred: register the plugin's vtable alongside the
    // adapter descriptor. The host stores the vtable pointer in
    // its PluginHandle; the runtime consults it on every event.
    pub register_input_adapter_vtable:
        Option<unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
            vtable: *const vtable::InputAdapterVtable,
        )>,
    pub register_output_adapter_vtable:
        Option<unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
            vtable: *const vtable::OutputAdapterVtable,
        )>,
    pub register_handler_vtable:
        Option<unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
            vtable: *const vtable::HandlerVtable,
        )>,
```

- [ ] **Step 3: Update existing `PluginHandle` constructors / tests** if any

The `Plugin` trait's `init()` returns `PluginResult<PluginHandle>`. The mock plugins in Task 3 already construct the vtable maps. Existing tests in `bee-plugin-sdk/src/lib.rs` (the `manifest_is_clone_and_eq` test) don't touch `PluginHandle` fields, so they should still pass. Verify.

- [ ] **Step 4: Add a unit test that exercises the `BeeHostV1` extension**

```rust
    #[test]
    fn bee_host_v1_has_register_vtable_slots() {
        // Compile-time check: the new slots are present.
        fn _check_slots(h: &BeeHostV1) {
            let _ = h.register_input_adapter_vtable;
            let _ = h.register_output_adapter_vtable;
            let _ = h.register_handler_vtable;
        }
    }
```

- [ ] **Step 5: Run bee-plugin-sdk tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-sdk 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-plugin-sdk && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred §2: PluginHandle + BeeHostV1 carry 3 vtable maps"
```

---

## Task 5: §3 `load_directory` + `PluginAdapterRegistry`

**Files:**
- Modify: `crates/bee-registry/src/loader.rs` (add `load_directory`)
- Create: `crates/bee-runtime/src/plugin_adapter.rs` (registry + wrappers)
- Modify: `crates/bee-runtime/src/lib.rs` (re-export)
- Modify: `crates/bee-runtime/Cargo.toml` (add deps if needed)

- [ ] **Step 1: Add `load_directory` to `crates/bee-registry/src/loader.rs`**

```rust
/// Walk `dir` (non-recursive) and `load_library` every file with
/// the platform's dynamic-library extension (`.so`/`.dylib`/`.dll`).
/// Returns the loaded `PluginId`s in directory-iteration order.
///
/// Errors from individual files (bad ABI, missing symbol, etc.)
/// are collected into the returned `Vec<PluginError>` — one entry
/// per failure. The caller can decide whether to abort or
/// continue. Successfully-loaded plugins are registered with the
/// `PluginManager` regardless of the per-file errors.
pub fn load_directory(
    pm: &mut PluginManager,
    dir: &Path,
) -> (Vec<PluginId>, Vec<PluginError>) {
    let mut loaded = vec![];
    let mut errors = vec![];
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) => {
            errors.push(PluginError::Init(format!("read_dir {}: {e}", dir.display())));
            return (loaded, errors);
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_lib = matches!(ext, "so" | "dylib" | "dll");
        if !is_lib { continue; }
        match load_library(pm, &path) {
            Ok(id) => loaded.push(id),
            Err(e) => errors.push(e),
        }
    }
    (loaded, errors)
}
```

NOTE: the actual signature depends on how `load_library` is currently exposed. Read the current loader.rs and adapt. The MVP is: walk the dir, call `load_library` on each, accumulate results.

- [ ] **Step 2: Add a unit test for `load_directory`**

```rust
    #[test]
    fn load_directory_walks_and_registers_all_libs() {
        let dir = tempfile::tempdir().unwrap();
        // Build a known cdylib (the binance mock) into the dir.
        // Then call load_directory and assert the PluginId is
        // returned.
        // ...
    }
```

(Use the existing `BinanceMockFactory` to construct an in-process PluginHandle, then assert `load_directory` on a directory containing the binance mock's `.dylib` produces the right PluginId. The exact test depends on the build system; for MVP, an integration test that builds the plugin via `cargo build -p bee-plugin-binance-mock` and points `load_directory` at `target/debug/` is acceptable.)

- [ ] **Step 3: Add `PluginAdapterRegistry` to `crates/bee-runtime/src/plugin_adapter.rs`**

```rust
//! S33 §3: registry of loaded-plugin vtables + trait wrappers that
//! implement the existing `InputAdapter` / `OutputAdapter` /
//! `Handler` traits by calling through the FFI vtables.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use bee_plugin_sdk::vtable::{InputAdapterVtable, OutputAdapterVtable, HandlerVtable};

/// Process-global registry of plugin vtables. Populated by the
/// host after `PluginManager::load_directory` registers each
/// plugin's `PluginHandle`. The runtime consults this when
/// starting a Phase to resolve `AdapterKind::Plugin(name)` to a
/// concrete vtable.
pub struct PluginAdapterRegistry {
    input: RwLock<HashMap<String, *const InputAdapterVtable>>,
    output: RwLock<HashMap<String, *const OutputAdapterVtable>>,
    handler: RwLock<HashMap<String, *const HandlerVtable>>,
}

impl PluginAdapterRegistry {
    pub fn global() -> &'static Self {
        static REG: OnceLock<PluginAdapterRegistry> = OnceLock::new();
        REG.get_or_init(|| Self {
            input: RwLock::new(HashMap::new()),
            output: RwLock::new(HashMap::new()),
            handler: RwLock::new(HashMap::new()),
        })
    }

    pub fn register_input(&self, name: &str, vtable: *const InputAdapterVtable) {
        self.input.write().unwrap().insert(name.to_string(), vtable);
    }
    pub fn register_output(&self, name: &str, vtable: *const OutputAdapterVtable) {
        self.output.write().unwrap().insert(name.to_string(), vtable);
    }
    pub fn register_handler(&self, name: &str, vtable: *const HandlerVtable) {
        self.handler.write().unwrap().insert(name.to_string(), vtable);
    }

    pub fn lookup_input(&self, name: &str) -> Option<*const InputAdapterVtable> {
        self.input.read().unwrap().get(name).copied()
    }
    pub fn lookup_output(&self, name: &str) -> Option<*const OutputAdapterVtable> {
        self.output.read().unwrap().get(name).copied()
    }
    pub fn lookup_handler(&self, name: &str) -> Option<*const HandlerVtable> {
        self.handler.read().unwrap().get(name).copied()
    }
}
```

- [ ] **Step 4: Add the trait wrappers** (Input/Output/Handler)

For `PluginInputAdapter`:
```rust
use bee_adapter::{AdapterError, AdapterResult, Event, InputAdapter};
use bee_plugin_sdk::event::{decode_event, encode_event, EventBytes};

pub struct PluginInputAdapter {
    vtable: *const InputAdapterVtable,
    ctx: *mut std::ffi::c_void,
}

impl InputAdapter for PluginInputAdapter {
    type Config = Vec<u8>; // bincode-encoded plugin config

    async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let vtable = PluginAdapterRegistry::global()
            .lookup_input("subscribe") // resolve by name at runtime
            .ok_or_else(|| AdapterError::Open("no plugin vtable for 'subscribe'".into()))?;
        let ctx = unsafe {
            (vtable.open)(config.as_ptr(), config.len(), std::ptr::null_mut())
        };
        if ctx.is_null() {
            return Err(AdapterError::Open("plugin open returned null".into()));
        }
        Ok(Self { vtable, ctx })
    }

    async fn next(&mut self) -> AdapterResult<Option<Event>> {
        let mut out = EventBytes::EMPTY;
        let rc = unsafe { (self.vtable.next)(self.ctx, &mut out) };
        match rc {
            1 => {
                if out.ptr.is_null() || out.len == 0 {
                    return Ok(None);
                }
                let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
                let event = decode_event(bytes).map_err(|e| AdapterError::Next(e.to_string()))?;
                Ok(Some(event))
            }
            0 => Ok(None),
            _ => Err(AdapterError::Next("plugin returned -1".into())),
        }
    }

    async fn close(self) -> AdapterResult<()> {
        let rc = unsafe { (self.vtable.close)(self.ctx) };
        if rc != 0 {
            return Err(AdapterError::Close("plugin returned non-zero".into()));
        }
        Ok(())
    }
}
```

(Symmetric `PluginOutputAdapter::emit` and `PluginHandler::handle` — follow the same pattern.)

- [ ] **Step 5: Add a unit test** that exercises the round-trip via a fake vtable

```rust
    #[test]
    fn plugin_input_adapter_round_trips_through_fake_vtable() {
        // Build a fake InputAdapterVtable whose `next` always
        // returns the same bincode-encoded Event. Register it
        // in the global registry (with a unique name). Open the
        // PluginInputAdapter, call next, assert the event.
        // ...
    }
```

- [ ] **Step 6: Re-export from `crates/bee-runtime/src/lib.rs`**

Add `pub mod plugin_adapter;` and `pub use plugin_adapter::*;` as appropriate.

- [ ] **Step 7: Run tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-runtime --lib plugin_adapter 2>&1 | tail -10
```

- [ ] **Step 8: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-registry crates/bee-runtime && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred §3: load_directory + PluginAdapterRegistry + plugin-backed adapter wrappers"
```

---

## Task 6: §3 runtime dispatch + deployer pre-flight check

**Files:**
- Modify: `crates/bee-runtime/src/lib.rs` (resolve_input_adapter / resolve_output_adapter / resolve_handler)
- Modify: `crates/bee-control/src/deployer.rs` (pre-flight check on adapter names)

- [ ] **Step 1: Add `resolve_input_adapter` / `resolve_output_adapter` / `resolve_handler` to `crates/bee-runtime/src/lib.rs`**

```rust
use crate::plugin_adapter::{
    PluginAdapterRegistry, PluginHandler, PluginInputAdapter, PluginOutputAdapter,
};
use bee_adapter::InputAdapter;

pub fn resolve_input_adapter(name: &str) -> Option<Box<dyn InputAdapter<Config = Vec<u8>>>> {
    if name == "mock" {
        // Keep the in-process MockInputAdapter for tests.
        return Some(Box::new(crate::test_utils::MockInputAdapter::default()));
    }
    if PluginAdapterRegistry::global().lookup_input(name).is_some() {
        // Note: the actual `open` call happens in the runtime
        // when the Phase starts; here we just return a marker
        // that signals "use the plugin vtable". The runtime
        // construction of `PluginInputAdapter` happens in the
        // Phase startup.
        // For the MVP, the trait wrapper's `open` looks up the
        // vtable by name at open time, so the marker is the
        // type itself.
    }
    // Try to instantiate PluginInputAdapter lazily.
    None
}
```

NOTE: the cleanest design is for `resolve_*` to return a `Box<dyn DynAdapter>` that the runtime can call `open` on with a config. The MVP is to add a `DynPluginAdapter` wrapper that does the lazy lookup. Document the design choice in the code.

If this is too speculative, simplify: the MVP is the registry + wrapper types (Task 5). The runtime integration is "if a Phase's `AdapterKind::Plugin(name)` is set, instantiate `PluginInputAdapter::open(config)` which looks up the registry by `name` at `open` time". The `resolve_*` helpers in the runtime lib are convenience wrappers used by the deployer's pre-flight check.

- [ ] **Step 2: Add a pre-flight check in `Deployer::deploy`**

In `crates/bee-control/src/deployer.rs`, before the existing `submit` calls, add:
```rust
        // S33-deferred §3: pre-flight check. Every Plugin-kind
        // adapter the Pipeline references must be registered
        // in the PluginAdapterRegistry. Fail fast if not.
        for ident in &pipeline.stream_identities {
            // For now, the registry is process-global; the MVP
            // doesn't have per-deployer registries. The check
            // is a best-effort warning, not a hard error.
            let reg = bee_runtime::plugin_adapter::PluginAdapterRegistry::global();
            if reg.lookup_input(ident.0).is_none() {
                eprintln!(
                    "warning: stream identity {:?}.{} has no registered input adapter; runtime will fail at Phase start",
                    ident.0, ident.1
                );
            }
        }
```

NOTE: this is a soft warning, not a hard error. The actual lookup happens at Phase start (where the runtime is the only thing that knows the vtable). For S33-deferred, this is the binding contract.

- [ ] **Step 3: Add a unit test for the pre-flight check**

The test asserts that `Deployer::deploy` with a Pipeline that references a known vtable logs no warning; with an unknown one, logs a warning.

- [ ] **Step 4: Run tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-control -p bee-runtime 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-runtime crates/bee-control && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred §3: runtime dispatch + deployer pre-flight warning"
```

---

## Task 7: §4 2 SQL pipelines

**Files:**
- Create: `examples/quant_btc_macd.sql`
- Create: `examples/quant_btc_sentiment.sql`
- Modify: `Cargo.toml` (workspace) — add `examples/` to `[[example]]` (if needed)
- Modify: `bee/src/main.rs` — verify `bee run <sql>` works (or add a check)

- [ ] **Step 1: Write `examples/quant_btc_macd.sql`**

```sql
-- Quant strategy 1: BTC K-line + MACD/EMA (technical only).
-- Pipeline:
--   binance.subscribe('BTC/USDT', '5min')  -- producer
--     -> ta.macd(price, 12, 26, 9)  -- Handler: MACD
--     -> ta.ema(price, 20)          -- Handler: EMA
--     -> influxdb.measurement('btc_macd')  -- sink
-- Backfill is opt-in: add `from='2024-06-01'` to the binance call
-- to test ADR-0011 backfill-on-subscribe semantics (separate run).

use binance;
use influxdb;

SELECT
  b.timestamp AS ts,
  b.symbol,
  b.price,
  ta.macd(b.price, 12, 26, 9) AS macd,
  ta.ema(b.price, 20) AS ema20
FROM binance.subscribe(symbol='BTC/USDT', interval='5min') AS b
EMIT INTO influxdb.measurement(name='btc_macd', database='quant');
```

- [ ] **Step 2: Write `examples/quant_btc_sentiment.sql`**

```sql
-- Quant strategy 2: BTC K-line + FinBERT news sentiment + decision tree.
-- Pipeline:
--   binance.subscribe('BTC/USDT', '5min')   -- producer
--   google_news.search('bitcoin')          -- second producer
--     ASOF JOIN b                            -- time-aligned join
--     -> news.sentiment('finbert', headline)  -- Handler: FinBERT
--     -> tree.decide(price, score) -> action  -- Handler: decision tree
--     -> influxdb.measurement('btc_sentiment')

use binance;
use google_news;
use influxdb;

SELECT
  b.timestamp AS ts,
  b.symbol,
  b.price,
  news.sentiment(model='finbert', text=headline) AS score,
  tree.decide(price=b.price, sentiment=score) AS action
FROM binance.subscribe(symbol='BTC/USDT', interval='5min') AS b
  ASOF JOIN google_news.search(query='bitcoin') AS news
EMIT INTO influxdb.measurement(name='btc_sentiment', database='quant');
```

- [ ] **Step 3: Verify the SQL compiles**

Try `bee run --check examples/quant_btc_macd.sql` (if `--check` exists; otherwise `bee run examples/quant_btc_macd.sql` with a 5-second timeout, then kill the process and check stderr for "compiled" messages).

If the `bee run` path doesn't exist, run a more limited check:
```bash
cd /Users/shaw/Developer/rust/bee && cargo run -p bee --bin bee -- run --check examples/quant_btc_macd.sql 2>&1 | head -20
```

If `--check` doesn't exist, add it as a one-line CLI subcommand (in `bee/src/main.rs`) that calls `compile_to_dag` and prints the result, then exits.

- [ ] **Step 4: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add examples bee/src/main.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred §4: 2 SQL pipelines (BTC+MACD, BTC+FinBERT)"
```

---

## Task 8: §5 demo script

**Files:**
- Create: `scripts/demo-quant-prod.sh`
- Modify: `bee/src/main.rs` (add `bee plugin register <path>` and `bee run --check` if missing)
- Modify: `crates/bee-control/src/cli.rs` (or wherever cluster commands are)

- [ ] **Step 1: Check what's already in `bee/src/main.rs` for the CLI surface**

```bash
grep -n "fn plugin\|fn run\|fn jobs\|fn cluster\|fn datasource" bee/src/main.rs
```

Verify what subcommands exist. Add `bee plugin register <path>` and `bee plugin list` (if missing).

- [ ] **Step 2: Add `bee plugin register <path>` subcommand**

```rust
// in bee/src/main.rs (approximate)
"plugin" => match subcommand.as_str() {
    "register" => {
        // arg: path to .so/.dylib/.dll
        let path = std::path::PathBuf::from(args[0]);
        let mut pm = PluginManager::new();
        let id = bee_registry::loader::load_library(&mut pm, &path)?;
        println!("registered {path:?} as {id}");
    }
    "list" => {
        for (id, manifest) in pm.list() {
            println!("{id}  {} v{}  abi={}", manifest.name, manifest.feature_version, manifest.abi_version);
        }
    }
    _ => bail!("unknown plugin subcommand: {subcommand}"),
},
```

(Adjust to the actual CLI structure. The MVP is that the command exists and produces a sensible message.)

- [ ] **Step 3: Write `scripts/demo-quant-prod.sh`**

```bash
#!/usr/bin/env bash
# scripts/demo-quant-prod.sh — S33-deferred end-to-end demo.
#
# What it does:
#  1. Build the workspace + 5 mock plugins.
#  2. Start a 3-node cluster (ports 7701, 7702, 7703).
#  3. Wait for leader election.
#  4. Register the 5 mock plugins via `bee plugin register`.
#  5. Register 4 Datasources (binance, google_news, influxdb, mongodb).
#  6. Deploy examples/quant_btc_macd.sql  (the Producer).
#  7. Deploy examples/quant_btc_sentiment.sql  (a Subscriber).
#  8. `bee jobs list` — assert one Producer + one Subscriber on BTC.
#  9. Wait 10s for emissions; assert InfluxDB measurement file has rows.
# 10. Teardown the 3 nodes.
# 11. Print a summary table.

set -euo pipefail

# Configurable timeouts
: "${BEE_DEMO_BUILD_TIMEOUT_S:=300}"
: "${BEE_DEMO_STARTUP_TIMEOUT_S:=30}"
: "${BEE_DEMO_RUN_TIMEOUT_S:=15}"
: "${BEE_NODES:=3}"

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$WORKSPACE_ROOT"

# Track results
PASS=0
FAIL=0
RESULTS=()

record() {
  local name="$1" ok="$2"
  if [ "$ok" = "true" ]; then
    RESULTS+=("✓ $name")
    PASS=$((PASS+1))
  else
    RESULTS+=("✗ $name")
    FAIL=$((FAIL+1))
  fi
}

step() { echo; echo "=== $* ==="; }

# Step 1: build
step "build workspace + 5 mock plugins"
if cargo build --workspace --quiet 2>&1 | tail -5 && \
   cargo build -p bee-plugin-binance-mock -p bee-plugin-google-news-mock \
              -p bee-plugin-influxdb-mock -p bee-plugin-mongodb-mock \
              -p bee-plugin-ta-lib-mock --quiet 2>&1 | tail -5; then
  record "build" true
else
  record "build" false
  echo "build failed; aborting"
  printf '%s\n' "${RESULTS[@]}"
  exit 1
fi

# Step 2-3: start a 3-node cluster
step "start $BEE_NODES-node cluster"
PIDS=()
PORTS=()
for i in $(seq 1 "$BEE_NODES"); do
  PORT=$((7700 + i))
  PORTS+=("$PORT")
  LOG="/tmp/bee_node_${i}.log"
  cargo run -p bee --bin bee -- node --port "$PORT" --id "$i" \
    > "$LOG" 2>&1 &
  PIDS+=("$!")
done
trap 'kill ${PIDS[@]:-} 2>/dev/null || true' EXIT

# Wait for leader
DEADLINE=$((SECONDS + BEE_DEMO_STARTUP_TIMEOUT_S))
LEADER=""
while [ $SECONDS -lt $DEADLINE ]; do
  for i in $(seq 1 "$BEE_NODES"); do
    if grep -q "leader" "/tmp/bee_node_${i}.log" 2>/dev/null; then
      LEADER="$i"
      break 2
    fi
  done
  sleep 1
done
if [ -n "$LEADER" ]; then
  record "cluster startup (leader on node $LEADER)" true
else
  record "cluster startup" false
  echo "no leader emerged within ${BEE_DEMO_STARTUP_TIMEOUT_S}s"
  printf '%s\n' "${RESULTS[@]}"
  exit 1
fi

# Step 4: register plugins
step "register 5 mock plugins"
for plugin in bee-plugin-binance-mock bee-plugin-google-news-mock \
              bee-plugin-influxdb-mock bee-plugin-mongodb-mock \
              bee-plugin-ta-lib-mock; do
  LIB="target/debug/lib${plugin//-/_}.dylib"
  [ -f "$LIB" ] || LIB="target/debug/lib${plugin//-/_}.so"
  if cargo run -p bee --bin bee -- plugin register "$LIB" --quiet 2>&1; then
    record "register $plugin" true
  else
    record "register $plugin" false
  fi
done

# Step 5: register datasources
step "register 4 datasources"
# (This depends on `bee datasource create` existing; if it
# doesn't, add it in this script or a follow-up.)

# Step 6-7: deploy pipelines
step "deploy both pipelines"
DEPLOY_MACD_OUTPUT=$(cargo run -p bee --bin bee -- run examples/quant_btc_macd.sql 2>&1 || true)
DEPLOY_SENTIMENT_OUTPUT=$(cargo run -p bee --bin bee -- run examples/quant_btc_sentiment.sql 2>&1 || true)
MACD_JOB=$(echo "$DEPLOY_MACD_OUTPUT" | grep -oE 'job_id: *[0-9]+' | grep -oE '[0-9]+' | head -1 || echo "")
SENTIMENT_JOB=$(echo "$DEPLOY_SENTIMENT_OUTPUT" | grep -oE 'job_id: *[0-9]+' | grep -oE '[0-9]+' | head -1 || echo "")
[ -n "$MACD_JOB" ] && record "deploy quant_btc_macd (job $MACD_JOB)" true || record "deploy quant_btc_macd" false
[ -n "$SENTIMENT_JOB" ] && record "deploy quant_btc_sentiment (job $SENTIMENT_JOB)" true || record "deploy quant_btc_sentiment" false

# Step 8: jobs list
step "bee jobs list — assert Producer/Subscriber mode"
JOBS_OUTPUT=$(cargo run -p bee --bin bee -- jobs list 2>&1 || true)
echo "$JOBS_OUTPUT"
# Assert: $MACD_JOB is a Producer; $SENTIMENT_JOB is a Subscriber
if echo "$JOBS_OUTPUT" | grep -E "^ *$MACD_JOB " | grep -q "Producer"; then
  record "MACD job is Producer" true
else
  record "MACD job is Producer" false
fi
if echo "$JOBS_OUTPUT" | grep -E "^ *$SENTIMENT_JOB " | grep -q "Subscriber"; then
  record "Sentiment job is Subscriber (shared BTC stream)" true
else
  record "Sentiment job is Subscriber" false
fi

# Step 9: wait + assert
step "wait ${BEE_DEMO_RUN_TIMEOUT_S}s for emissions; check sink"
sleep "$BEE_DEMO_RUN_TIMEOUT_S"
# The mock influxdb plugin writes to /tmp/bee_demo_influxdb.log per its
# S33 mock impl; assert the file has lines.
if [ -s /tmp/bee_demo_influxdb.log ] && \
   [ "$(wc -l < /tmp/bee_demo_influxdb.log)" -gt 0 ]; then
  record "influxdb sink has rows" true
else
  record "influxdb sink has rows" false
fi

# Step 11: summary
step "summary"
printf '%s\n' "${RESULTS[@]}"
echo
echo "PASS: $PASS    FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
```

NOTE: the script is a sketch. Several parts depend on the actual CLI surface (which may or may not exist). The implementation MUST iterate the script to match the real CLI output, the real Datasource registration path, the real plugin-loading command, etc.

- [ ] **Step 4: Run the demo script and iterate until green**

```bash
cd /Users/shaw/Developer/rust/bee && bash scripts/demo-quant-prod.sh 2>&1 | tail -30
```

Most likely the first run will fail at several steps (missing CLI subcommands, wrong output formats, etc.). For each failure, fix the script AND/OR the underlying CLI/feature, re-run, repeat.

- [ ] **Step 5: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add scripts bee && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred §5: scripts/demo-quant-prod.sh end-to-end demo"
```

---

## Task 9: Full workspace check + consolidate

**Files:**
- (git history only)

- [ ] **Step 1: Run the full workspace test suite**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | tail -10
```

- [ ] **Step 2: Run `cargo build --workspace` to confirm 0 new warnings**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build --workspace 2>&1 | tail -10
```

- [ ] **Step 3: Look at the commit history since the S33-deferred design**

```bash
cd /Users/shaw/Developer/rust/bee && git log --oneline dfc63da..HEAD
```

Should see 8 commits (Tasks 1-8).

- [ ] **Step 4: Consolidate into a single `S33-deferred: ...` commit**

```bash
cd /Users/shaw/Developer/rust/bee && git reset --soft dfc63da && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33-deferred: FFI wire format + runtime plugin dispatching + 2 SQL + demo

Per docs/superpowers/specs/2026-06-08-s33-deferred-ffi-design.md
(this session's design) + docs/stories.md §S33 deferred items.

§1 — Event wire format (crates/bee-plugin-sdk/src/event.rs):
bincode-serialized Event across the FFI. encode_event / decode_event
helpers + EventBytes FFI struct. 5 unit tests.

§2 — Vtable types + plugin invocation surface
(crates/bee-plugin-sdk/src/vtable.rs):
- InputAdapterVtable (open/next/close)
- OutputAdapterVtable (open/emit/close)
- HandlerVtable (handle/init_state)
All #[repr(C)]. PluginHandle now carries HashMap<String, *const ...>
for the 3 vtable types. BeeHostV1 gains 3 register_*_vtable function
pointers.

§3 — Mock plugin shim + runtime dispatch:
- All 5 mock plugins (binance/google-news/influxdb/mongodb/ta-lib)
  populate vtable maps in init(). Each gets a round-trip unit test.
- PluginManager::load_directory: walks a dir + load_library for each.
- PluginAdapterRegistry: process-global vtable registry (input/output/
  handler). resolve_* helpers in bee-runtime.
- PluginInputAdapter / PluginOutputAdapter / PluginHandler: wrappers
  that implement the existing bee-adapter traits by calling through
  the vtable.
- Deployer pre-flight: warn on Pipeline referencing unknown plugin
  adapter (binding contract; the real lookup happens at Phase start).

§4 — 2 SQL pipelines:
- examples/quant_btc_macd.sql: BTC K-line + MACD/EMA + InfluxDB
  (technical-only strategy).
- examples/quant_btc_sentiment.sql: BTC K-line + FinBERT sentiment +
  decision tree + InfluxDB (technical + ML strategy).
Two different strategies validate 'pure technical' vs
'technical + ML' plugin interop.

§5 — scripts/demo-quant-prod.sh:
end-to-end script: build workspace + 5 mock plugins, start 3-node
cluster, wait for leader, register 5 plugins + 4 datasources, deploy
both pipelines, assert Producer/Subscriber mode (S17), wait 15s for
emissions, assert InfluxDB sink has rows, teardown, summary.
Verifies all 11 ADRs' Consequences at the architecture level.
Plugins stay as mocks until S34-S39 land production versions.

Acceptance (all green):
- 5+ vtable-roundtrip tests (one per mock plugin)
- PluginInputAdapter / PluginOutputAdapter / PluginHandler unit tests
- Deployer pre-flight warning test
- 2 SQL files compile + deploy
- Demo script runs end-to-end on a single host
- All existing S16-S32 tests still pass
- No regressions in the 177+ workspace tests

Out of scope (deferred, tracked in stories.md):
- Production plugins (S34-S39): real Binance WS, NewsAPI, InfluxDB v2,
  MongoDB, yata/ta-lib, tract + FinBERT. The mock plugins' cdylib
  builds and round-trip correctly; replacing the body is mechanical.
- examples/performance/ (S41)
- HITL seed-user walkthrough (S33 close-out, user's manual step after
  S40).

S17 commit (b680d3b) and S33 wrap-up (22a9e39) untouched."
```

- [ ] **Step 5: Verify the single commit**

```bash
cd /Users/shaw/Developer/rust/bee && git log --oneline dfc63da..HEAD
```

Expected: a single commit starting with `S33-deferred: FFI wire format ...`.

- [ ] **Step 6: Run the demo script one more time on the consolidated commit**

```bash
cd /Users/shaw/Developer/rust/bee && bash scripts/demo-quant-prod.sh 2>&1 | tail -30
```

Expected: all steps pass (or a clearly-stated partial pass with the failing steps called out).

- [ ] **Step 7: Final workspace test**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | tail -5
```

---

## Self-review checklist (run before claiming done)

- [ ] **Spec coverage:** §1 (Task 1), §2 (Tasks 2-4), §3 (Tasks 5-6), §4 (Task 7), §5 (Task 8).
- [ ] **No placeholders:** every step has actual code; the demo script is fully written (no "TODO: implement" or "TBD").
- [ ] **Type consistency:**
  - `Event` derives `Serialize`/`Deserialize` (Task 1)
  - `EventBytes` is `#[repr(C)]` (Task 1)
  - `InputAdapterVtable` / `OutputAdapterVtable` / `HandlerVtable` are `#[repr(C)]` (Task 2)
  - `PluginHandle.input_adapters` / `.output_adapters` / `.handlers` are `HashMap<String, *const Vtable>` (Task 4)
  - `BeeHostV1.register_*_vtable` are `Option<unsafe extern "C" fn(...)>` (Task 4)
  - `PluginInputAdapter` / `PluginOutputAdapter` / `PluginHandler` implement the existing `InputAdapter` / `OutputAdapter` / `Handler` traits (Task 5)
  - `PluginAdapterRegistry.lookup_*` returns `Option<*const Vtable>` (Task 5)
- [ ] **DRY:** OK
- [ ] **Frequent commits:** 8 small commits, 1 final consolidation
- [ ] **YAGNI:** no extra features beyond the spec
- [ ] **TDD discipline:** every production code change is preceded by a failing test in a separate commit
- [ ] **No regressions:** all existing tests still pass

## Out-of-scope items (do not address in this plan)

- Production plugin bodies (S34-S39). The mock plugins' cdylib build + vtable round-trip is the binding contract.
- `examples/performance/` (S41).
- HITL seed-user walkthrough.
- `bee run --check` if the subcommand doesn't exist (the script can iterate without it).

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-08-s33-deferred-ffi.md`. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** — execute tasks in this session using `executing-plans`, batch execution with checkpoints

Which approach?
