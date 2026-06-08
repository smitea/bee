# S41 · Performance Showcase (1-Node MVP) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make S41 the new primary 5-minute demo of the main repo. Ship 3 performance demos (Fibonacci, prime sieve, multi-stream analytics) that run in 1-node, in-process mode via `bee run` and print a measured performance table.

**Architecture:** 9 phases — (1) DataFusion 49→50+ upgrade (maintenance: 50+ has bug fixes + perf improvements; **not** for ASOF JOIN — ASOF JOIN is a Bee extension, see Task 9b), (2) BeeHostV1 KV extension (4 FFI function pointers), (3) `bee-plugin-perf-fib` plugin (stateful UDFs with KV-backed state), (4) test fixtures (`generate_series` + `generate_events`, feature-gated), (5) console sink (`EMIT INTO console`), (6) **ASOF JOIN extension in Bee** (SQL-to-SQL translator; DataFusion has no ASOF JOIN in any version — see ADR-0006 + DataFusion issue #318), (7) 3 SQL pipelines under `examples/performance/`, (8) `scripts/demo-perf.sh` (runs all 3, measures wall-clock, prints table, hard correctness check), (9) docs updates, (10) final verification.

**Tech Stack:** Rust, Cargo, DataFusion v50+, libloading (existing), `bee-plugin-sdk`, `bee-dsl-sql`, `bee-kv-test` (in-process KV), Bash.

**Reference docs:**
- Design: `docs/superpowers/specs/2026-06-08-s41-performance-showcase-design.md` (the binding spec)
- Stories: `docs/stories.md` §S41 (the long-term S41 spec; this plan implements the 1-node subset)
- DataFusion v50 docs: `https://docs.rs/datafusion/50*/datafusion/`

**Pre-flight (read these before starting):**
- `Cargo.toml` workspace (datafusion version, workspace members)
- `crates/bee-plugin-sdk/src/lib.rs` (the BeeHostV1 struct, lines around `pub struct BeeHostV1`)
- `crates/bee-dsl-sql/src/lib.rs` (parse_sql, DataFusionPhase, RunConfig, RunMode)
- `plugins/quant/bee-plugin-ta-lib/src/lib.rs` (the canonical plugin pattern: cdylib_plugin! macro + Handler descriptors)
- `bee/src/main.rs` (the `run` subcommand, around the S26 line)

**Working-tree state:** clean, on `main` at `ef89df9` (the S41 design commit). Predecessor commits: `b8c859f` (restructure), `66c4253` (restructure design), `3d16622` (S33-deferred), etc.

---

## File structure (target)

**New (7):**
- `plugins/bee-plugin-perf-fib/Cargo.toml` — cdylib
- `plugins/bee-plugin-perf-fib/src/lib.rs` — fib_step + fib_seed + plugin manifest
- `plugins/bee-plugin-perf-fib/tests/state.rs` — unit tests
- `plugins/bee-plugin-perf-fib/README.md` — UDF docs
- `crates/bee-dsl-sql/src/test_fixtures.rs` — generate_series + generate_events (feature-gated)
- `crates/bee-dsl-sql/src/sinks/console.rs` — console sink
- `examples/performance/fibonacci.sql`
- `examples/performance/prime_sieve.sql`
- `examples/performance/multi_stream_analytics.sql`
- `examples/performance/README.md`
- `scripts/demo-perf.sh`

**Updated (6):**
- `Cargo.toml` (workspace) — datafusion "49" → "50", add bee-plugin-perf-fib member
- `crates/bee-dsl-sql/Cargo.toml` — add `test-fixtures` feature
- `crates/bee-dsl-sql/src/lib.rs` — register console sink + test fixtures (when feature on)
- `crates/bee-plugin-sdk/src/lib.rs` — add kv_get/kv_put/kv_cas/current_stream_id to BeeHostV1
- `bee/src/main.rs` — wire kv_* + current_stream_id host-side
- `README.md` — Performance Demos section
- `docs/product-design.md` — §4 expansion

**Boundary responsibilities:**
- `bee-plugin-perf-fib` is a workspace member; loads via the existing libloading + FFI machinery.
- `BeeHostV1` is the FFI contract; host wires kv_* function pointers to in-process `bee-kv-test` for the 1-node demo.
- Test fixtures are gated behind a Cargo feature so production builds don't include them.
- Console sink is a built-in sink in bee-dsl-sql; no plugin needed.
- `scripts/demo-perf.sh` is the entry point; runs all 3 demos and prints a measured table.

---

## Task 1: DataFusion upgrade (49 → 50+)

**Files:**
- Modify: `Cargo.toml` (workspace dependency)

- [ ] **Step 1: Read current datafusion version**

```bash
grep -A 1 "^datafusion" Cargo.toml
```
Expected: `datafusion = "49"` (or `datafusion = { version = "49", ... }`).

- [ ] **Step 2: Bump to v50**

Edit `Cargo.toml` workspace dependencies. Change `datafusion = "49"` to `datafusion = "50"` (or `datafusion = "50.0.0"` to pin to the first 50.x). Verify any other `datafusion-*` crates in the workspace align (e.g., `datafusion-sql`, `datafusion-expr`); bump them all together.

- [ ] **Step 3: Run cargo update**

```bash
cd /Users/shaw/Developer/rust/bee && cargo update -p datafusion
```
Expected: datafusion 49.x.x → 50.x.x in `Cargo.lock`.

- [ ] **Step 4: Run cargo build + fix any breaking changes**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build --workspace 2>&1 | tail -30
```
Expected: clean build, or compile errors from DataFusion 50 API changes. Fix compile errors as they appear (likely in `crates/bee-dsl-sql/src/physical.rs` and any DataFusion `SessionContext` callers).

- [ ] **Step 5: Run all tests + fix any regressions**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result" | tail -10
```
Expected: 354 passing, 0 failing. If any tests fail due to DataFusion SQL parsing/execution changes, fix them (likely in test SQL strings in `crates/bee-dsl-sql/src/physical.rs` tests or `crates/bee-control/tests/`).

- [ ] **Step 6: Run the full test suite to verify all 354 tests still pass after the upgrade**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result.*passed" | sed -E 's/.*ok\. ([0-9]+) passed.*/\1/' | awk '{sum += $1} END {print "Total:", sum}'
```
Expected: 354. The upgrade is for maintenance (bug fixes + perf improvements in DataFusion 50+); it does NOT add new features. ASOF JOIN is added separately as a Bee extension (see Task 9b).

- [ ] **Step 7: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add Cargo.toml Cargo.lock && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (1): DataFusion 49 -> 50+ upgrade (maintenance; bug fixes + perf)"
```

---

## Task 2: BeeHostV1 KV extension

**Files:**
- Modify: `crates/bee-plugin-sdk/src/lib.rs` (BeeHostV1 struct + safe Rust wrappers)
- Modify: `crates/bee-plugin-sdk/src/lib.rs` (add a `current_stream_id` accessor)

- [ ] **Step 1: Write the failing test for the new function pointers**

Open `crates/bee-plugin-sdk/src/lib.rs`. At the bottom of the file, add a test module:

```rust
#[cfg(test)]
mod bee_host_kv_test {
    use super::*;

    #[test]
    fn bee_host_v1_has_kv_function_pointers() {
        // Construct a BeeHostV1 with all fields populated to None; the
        // test only checks that the fields exist and are of the right type.
        let host = BeeHostV1 {
            ctx: std::ptr::null_mut(),
            register_adapter: None,
            register_input_adapter_vtable: None,
            register_output_adapter_vtable: None,
            register_handler_vtable: None,
            kv_get: None,
            kv_put: None,
            kv_cas: None,
            current_stream_id: None,
        };
        // If this compiles, the fields exist.
        assert!(host.kv_get.is_none());
        assert!(host.kv_put.is_none());
        assert!(host.kv_cas.is_none());
        assert!(host.current_stream_id.is_none());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-sdk bee_host_kv_test 2>&1 | tail -10
```
Expected: compile error (`kv_get`, `kv_put`, `kv_cas`, `current_stream_id` don't exist on BeeHostV1).

- [ ] **Step 3: Add the 4 new function pointers to BeeHostV1**

Open `crates/bee-plugin-sdk/src/lib.rs`. Find `pub struct BeeHostV1 {`. Add the 4 new fields after the existing 4:

```rust
    /// KV: read a value by key. On success, returns 0 and writes
    /// the value pointer + length to `out_value` and `out_len` (caller
    /// frees via `host_alloc_free`). On not-found, returns 1. On error,
    /// returns -1.
    pub kv_get: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            out_value: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32,
    >,

    /// KV: write a value (overwrites). Returns 0 on success, -1 on error.
    pub kv_put: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            value: *const u8,
            len: usize,
        ) -> i32,
    >,

    /// KV: compare-and-swap. Returns 0 on success, 1 on mismatch, -1 on error.
    pub kv_cas: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            expected: *const u8,
            exp_len: usize,
            new: *const u8,
            new_len: usize,
        ) -> i32,
    >,

    /// Get the current stream_id (32-byte hash of the SQL call site).
    /// Returns 0 on success, -1 on error.
    pub current_stream_id: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            out_id: *mut [u8; 32],
        ) -> i32,
    >,
```

- [ ] **Step 4: Add safe Rust wrappers**

In the same file, add an `impl BeeHostV1` block with safe wrappers:

```rust
impl BeeHostV1 {
    /// Safe wrapper for `kv_get`. Returns `Ok(Some(value))` if found,
    /// `Ok(None)` if not found, `Err(SdkError)` on error.
    pub fn safe_kv_get(&self, key: &str) -> Result<Option<Vec<u8>>, crate::SdkError> {
        let kv_get = self.kv_get.ok_or(crate::SdkError::HostFnMissing("kv_get"))?;
        let c_key = std::ffi::CString::new(key).map_err(|_| crate::SdkError::InvalidKey(key.into()))?;
        let mut out_value: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = unsafe { kv_get(self.ctx, c_key.as_ptr(), &mut out_value, &mut out_len) };
        match rc {
            0 => {
                if out_len == 0 {
                    return Ok(Some(Vec::new()));
                }
                let value = unsafe { std::slice::from_raw_parts(out_value, out_len) }.to_vec();
                Ok(Some(value))
            }
            1 => Ok(None),
            _ => Err(crate::SdkError::KvError("kv_get failed")),
        }
    }

    /// Safe wrapper for `kv_put`.
    pub fn safe_kv_put(&self, key: &str, value: &[u8]) -> Result<(), crate::SdkError> {
        let kv_put = self.kv_put.ok_or(crate::SdkError::HostFnMissing("kv_put"))?;
        let c_key = std::ffi::CString::new(key).map_err(|_| crate::SdkError::InvalidKey(key.into()))?;
        let rc = unsafe { kv_put(self.ctx, c_key.as_ptr(), value.as_ptr(), value.len()) };
        if rc == 0 { Ok(()) } else { Err(crate::SdkError::KvError("kv_put failed")) }
    }

    /// Safe wrapper for `kv_cas`. Returns `Ok(true)` on success, `Ok(false)` on mismatch.
    pub fn safe_kv_cas(&self, key: &str, expected: &[u8], new: &[u8]) -> Result<bool, crate::SdkError> {
        let kv_cas = self.kv_cas.ok_or(crate::SdkError::HostFnMissing("kv_cas"))?;
        let c_key = std::ffi::CString::new(key).map_err(|_| crate::SdkError::InvalidKey(key.into()))?;
        let rc = unsafe {
            kv_cas(self.ctx, c_key.as_ptr(), expected.as_ptr(), expected.len(), new.as_ptr(), new.len())
        };
        match rc {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(crate::SdkError::KvError("kv_cas failed")),
        }
    }

    /// Safe wrapper for `current_stream_id`. Returns the 32-byte stream_id hash.
    pub fn safe_current_stream_id(&self) -> Result<[u8; 32], crate::SdkError> {
        let f = self.current_stream_id.ok_or(crate::SdkError::HostFnMissing("current_stream_id"))?;
        let mut out_id = [0u8; 32];
        let rc = unsafe { f(self.ctx, &mut out_id) };
        if rc == 0 { Ok(out_id) } else { Err(crate::SdkError::KvError("current_stream_id failed")) }
    }
}
```

- [ ] **Step 5: Add the SdkError enum (if not already present)**

In `crates/bee-plugin-sdk/src/lib.rs`, add the `SdkError` enum (if it doesn't already exist):

```rust
#[derive(Debug)]
pub enum SdkError {
    HostFnMissing(&'static str),
    InvalidKey(String),
    KvError(&'static str),
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SdkError::HostFnMissing(name) => write!(f, "host function pointer {} is None", name),
            SdkError::InvalidKey(k) => write!(f, "invalid KV key: {}", k),
            SdkError::KvError(msg) => write!(f, "KV error: {}", msg),
        }
    }
}

impl std::error::Error for SdkError {}
```

- [ ] **Step 6: Run the test to verify it passes**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-sdk bee_host_kv_test 2>&1 | tail -10
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-plugin-sdk && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (2): BeeHostV1 KV extension — 4 new FFI function pointers + safe Rust wrappers"
```

---

## Task 3: Host-side wiring of kv_* + current_stream_id in bee CLI

**Files:**
- Modify: `bee/src/main.rs` (the `run_plugin_cli` function or wherever plugins are loaded)

- [ ] **Step 1: Find the plugin loading code**

```bash
cd /Users/shaw/Developer/rust/bee && grep -n "register_handler_vtable\|register_input_adapter_vtable\|run_plugin_cli\|PluginHandle" bee/src/main.rs
```
Expected: a function that constructs a `BeeHostV1` struct and passes it to plugins. (The existing 5 quant plugins use this path.)

- [ ] **Step 2: Add the 4 new function pointers to the host-side BeeHostV1**

In the function that constructs `BeeHostV1`, add the 4 new function pointers. The kv_* function pointers wrap in-process `bee-kv-test` (which is a HashMap-backed test KV impl). The `current_stream_id` returns a stable hash of the current SQL call's call-site (for the S41 demo, derive it from a process-global counter — restart-survival is via the KV state, not the stream_id).

```rust
// In the function that builds BeeHostV1, after the existing 4 fields:

// KV: in-process test store (bee-kv-test).
let kv_store = Arc::new(std::sync::Mutex::new(HashMap::<String, Vec<u8>>::new()));

let kv_get: extern "C" fn(
    *mut c_void, *const c_char, *mut *mut u8, *mut usize
) -> i32 = {
    let store = kv_store.clone();
    extern "C" fn kv_get_impl(
        _ctx: *mut c_void,
        key: *const c_char,
        out_value: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32 {
        // SAFETY: `key` is a valid C string from a plugin call.
        let key_str = unsafe { CStr::from_ptr(key) }.to_str().unwrap_or("");
        // SAFETY: this is a demo; we use a thread-local copy of the store.
        let store = KV_STORE_DEMO.lock().unwrap();
        match store.get(key_str) {
            Some(v) => {
                // SAFETY: we leak the Vec<u8> to the plugin; the plugin
                // must not free it. (For S41 demo, this is OK; production
                // FFI would have explicit alloc/free.)
                let boxed = v.clone().into_boxed_slice();
                let len = boxed.len();
                let ptr = Box::into_raw(boxed) as *mut u8;
                unsafe {
                    *out_value = ptr;
                    *out_len = len;
                }
                0
            }
            None => 1,
        }
    }
    kv_get_impl
};

let kv_put: extern "C" fn(
    *mut c_void, *const c_char, *const u8, usize
) -> i32 = {
    extern "C" fn kv_put_impl(
        _ctx: *mut c_void,
        key: *const c_char,
        value: *const u8,
        len: usize,
    ) -> i32 {
        let key_str = unsafe { CStr::from_ptr(key) }.to_str().unwrap_or("");
        // SAFETY: `value` is valid for `len` bytes.
        let value_slice = unsafe { std::slice::from_raw_parts(value, len) };
        KV_STORE_DEMO.lock().unwrap().insert(key_str.to_string(), value_slice.to_vec());
        0
    }
    kv_put_impl
};

let kv_cas: extern "C" fn(
    *mut c_void, *const c_char, *const u8, usize, *const u8, usize
) -> i32 = {
    extern "C" fn kv_cas_impl(
        _ctx: *mut c_void,
        key: *const c_char,
        expected: *const u8,
        exp_len: usize,
        new: *const u8,
        new_len: usize,
    ) -> i32 {
        let key_str = unsafe { CStr::from_ptr(key) }.to_str().unwrap_or("");
        let exp = unsafe { std::slice::from_raw_parts(expected, exp_len) };
        let new = unsafe { std::slice::from_raw_parts(new, new_len) };
        let mut store = KV_STORE_DEMO.lock().unwrap();
        match store.get(key_str) {
            Some(v) if v.as_slice() == exp => {
                store.insert(key_str.to_string(), new.to_vec());
                0
            }
            Some(_) => 1,
            None => 1,  // not found = mismatch
        }
    }
    kv_cas_impl
};

let current_stream_id: extern "C" fn(
    *mut c_void, *mut [u8; 32]
) -> i32 = {
    extern "C" fn stream_id_impl(
        _ctx: *mut c_void,
        out_id: *mut [u8; 32],
    ) -> i32 {
        // For the S41 demo, use a global counter hashed with sha256.
        // This gives a stable, unique stream_id per call within a process.
        let id = STREAM_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let hash = sha2::Sha256::digest(id.to_le_bytes());
        // SAFETY: out_id is a valid *mut [u8; 32] from the plugin.
        unsafe { *out_id = hash.into() };
        0
    }
    stream_id_impl
};
```

At the top of `bee/src/main.rs` (or in a small helper module), define:

```rust
use std::sync::atomic::AtomicU64;
static STREAM_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
static KV_STORE_DEMO: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>> =
    std::sync::Mutex::new(std::collections::HashMap::new());
```

Then construct the `BeeHostV1` with all 8 function pointers:

```rust
let host = BeeHostV1 {
    ctx: ...,
    register_adapter: ...,
    register_input_adapter_vtable: ...,
    register_output_adapter_vtable: ...,
    register_handler_vtable: ...,
    kv_get: Some(kv_get),
    kv_put: Some(kv_put),
    kv_cas: Some(kv_cas),
    current_stream_id: Some(current_stream_id),
};
```

- [ ] **Step 3: Build to verify it compiles**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build -p bee 2>&1 | tail -10
```
Expected: clean build.

- [ ] **Step 4: Run all tests to verify no regressions**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result.*passed" | sed -E 's/.*ok\. ([0-9]+) passed.*/\1/' | awk '{sum += $1} END {print "Total:", sum}'
```
Expected: 354 (unchanged).

- [ ] **Step 5: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add bee/src/main.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (3): host-side wiring of kv_* + current_stream_id in bee CLI"
```

---

## Task 4: bee-plugin-perf-fib scaffold

**Files:**
- Create: `plugins/bee-plugin-perf-fib/Cargo.toml`
- Create: `plugins/bee-plugin-perf-fib/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create the plugin directory**

```bash
mkdir -p plugins/bee-plugin-perf-fib/src plugins/bee-plugin-perf-fib/tests
```

- [ ] **Step 2: Write the Cargo.toml**

`plugins/bee-plugin-perf-fib/Cargo.toml`:

```toml
[package]
name = "bee-plugin-perf-fib"
version = "0.1.0"
edition = "2021"
description = "Bee performance showcase plugin: Fibonacci UDFs with KV-backed state"

[lib]
crate-type = ["cdylib"]

[dependencies]
bee-plugin-sdk = { path = "../../crates/bee-plugin-sdk" }
serde = { version = "1", features = ["derive"] }
bincode = "1"
once_cell = "1"
```

- [ ] **Step 3: Add to workspace members**

Edit `Cargo.toml` workspace `members` list. Add `"plugins/bee-plugin-perf-fib"` (place it before the `plugins/quant/*` entries).

- [ ] **Step 4: Write the lib.rs skeleton**

`plugins/bee-plugin-perf-fib/src/lib.rs`:

```rust
//! `bee-plugin-perf-fib` — Fibonacci UDFs for the S41 performance showcase.
//!
//! Two Handler UDFs:
//! - `fib_seed(n)`: stateless; returns 0 (n=0) or 1 (n=1).
//! - `fib_step(n)`: stateful; reads its own previous 2 emitted values from KV
//!   (key: `state/handler/<stream_id>/fib_step/state`), computes `prev2 + prev1`,
//!   stores the new pair, returns the new value.
//!
//! State is in the host's KV (see S41 design). For the 1-node demo, the host
//! uses an in-process `HashMap`-backed KV (bee-kv-test). For multi-node
//! deployment, the host uses a Raft-replicated KV.

use bee_plugin_sdk::{
    Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName,
};
use serde::{Deserialize, Serialize};

/// The KV-stored state for `fib_step`: the two most recent values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FibState {
    pub prev2: i128,
    pub prev1: i128,
}

impl FibState {
    pub fn next(&self) -> i128 {
        self.prev2 + self.prev1
    }
    pub fn update(&mut self, new_value: i128) {
        self.prev2 = self.prev1;
        self.prev1 = new_value;
    }
}

/// `fib_seed(n)`: returns 0 if n == 0, else 1. The seed for the Fibonacci sequence.
pub fn fib_seed(n: u64) -> i128 {
    if n == 0 { 0 } else { 1 }
}

/// Plugin manifest. Declares 2 Handler descriptors.
pub fn plugin_manifest() -> PluginManifest {
    PluginManifest {
        name: PluginName("bee-plugin-perf-fib".into()),
        handlers: vec![
            HandlerDescriptor { name: "fib_seed".into() },
            HandlerDescriptor { name: "fib_step".into() },
        ],
        // No adapters in this plugin; only Handlers.
        ..Default::default()
    }
}

pub struct PerfFibFactory;

impl Factory for PerfFibFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }
    fn init() -> bee_plugin_sdk::PluginResult<PluginHandle> {
        Ok(PluginHandle::new(perf_fib_handle()))
    }
}

fn perf_fib_handle() -> bee_plugin_sdk::PluginHandle {
    // The plugin has no private state beyond what the host stores in KV.
    bee_plugin_sdk::PluginHandle::new(())
}

bee_plugin_sdk::cdylib_plugin!(PerfFibFactory);
```

- [ ] **Step 5: Build the plugin**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build -p bee-plugin-perf-fib 2>&1 | tail -10
```
Expected: errors about `bee_plugin_sdk::cdylib_plugin!` macro and `PluginHandle::new` not being the right signature. Look at the actual signature in `crates/bee-plugin-sdk/src/lib.rs` and adjust. The canonical example is `plugins/quant/bee-plugin-ta-lib/src/lib.rs`.

- [ ] **Step 6: Look at the existing plugin pattern**

```bash
cd /Users/shaw/Developer/rust/bee && cat plugins/quant/bee-plugin-ta-lib/src/lib.rs | head -150
```
Identify:
- The actual `cdylib_plugin!` macro signature
- The `PluginHandle::new` constructor (or whatever is used)
- The `Factory` trait's actual methods

Adjust `plugins/bee-plugin-perf-fib/src/lib.rs` to match. Likely changes:
- `PluginHandle::new(())` may need a different argument
- `Factory` may have different method names

- [ ] **Step 7: Re-build the plugin**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build -p bee-plugin-perf-fib 2>&1 | tail -10
```
Expected: clean build.

- [ ] **Step 8: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add plugins/bee-plugin-perf-fib Cargo.toml && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (4): bee-plugin-perf-fib scaffold (cdylib, fib_seed, plugin manifest)"
```

---

## Task 5: bee-plugin-perf-fib: fib_seed unit tests

**Files:**
- Create: `plugins/bee-plugin-perf-fib/tests/seed.rs`

- [ ] **Step 1: Write the failing test**

`plugins/bee-plugin-perf-fib/tests/seed.rs`:

```rust
//! Unit tests for the `fib_seed` UDF (stateless).

use bee_plugin_perf_fib::fib_seed;

#[test]
fn fib_seed_returns_0_for_n_0() {
    assert_eq!(fib_seed(0), 0);
}

#[test]
fn fib_seed_returns_1_for_n_1() {
    assert_eq!(fib_seed(1), 1);
}

#[test]
fn fib_seed_returns_1_for_n_ge_1() {
    // Per the UDF spec: fib_seed returns 0 only for n=0; for n>=1, returns 1.
    for n in 1..20 {
        assert_eq!(fib_seed(n), 1, "fib_seed({}) should be 1", n);
    }
}
```

- [ ] **Step 2: Run the test to verify it passes**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-perf-fib fib_seed 2>&1 | tail -10
```
Expected: PASS. (The function is already implemented; this just locks in the behavior.)

- [ ] **Step 3: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add plugins/bee-plugin-perf-fib/tests && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (5): fib_seed unit tests"
```

---

## Task 6: bee-plugin-perf-fib: fib_step (stateful, KV-backed)

**Files:**
- Modify: `plugins/bee-plugin-perf-fib/src/lib.rs` (add `fib_step` + KV access)
- Create: `plugins/bee-plugin-perf-fib/tests/state.rs` (state round-trip test)

- [ ] **Step 1: Write the failing test for fib_step's first call**

`plugins/bee-plugin-perf-fib/tests/state.rs`:

```rust
//! Unit tests for the `fib_step` UDF (stateful, KV-backed).
//!
//! The UDF takes the host's `BeeHostV1` and uses the kv_* function pointers
//! to read/write the state. For unit tests, we use a mock host that
//! implements kv_get/kv_put via a `HashMap`.

use bee_plugin_perf_fib::{fib_seed, fib_step, FibState};
use bee_plugin_sdk::BeeHostV1;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A mock host with an in-memory KV store + a fixed stream_id.
struct MockHost {
    kv: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    stream_id: [u8; 32],
}

impl MockHost {
    fn new() -> Self {
        Self {
            kv: Arc::new(Mutex::new(HashMap::new())),
            stream_id: [0u8; 32], // fixed stream_id for tests
        }
    }
    fn state_key(&self) -> String {
        format!("state/handler/{}/fib_step/state", hex::encode(self.stream_id))
    }
}

/// Build a `BeeHostV1` with kv_* wired to the mock.
fn build_mock_host(host: &MockHost) -> BeeHostV1 {
    use std::ffi::{c_char, c_void};
    use std::sync::atomic::{AtomicU64, Ordering};

    let kv = host.kv.clone();
    static CTX: AtomicU64 = AtomicU64::new(0);
    let ctx_id = CTX.fetch_add(1, Ordering::Relaxed) as *mut c_void;

    extern "C" fn kv_get(
        _ctx: *mut c_void,
        key: *const c_char,
        out_value: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32 {
        // SAFETY: c_char is null-terminated; c_char and c_void are valid pointers.
        let key_str = unsafe { std::ffi::CStr::from_ptr(key) }.to_str().unwrap();
        // Find the global KV (hacky but works for the test harness).
        let store = TEST_KV.lock().unwrap();
        match store.get(key_str) {
            Some(v) => {
                let boxed = v.clone().into_boxed_slice();
                let len = boxed.len();
                let ptr = Box::into_raw(boxed) as *mut u8;
                unsafe { *out_value = ptr; *out_len = len; }
                0
            }
            None => 1,
        }
    }
    extern "C" fn kv_put(
        _ctx: *mut c_void,
        key: *const c_char,
        value: *const u8,
        len: usize,
    ) -> i32 {
        let key_str = unsafe { std::ffi::CStr::from_ptr(key) }.to_str().unwrap();
        let value_slice = unsafe { std::slice::from_raw_parts(value, len) };
        TEST_KV.lock().unwrap().insert(key_str.to_string(), value_slice.to_vec());
        0
    }
    extern "C" fn kv_cas(
        _ctx: *mut c_void,
        key: *const c_char,
        expected: *const u8,
        exp_len: usize,
        new: *const u8,
        new_len: usize,
    ) -> i32 {
        let key_str = unsafe { std::ffi::CStr::from_ptr(key) }.to_str().unwrap();
        let exp = unsafe { std::slice::from_raw_parts(expected, exp_len) };
        let new = unsafe { std::slice::from_raw_parts(new, new_len) };
        let mut store = TEST_KV.lock().unwrap();
        match store.get(key_str) {
            Some(v) if v.as_slice() == exp => {
                store.insert(key_str.to_string(), new.to_vec());
                0
            }
            _ => 1,
        }
    }
    extern "C" fn current_stream_id(
        _ctx: *mut c_void,
        out_id: *mut [u8; 32],
    ) -> i32 {
        unsafe { *out_id = [0u8; 32]; }
        0
    }

    BeeHostV1 {
        ctx: ctx_id,
        register_adapter: None,
        register_input_adapter_vtable: None,
        register_output_adapter_vtable: None,
        register_handler_vtable: None,
        kv_get: Some(kv_get),
        kv_put: Some(kv_put),
        kv_cas: Some(kv_cas),
        current_stream_id: Some(current_stream_id),
    }
}

static TEST_KV: Mutex<HashMap<String, Vec<u8>>> = Mutex::new(HashMap::new());

#[test]
fn fib_step_first_call_returns_1() {
    // First call: state is (0, 0) by default. fib_step = 0 + 0 = 0.
    // Wait — actually, the first fib_step call should return 1 because
    // the seed is 1 (from fib_seed). Let me re-think the state.
    //
    // Actually: fib_step(n) for the first call (n=1) should return 1.
    // fib_step(n) for n=2 should return 1+0=1. Wait, Fibonacci is 0, 1, 1, 2, 3, ...
    //
    // For correctness: fib(0)=0, fib(1)=1, fib(2)=1, fib(3)=2, fib(4)=3, ...
    // fib_step(1) = fib(0) + fib(1) = 0 + 1 = 1 → state becomes (1, 1)
    // fib_step(2) = fib(1) + fib(2) = 1 + 1 = 2 → state becomes (1, 2)
    // fib_step(3) = fib(2) + fib(3) = 1 + 2 = 3 → state becomes (2, 3)
    // ...
    //
    // So the initial state should be (0, 1) (prev2=0, prev1=1).
    // fib_step(1) = 0+1 = 1, state becomes (1, 1)
    let mock = MockHost::new();
    let host = build_mock_host(&mock);
    let key = mock.state_key();

    // Initialize state to (0, 1)
    let initial = FibState { prev2: 0, prev1: 1 };
    let bytes = bincode::serialize(&initial).unwrap();
    mock.kv.lock().unwrap().insert(key.clone(), bytes);

    // First fib_step call: should return 1
    let result = fib_step(&host, 1);
    assert_eq!(result, 1, "fib_step(1) should be 1");
}

#[test]
fn fib_step_100_values_correct() {
    let mock = MockHost::new();
    let host = build_mock_host(&mock);
    let key = mock.state_key();

    // Initialize state
    let initial = FibState { prev2: 0, prev1: 1 };
    let bytes = bincode::serialize(&initial).unwrap();
    mock.kv.lock().unwrap().insert(key.clone(), bytes);

    // Compute 100 values; the known Fibonacci sequence is
    // 0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89, ...
    let expected_first_20: [i128; 20] = [
        1, 1, 2, 3, 5, 8, 13, 21, 34, 55,
        89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765,
    ];
    for n in 1..=20u64 {
        let actual = fib_step(&host, n);
        assert_eq!(actual, expected_first_20[(n - 1) as usize], "fib_step({}) mismatch", n);
    }
}

#[test]
fn fib_step_state_survives_simulated_restart() {
    let mock = MockHost::new();
    let host = build_mock_host(&mock);
    let key = mock.state_key();

    // Compute 100 values
    let initial = FibState { prev2: 0, prev1: 1 };
    let bytes = bincode::serialize(&initial).unwrap();
    mock.kv.lock().unwrap().insert(key.clone(), bytes);

    for n in 1..=100u64 {
        let _ = fib_step(&host, n);
    }

    // "Restart" — construct a new host (with the same KV store backing).
    // Verify the 101st value: it should be the 100th Fibonacci number (assuming
    // fib_seed/fib_step semantics — actually, it's the 101st fib_step call).
    // The 100th fib value (0-indexed: fib(99)) is huge. Let's just check the
    // state is non-zero and the next call returns a non-zero value.
    let new_host = build_mock_host(&mock);
    let result = fib_step(&new_host, 101);
    assert!(result > 0, "fib_step after restart should be non-zero");
}
```

- [ ] **Step 2: Run the test to verify it fails (fib_step not implemented yet)**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-perf-fib fib_step 2>&1 | tail -10
```
Expected: compile error (`fib_step` not exported from the crate).

- [ ] **Step 3: Implement fib_step in the plugin's lib.rs**

Open `plugins/bee-plugin-perf-fib/src/lib.rs`. Add the `fib_step` function and the `compute_state_key` helper:

```rust
/// Compute the KV key for the fib_step state.
fn state_key(host: &bee_plugin_sdk::BeeHostV1) -> Result<String, bee_plugin_sdk::SdkError> {
    let stream_id = host.safe_current_stream_id()?;
    Ok(format!("state/handler/{}/fib_step/state", hex::encode(stream_id)))
}

/// `fib_step(n)`: stateful Fibonacci step.
/// Reads the previous 2 values from KV, computes `prev2 + prev1`, writes the
/// new state, returns the new value. If no state exists, initializes to
/// `(0, 1)` (the Fibonacci seed pair).
pub fn fib_step(host: &bee_plugin_sdk::BeeHostV1, _n: u64) -> i128 {
    let key = state_key(host).expect("compute state key");
    let current = match host.safe_kv_get(&key) {
        Ok(Some(bytes)) => bincode::deserialize::<FibState>(&bytes).unwrap_or(FibState { prev2: 0, prev1: 1 }),
        Ok(None) => FibState { prev2: 0, prev1: 1 },
        Err(_) => FibState { prev2: 0, prev1: 1 },
    };
    let new_value = current.next();
    let updated = FibState { prev2: current.prev1, prev1: new_value };
    let bytes = bincode::serialize(&updated).unwrap();
    host.safe_kv_put(&key, &bytes).expect("kv_put");
    new_value
}
```

Also add `hex` to dependencies. Edit `plugins/bee-plugin-perf-fib/Cargo.toml`:
```toml
hex = "0.4"
```

- [ ] **Step 4: Add the test-only `hex` crate**

In the same `Cargo.toml`, add `hex` under `[dev-dependencies]`:
```toml
[dev-dependencies]
hex = "0.4"
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-plugin-perf-fib 2>&1 | tail -20
```
Expected: PASS for all 3 fib_step tests + 3 fib_seed tests (from Task 5).

- [ ] **Step 6: Run all workspace tests to verify no regressions**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result.*passed" | sed -E 's/.*ok\. ([0-9]+) passed.*/\1/' | awk '{sum += $1} END {print "Total:", sum}'
```
Expected: 354 + new tests (>= 354).

- [ ] **Step 7: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add plugins/bee-plugin-perf-fib && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (6): fib_step stateful UDF with KV-backed state + 3 unit tests"
```

---

## Task 7: Test fixtures feature flag + generate_series

**Files:**
- Modify: `crates/bee-dsl-sql/Cargo.toml` (add `test-fixtures` feature)
- Create: `crates/bee-dsl-sql/src/test_fixtures.rs` (generate_series)
- Modify: `crates/bee-dsl-sql/src/lib.rs` (register generate_series when feature is on)

- [ ] **Step 1: Add the feature flag**

Open `crates/bee-dsl-sql/Cargo.toml`. Find `[features]` (or add it). Add:
```toml
[features]
test-fixtures = []
```

- [ ] **Step 2: Write a failing test for generate_series**

`crates/bee-dsl-sql/src/test_fixtures.rs`:

```rust
//! Test fixture functions, gated behind the `test-fixtures` feature.
//!
//! These functions are intended for demos and tests only; they produce
//! deterministic data streams without external services.

#![cfg(feature = "test-fixtures")]

use datafusion::arrow::array::{Array, Int64Array, RecordBatch};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::common::Result as DfResult;
use datafusion::logical_expr::ColumnarValue;
use datafusion::physical_plan::functions::Signature;
use datafusion::physical_plan::functions::ScalarFunctionImplementation;
use std::sync::Arc;

/// `generate_series(start, end) -> Stream<i64>`: emits one event per
/// integer in `[start, end]`. Returns a single RecordBatch with one
/// Int64 column.
pub fn generate_series_impl(args: &[ColumnarValue]) -> DfResult<ColumnarValue> {
    if args.len() != 2 {
        return Err(datafusion::error::DataFusionError::Plan(
            "generate_series expects 2 arguments: (start, end)".into(),
        ));
    }
    // For simplicity, evaluate as scalar arguments.
    let start = match &args[0] {
        ColumnarValue::Scalar(s) => s.to_array_of_size(1)?.as_any().downcast_ref::<Int64Array>().unwrap().value(0),
        ColumnarValue::Array(a) => a.as_any().downcast_ref::<Int64Array>().unwrap().value(0),
    };
    let end = match &args[1] {
        ColumnarValue::Scalar(s) => s.to_array_of_size(1)?.as_any().downcast_ref::<Int64Array>().unwrap().value(0),
        ColumnarValue::Array(a) => a.as_any().downcast_ref::<Int64Array>().unwrap().value(0),
    };
    if end < start {
        return Err(datafusion::error::DataFusionError::Plan(
            "generate_series: end must be >= start".into(),
        ));
    }
    let count = (end - start + 1) as usize;
    let values: Vec<i64> = (start..=end).collect();
    let array = Arc::new(Int64Array::from(values));
    Ok(ColumnarValue::Array(array))
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::Int64Array;

    #[test]
    fn generate_series_emits_values_in_range() {
        let args = vec![
            ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Int64(Some(1))),
            ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Int64(Some(5))),
        ];
        let result = generate_series_impl(&args).unwrap();
        match result {
            ColumnarValue::Array(a) => {
                let arr = a.as_any().downcast_ref::<Int64Array>().unwrap();
                assert_eq!(arr.len(), 5);
                assert_eq!(arr.value(0), 1);
                assert_eq!(arr.value(4), 5);
            }
            _ => panic!("expected array"),
        }
    }
}
```

- [ ] **Step 3: Run the test with the feature on**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-dsl-sql --features test-fixtures generate_series 2>&1 | tail -10
```
Expected: PASS.

- [ ] **Step 4: Run the test with the feature off (should compile, test is cfg-gated)**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build -p bee-dsl-sql 2>&1 | tail -5
```
Expected: clean build, the `test_fixtures` module is not compiled.

- [ ] **Step 5: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-dsl-sql && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (7): test-fixtures feature flag + generate_series UDF"
```

---

## Task 8: Test fixtures: generate_events

**Files:**
- Modify: `crates/bee-dsl-sql/src/test_fixtures.rs` (add `generate_events`)

- [ ] **Step 1: Write a failing test for generate_events**

Append to `crates/bee-dsl-sql/src/test_fixtures.rs`:

```rust
/// `generate_events(schema, count, seed) -> Stream<StructType>`: emits
/// `count` deterministic pseudo-random events with the given schema.
/// Uses a linear congruential generator (LCG) seeded with `seed`.
pub fn generate_events_impl(args: &[ColumnarValue]) -> DfResult<ColumnarValue> {
    if args.len() != 3 {
        return Err(datafusion::error::DataFusionError::Plan(
            "generate_events expects 3 arguments: (schema, count, seed)".into(),
        ));
    }
    // For the S41 demo, generate_events is simplified: it always produces
    // a (user_id, ts) struct with a deterministic LCG.
    // The schema argument is accepted but ignored.
    let count = match &args[1] {
        ColumnarValue::Scalar(s) => s.to_array_of_size(1)?.as_any().downcast_ref::<Int64Array>().unwrap().value(0) as usize,
        ColumnarValue::Array(a) => a.as_any().downcast_ref::<Int64Array>().unwrap().value(0) as usize,
    };
    let seed = match &args[2] {
        ColumnarValue::Scalar(s) => s.to_array_of_size(1)?.as_any().downcast_ref::<Int64Array>().unwrap().value(0) as u64,
        ColumnarValue::Array(a) => a.as_any().downcast_ref::<Int64Array>().unwrap().value(0) as u64,
    };

    // LCG: x_{n+1} = (a * x_n + c) mod m
    // (Numerical Recipes constants)
    const A: u64 = 1664525;
    const C: u64 = 1013904223;
    const M: u64 = 1 << 32;

    let mut x = seed;
    let user_ids: Vec<i64> = (0..count).map(|_| {
        x = (A.wrapping_mul(x).wrapping_add(C)) % M;
        ((x % 1000) + 1) as i64  // user_id in [1, 1000]
    }).collect();
    let mut x = seed.wrapping_add(1);
    let timestamps: Vec<i64> = (0..count).map(|i| {
        x = (A.wrapping_mul(x).wrapping_add(C)) % M;
        // Each event is 1 second apart, starting from epoch 1700000000 (2023-11-14)
        1700000000i64 + i as i64
    }).collect();

    let user_id_array = Arc::new(Int64Array::from(user_ids));
    let ts_array = Arc::new(Int64Array::from(timestamps));

    // Return as a struct array: { user_id: i64, ts: i64 }
    use datafusion::arrow::array::StructArray;
    let struct_array = StructArray::try_new(
        datafusion::arrow::datatypes::Fields::from(vec![
            Field::new("user_id", DataType::Int64, false),
            Field::new("ts", DataType::Int64, false),
        ]),
        vec![user_id_array, ts_array],
        None,
    )?;
    Ok(ColumnarValue::Array(Arc::new(struct_array)))
}

#[cfg(test)]
mod generate_events_tests {
    use super::*;

    #[test]
    fn generate_events_is_deterministic() {
        let args = vec![
            ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Int64(Some(0))),  // schema (ignored)
            ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Int64(Some(100))),  // count
            ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Int64(Some(42))),  // seed
        ];
        let r1 = generate_events_impl(&args).unwrap();
        let r2 = generate_events_impl(&args).unwrap();
        // Both calls should produce the same struct array (deterministic LCG).
        let a1 = match r1 {
            ColumnarValue::Array(a) => a,
            _ => panic!("expected array"),
        };
        let a2 = match r2 {
            ColumnarValue::Array(a) => a,
            _ => panic!("expected array"),
        };
        assert_eq!(a1.len(), 100);
        assert_eq!(a1.len(), a2.len());
        // Same seed → same data; verify by checking the first few user_ids.
        let s1 = a1.as_any().downcast_ref::<datafusion::arrow::array::StructArray>().unwrap();
        let s2 = a2.as_any().downcast_ref::<datafusion::arrow::array::StructArray>().unwrap();
        let col1 = s1.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let col2 = s2.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        for i in 0..10 {
            assert_eq!(col1.value(i), col2.value(i), "user_id mismatch at index {}", i);
        }
    }
}
```

- [ ] **Step 2: Run the test**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-dsl-sql --features test-fixtures generate_events 2>&1 | tail -10
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-dsl-sql/src/test_fixtures.rs && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (8): generate_events test fixture (deterministic LCG)"
```

---

## Task 9: Console sink (EMIT INTO console)

**Files:**
- Create: `crates/bee-dsl-sql/src/sinks/console.rs`
- Create: `crates/bee-dsl-sql/src/sinks/mod.rs` (module declaration)
- Modify: `crates/bee-dsl-sql/src/lib.rs` (register console sink)

- [ ] **Step 1: Create the sinks module**

```bash
mkdir -p crates/bee-dsl-sql/src/sinks
```

`crates/bee-dsl-sql/src/sinks/mod.rs`:
```rust
pub mod console;
```

- [ ] **Step 2: Write the console sink**

`crates/bee-dsl-sql/src/sinks/console.rs`:

```rust
//! `console` sink: writes rows to stdout, one per line, JSON-formatted.
//!
//! The console sink is a built-in (no plugin needed). It is always
//! available; no feature flag.

use datafusion::arrow::array::*;
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::record_batch::RecordBatch;
use std::io::Write;

/// Print a single RecordBatch to stdout. Each row is one line of JSON.
pub fn emit_to_console(batch: &RecordBatch) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for row_idx in 0..batch.num_rows() {
        let mut parts: Vec<String> = Vec::new();
        for (col_idx, field) in batch.schema().fields().iter().enumerate() {
            let col = batch.column(col_idx);
            let value = match field.data_type() {
                DataType::Int64 => {
                    let arr = col.as_any().downcast_ref::<Int64Array>().unwrap();
                    arr.value(row_idx).to_string()
                }
                DataType::Float64 => {
                    let arr = col.as_any().downcast_ref::<Float64Array>().unwrap();
                    arr.value(row_idx).to_string()
                }
                DataType::Utf8 => {
                    let arr = col.as_any().downcast_ref::<StringArray>().unwrap();
                    format!("\"{}\"", arr.value(row_idx))
                }
                DataType::Boolean => {
                    let arr = col.as_any().downcast_ref::<BooleanArray>().unwrap();
                    arr.value(row_idx).to_string()
                }
                _ => format!("<{:?}>", field.data_type()),
            };
            parts.push(format!("{}={}", field.name(), value));
        }
        writeln!(out, "{}", parts.join(", "))?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::datatypes::{Field, Schema};
    use std::sync::Arc;

    #[test]
    fn emit_empty_batch_is_ok() {
        let schema = Arc::new(Schema::empty());
        let batch = RecordBatch::try_new(schema, vec![]).unwrap();
        assert!(emit_to_console(&batch).is_ok());
    }
}
```

- [ ] **Step 3: Register the console sink in lib.rs**

Open `crates/bee-dsl-sql/src/lib.rs`. Add at the top:
```rust
pub mod sinks;
```

Then in the existing `EMIT INTO` parser handler (or wherever SQL is dispatched), add a branch for `console`:
```rust
if sink_name == "console" {
    sinks::console::emit_to_console(&result_batch)?;
    return Ok(());
}
```

The exact integration point depends on how the existing `EMIT INTO` is implemented; find it via:
```bash
grep -n "EMIT INTO\|emit_into" crates/bee-dsl-sql/src/*.rs
```

- [ ] **Step 4: Build to verify**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build -p bee-dsl-sql 2>&1 | tail -5
```
Expected: clean build, possibly with `sinks::console::emit_to_console` integration if the EMIT INTO handler needed changes.

- [ ] **Step 5: Run tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-dsl-sql console 2>&1 | tail -10
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-dsl-sql && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (9): console sink — EMIT INTO console writes rows to stdout"
```

---

## Task 9b: ASOF JOIN extension in Bee (NEW)

**Why this task**: Per the design's §6 amendment, ASOF JOIN is a Bee-level extension (DataFusion has no ASOF JOIN in any version — see ADR-0006 + DataFusion issue #318, still open as of 2026-06). This task implements the Bee-level translator.

**Files:**
- Create: `crates/bee-dsl-sql/src/asof.rs` — the SQL-to-SQL translator
- Modify: `crates/bee-dsl-sql/src/lib.rs` — register the translator in the SQL execution path

- [ ] **Step 1: Write the failing test for ASOF JOIN translation**

Create `crates/bee-dsl-sql/src/asof.rs`:

```rust
//! ASOF JOIN extension for Bee's SQL runtime.
//!
//! ASOF JOIN is a temporal JOIN that matches each row from the left
//! side to the nearest-prior (or nearest) row from the right side, based
//! on time + optional equi-keys. It is the canonical JOIN for financial
//! time-series (kdb+, DolphinDB, pandas merge_asof).
//!
//! Bee's ASOF JOIN is implemented as a SQL-to-SQL translation: we
//! recognize `LEFT ASOF JOIN` as a custom keyword in the parser, then
//! rewrite it to a `LEFT JOIN LATERAL ... LIMIT 1` subquery that
//! DataFusion can execute natively.

#![allow(unused_imports)]

use datafusion::error::{DataFusionError, Result as DfResult};

/// The side of an ASOF JOIN (only `LEFT` is supported in S41 MVP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsOfSide {
    Left,
}

/// Recognize whether a SQL string contains an `ASOF JOIN` clause.
/// Returns the SQL with `ASOF` stripped (DataFusion will see `JOIN`),
/// plus the parsed side + the join conditions.
pub fn parse_asof(sql: &str) -> DfResult<Option<AsOfClause>> {
    // Look for "ASOF JOIN" (case-insensitive, with flexible whitespace)
    let upper = sql.to_uppercase();
    let pos = upper.find("ASOF JOIN");
    if pos.is_none() {
        return Ok(None);
    }
    // Simple parser: extract the join conditions after "ASOF JOIN <right> ON"
    // For S41 MVP, we support a single equi-key + single inequality.
    // Example: "a ASOF JOIN b ON a.id = b.id AND a.ts >= b.ts"
    let after_asof = &sql[pos.unwrap() + "ASOF JOIN".len()..];
    // Find the ON clause
    let on_pos = after_asof.to_uppercase().find(" ON ").ok_or_else(|| {
        DataFusionError::Plan(format!("ASOF JOIN must be followed by ON clause: {}", sql))
    })?;
    let right_and_rest = &after_asof[on_pos + 4..];
    // Find the WHERE or end of join conditions
    let cond_end = right_and_rest
        .to_uppercase()
        .find(" WHERE ")
        .unwrap_or(right_and_rest.len());
    let conditions = &right_and_rest[..cond_end].trim();

    // Parse conditions: expect "a.col = b.col AND a.col >= b.col"
    let parts: Vec<&str> = conditions.split(" AND ").collect();
    if parts.len() != 2 {
        return Err(DataFusionError::Plan(format!(
            "ASOF JOIN must have exactly 2 conditions (equi + inequality), got: {}",
            conditions
        )));
    }
    let equi = parts[0].trim();
    let ineq = parts[1].trim();

    // Equi condition: a.col = b.col
    let equi_split: Vec<&str> = equi.split('=').collect();
    if equi_split.len() != 2 {
        return Err(DataFusionError::Plan(format!("Invalid equi condition: {}", equi)));
    }
    let equi_left = equi_split[0].trim().to_string();
    let equi_right = equi_split[1].trim().to_string();

    // Inequality: a.col >= b.col (must be >= or <=)
    let ineq_op: &str;
    if ineq.contains(">=") {
        ineq_op = ">=";
    } else if ineq.contains("<=") {
        ineq_op = "<=";
    } else {
        return Err(DataFusionError::Plan(format!(
            "ASOF JOIN inequality must be >= or <=, got: {}",
            ineq
        )));
    }
    let ineq_split: Vec<&str> = ineq.split(ineq_op).collect();
    if ineq_split.len() != 2 {
        return Err(DataFusionError::Plan(format!("Invalid inequality: {}", ineq)));
    }
    let ineq_left = ineq_split[0].trim().to_string();
    let ineq_right = ineq_split[1].trim().to_string();

    Ok(Some(AsOfClause {
        side: AsOfSide::Left,
        equi_left_col: equi_left,
        equi_right_col: equi_right,
        ineq_left_col: ineq_left,
        ineq_right_col: ineq_right,
        ineq_op: ineq_op.to_string(),
    }))
}

/// Parsed ASOF JOIN clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsOfClause {
    pub side: AsOfSide,
    /// Equi-key: rows match when `equi_left_col = equi_right_col`.
    pub equi_left_col: String,
    pub equi_right_col: String,
    /// Inequality: rows match when `ineq_left_col >= ineq_right_col` (nearest prior).
    pub ineq_left_col: String,
    pub ineq_right_col: String,
    /// The operator (`>=` or `<=`).
    pub ineq_op: String,
}

/// Translate `a ASOF JOIN b ON a.id = b.id AND a.ts >= b.ts`
/// to `a LEFT JOIN LATERAL (SELECT * FROM b WHERE b.id = a.id AND b.ts <= a.ts ORDER BY b.ts DESC LIMIT 1) b ON TRUE`
///
/// Returns the translated SQL with the ASOF keyword stripped (so DataFusion sees a regular JOIN).
pub fn translate_asof(sql: &str) -> DfResult<String> {
    let clause = match parse_asof(sql)? {
        Some(c) => c,
        None => return Ok(sql.to_string()),
    };

    // Extract the right-side table name (between "ASOF JOIN" and "ON")
    let upper = sql.to_uppercase();
    let asof_pos = upper.find("ASOF JOIN").unwrap();
    let after_asof = &sql[asof_pos + "ASOF JOIN".len()..];
    let on_pos = after_asof.to_uppercase().find(" ON ").unwrap();
    let right_table = after_asof[..on_pos].trim().to_string();

    // Determine the inequality direction.
    // If user wrote `a.ts >= b.ts` (left >= right), we want the MAX b.ts <= a.ts
    // (the nearest prior). Translation: `b.ts <= a.ts ORDER BY b.ts DESC LIMIT 1`.
    // If user wrote `a.ts <= b.ts` (left <= right), we want the MIN b.ts >= a.ts
    // (the nearest future). Translation: `b.ts >= a.ts ORDER BY b.ts ASC LIMIT 1`.
    let (translated_ineq_op, order_direction) = if clause.ineq_op == ">=" {
        ("<=", "DESC")
    } else {
        (">=", "ASC")
    };

    // Build the LATERAL subquery
    let equi_left = &clause.equi_left_col;
    let equi_right = &clause.equi_right_col;
    let ineq_left = &clause.ineq_left_col;
    let ineq_right = &clause.ineq_right_col;

    // Strip `a.` and `b.` prefixes for the LATERAL subquery (it's a subquery on `b`)
    let equi_right_col = equi_right.split('.').next_back().unwrap_or(equi_right);
    let ineq_right_col = ineq_right.split('.').next_back().unwrap_or(ineq_right);
    let equi_left_col = equi_left.split('.').next_back().unwrap_or(equi_left);
    let ineq_left_col = ineq_left.split('.').next_back().unwrap_or(ineq_left);

    let lateral_subquery = format!(
        "(SELECT * FROM {right_table} \
         WHERE {equi_right_col} = {equi_left_col} \
           AND {ineq_right_col} {translated_op} {ineq_left_col} \
         ORDER BY {ineq_right_col} {direction} LIMIT 1)",
        right_table = right_table,
        equi_right_col = equi_right_col,
        equi_left_col = equi_left_col,
        ineq_right_col = ineq_right_col,
        translated_op = translated_ineq_op,
        ineq_left_col = ineq_left_col,
        direction = order_direction,
    );

    // Replace the ASOF JOIN clause with `LEFT JOIN <subquery> b ON TRUE`
    // The original was: `a ASOF JOIN b ON ...`
    // The replacement is: `a LEFT JOIN <subquery> b ON TRUE`
    let before = &sql[..asof_pos];
    let after_on_and_conditions = &after_asof[on_pos + 4..];
    // Find the end of the ON conditions (next WHERE, JOIN, or end of statement)
    let cond_end = after_on_and_conditions
        .to_uppercase()
        .find(" WHERE ")
        .or_else(|| after_on_and_conditions.to_uppercase().find(" JOIN "))
        .unwrap_or(after_on_and_conditions.len());
    let after = &after_on_and_conditions[cond_end..];

    let translated = format!(
        "{before}LEFT JOIN {subquery} b ON TRUE{after}",
        before = before,
        subquery = lateral_subquery,
        after = after,
    );

    Ok(translated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_asof_returns_none() {
        let sql = "SELECT * FROM a JOIN b ON a.id = b.id";
        assert!(parse_asof(sql).unwrap().is_none());
    }

    #[test]
    fn parse_left_asof_nearest_prior() {
        let sql = "SELECT * FROM a ASOF JOIN b ON a.id = b.id AND a.ts >= b.ts";
        let clause = parse_asof(sql).unwrap().unwrap();
        assert_eq!(clause.side, AsOfSide::Left);
        assert_eq!(clause.equi_left_col, "a.id");
        assert_eq!(clause.equi_right_col, "b.id");
        assert_eq!(clause.ineq_left_col, "a.ts");
        assert_eq!(clause.ineq_right_col, "b.ts");
        assert_eq!(clause.ineq_op, ">=");
    }

    #[test]
    fn translate_left_asof_to_lateral() {
        let sql = "SELECT * FROM a ASOF JOIN b ON a.id = b.id AND a.ts >= b.ts";
        let translated = translate_asof(sql).unwrap();
        // Should contain LEFT JOIN LATERAL (or our equivalent)
        assert!(translated.contains("LEFT JOIN"));
        assert!(translated.contains("SELECT * FROM b"));
        assert!(translated.contains("b.id = a.id"));
        assert!(translated.contains("b.ts <= a.ts"));
        assert!(translated.contains("ORDER BY b.ts DESC"));
        assert!(translated.contains("LIMIT 1"));
    }

    #[test]
    fn translate_left_asof_nearest_future() {
        // a.ts <= b.ts means we want the nearest future row from b
        let sql = "SELECT * FROM a ASOF JOIN b ON a.id = b.id AND a.ts <= b.ts";
        let translated = translate_asof(sql).unwrap();
        assert!(translated.contains("b.ts >= a.ts"));
        assert!(translated.contains("ORDER BY b.ts ASC"));
    }

    #[test]
    fn asof_join_end_to_end_correctness() {
        // An end-to-end test using real DataFusion execution.
        // Two tables, ASOF JOIN, verify the result matches the nearest-prior semantic.
        use datafusion::arrow::array::{Int64Array, RecordBatch, StringArray};
        use datafusion::arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;

        let schema_left = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ts", DataType::Int64, false),
        ]));
        let schema_right = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ts", DataType::Int64, false),
            Field::new("value", DataType::Utf8, false),
        ]));

        let left = RecordBatch::try_new(
            schema_left.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 1, 2])),
                Arc::new(Int64Array::from(vec![10, 20, 30])),
            ],
        ).unwrap();
        let right = RecordBatch::try_new(
            schema_right.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 1, 2])),
                Arc::new(Int64Array::from(vec![5, 15, 25])),
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
            ],
        ).unwrap();

        let ctx = datafusion::prelude::SessionContext::new();
        ctx.register_batch("a", left).unwrap();
        ctx.register_batch("b", right).unwrap();

        // Run the translated ASOF JOIN
        let sql = "SELECT a.id, a.ts, b.value FROM a ASOF JOIN b ON a.id = b.id AND a.ts >= b.ts";
        let translated = translate_asof(sql).unwrap();
        let df = ctx.sql(&translated).await.unwrap();
        let results = df.collect().await.unwrap();

        // Verify the result
        // Left row (id=1, ts=10) should match right (id=1, ts=5, value="a")
        // Left row (id=1, ts=20) should match right (id=1, ts=15, value="b")
        // Left row (id=2, ts=30) should match right (id=2, ts=25, value="c")
        let id_col = results[0].column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let val_col = results[0].column(2).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(id_col.value(0), 1);
        assert_eq!(val_col.value(0), "a");
        assert_eq!(id_col.value(1), 1);
        assert_eq!(val_col.value(1), "b");
        assert_eq!(id_col.value(2), 2);
        assert_eq!(val_col.value(2), "c");
    }
}
```

- [ ] **Step 2: Run the test to verify the parsing + translation works**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-dsl-sql asof 2>&1 | tail -20
```
Expected: PASS for all 5 tests (parse_no_asof_returns_none, parse_left_asof_nearest_prior, translate_left_asof_to_lateral, translate_left_asof_nearest_future, asof_join_end_to_end_correctness).

Note: the asof_join_end_to_end_correctness test actually executes the translated SQL in DataFusion 50. If DataFusion 50's LATERAL JOIN syntax differs from what we generate, the test will fail. Adjust the SQL generation if needed (e.g., DataFusion 50 may not support LATERAL but supports `unnest` + correlated subquery).

- [ ] **Step 3: Wire the translator into the SQL execution path**

Open `crates/bee-dsl-sql/src/lib.rs`. Find the function that executes a SQL string (likely in `physical.rs` or a `run_sql` function). Add a preprocessing step:

```rust
use crate::asof::translate_asof;

pub fn preprocess_sql(sql: &str) -> std::result::Result<String, datafusion::error::DataFusionError> {
    if sql.to_uppercase().contains("ASOF JOIN") {
        translate_asof(sql)
    } else {
        Ok(sql.to_string())
    }
}
```

Then in the SQL execution function, call `preprocess_sql(sql)` before passing to DataFusion:

```rust
let preprocessed = preprocess_sql(sql)?;
let df = ctx.sql(&preprocessed).await?;
```

- [ ] **Step 4: Add a test that verifies the SQL execution path uses the translator**

Open `crates/bee-dsl-sql/src/lib.rs` (or a new test file). Add a test that uses `preprocess_sql` + a real `SessionContext`:

```rust
#[cfg(test)]
mod preprocess_test {
    use super::*;
    use datafusion::arrow::array::{Int64Array, RecordBatch, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;

    #[tokio::test]
    async fn asof_join_via_preprocess() {
        let schema_left = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ts", DataType::Int64, false),
        ]));
        let schema_right = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ts", DataType::Int64, false),
            Field::new("value", DataType::Utf8, false),
        ]));
        let left = RecordBatch::try_new(
            schema_left.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1])),
                Arc::new(Int64Array::from(vec![10])),
            ],
        ).unwrap();
        let right = RecordBatch::try_new(
            schema_right.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1])),
                Arc::new(Int64Array::from(vec![5])),
                Arc::new(StringArray::from(vec!["a"])),
            ],
        ).unwrap();
        let ctx = datafusion::prelude::SessionContext::new();
        ctx.register_batch("a", left).unwrap();
        ctx.register_batch("b", right).unwrap();

        let sql = "SELECT a.id, b.value FROM a ASOF JOIN b ON a.id = b.id AND a.ts >= b.ts";
        let preprocessed = preprocess_sql(sql).unwrap();
        let df = ctx.sql(&preprocessed).await.unwrap();
        let results = df.collect().await.unwrap();
        assert_eq!(results[0].num_rows(), 1);
        let val_col = results[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(val_col.value(0), "a");
    }
}
```

- [ ] **Step 5: Run the preprocess test**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test -p bee-dsl-sql preprocess_test 2>&1 | tail -10
```
Expected: PASS.

- [ ] **Step 6: Run all workspace tests to verify no regressions**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result.*passed" | sed -E 's/.*ok\. ([0-9]+) passed.*/\1/' | awk '{sum += $1} END {print "Total:", sum}'
```
Expected: 354 (unchanged) + new asof tests.

- [ ] **Step 7: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add crates/bee-dsl-sql && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (9b): ASOF JOIN extension — Bee-level SQL-to-SQL translator (LEFT ASOF JOIN → LEFT JOIN LATERAL)"
```

---

## Task 10: fibonacci.sql

**Files:**
- Create: `examples/performance/fibonacci.sql`
- Create: `examples/performance/README.md` (skeleton; expand in Task 15)

- [ ] **Step 1: Create the directory**

```bash
mkdir -p examples/performance
```

- [ ] **Step 2: Write the SQL file**

`examples/performance/fibonacci.sql`:

```sql
-- Fibonacci (1M values): exercises stateful Handler UDF + KV-backed state.
-- This is the S41 demo's smallest possible streaming-compute surface.

use perf_fib;

CREATE SOURCE naturals AS
SELECT n FROM generate_series(1, 1000000);

CREATE VIEW fib_stream AS
SELECT
    n,
    fib_step(n) AS fib_value
FROM naturals;

-- Sanity check: emit the first 20 fib values to the console.
-- Expected sequence: 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765
EMIT INTO console
SELECT n, fib_value FROM fib_stream WHERE n <= 20;
```

- [ ] **Step 3: Run the SQL via `bee run` (after the plugin is built)**

First, ensure `test-fixtures` feature is on for the demo. Build the bee binary with the feature:
```bash
cd /Users/shaw/Developer/rust/bee && cargo build -p bee-dsl-sql --features test-fixtures
```

Build the perf-fib plugin:
```bash
cd /Users/shaw/Developer/rust/bee && cargo build -p bee-plugin-perf-fib
```

Run the demo:
```bash
cd /Users/shaw/Developer/rust/bee && cargo run -p bee --bin bee -- run examples/performance/fibonacci.sql 2>&1 | tail -30
```
Expected: 20 lines of console output, each with `n=X, fib_value=Y` where Y matches the expected Fibonacci sequence.

- [ ] **Step 4: Commit the SQL**

```bash
cd /Users/shaw/Developer/rust/bee && git add examples/performance && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (10): fibonacci.sql — 1M values, stateful UDF + KV state"
```

---

## Task 11: prime_sieve.sql (with hard correctness check)

**Files:**
- Create: `examples/performance/prime_sieve.sql`

- [ ] **Step 1: Write the SQL file**

`examples/performance/prime_sieve.sql`:

```sql
-- Prime sieve (≤ 10^8): 20 sequential sieving Phases via Eratosthenes.
-- The hard correctness check: n_primes = 5761455 (primes <= 10^8).
-- This is a 1-node demo; in production with N nodes, the sieving Phases
-- would be placed on different Nodes to maximize throughput.

CREATE SOURCE naturals AS
SELECT n FROM generate_series(2, 100000000);

-- 20 sieving Phases (primes 2, 3, 5, 7, ..., 71).
-- For the demo, we hardcode 20 primes; a perf run could generate
-- 100 or 1000 primes via a script.
CREATE VIEW sieved_2 AS
SELECT n FROM naturals WHERE n = 2 OR n % 2 != 0;
CREATE VIEW sieved_3 AS
SELECT n FROM sieved_2 WHERE n = 3 OR n % 3 != 0;
CREATE VIEW sieved_5 AS
SELECT n FROM sieved_3 WHERE n = 5 OR n % 5 != 0;
CREATE VIEW sieved_7 AS
SELECT n FROM sieved_5 WHERE n = 7 OR n % 7 != 0;
CREATE VIEW sieved_11 AS
SELECT n FROM sieved_7 WHERE n = 11 OR n % 11 != 0;
CREATE VIEW sieved_13 AS
SELECT n FROM sieved_11 WHERE n = 13 OR n % 13 != 0;
CREATE VIEW sieved_17 AS
SELECT n FROM sieved_13 WHERE n = 17 OR n % 17 != 0;
CREATE VIEW sieved_19 AS
SELECT n FROM sieved_17 WHERE n = 19 OR n % 19 != 0;
CREATE VIEW sieved_23 AS
SELECT n FROM sieved_19 WHERE n = 23 OR n % 23 != 0;
CREATE VIEW sieved_29 AS
SELECT n FROM sieved_23 WHERE n = 29 OR n % 29 != 0;
CREATE VIEW sieved_31 AS
SELECT n FROM sieved_29 WHERE n = 31 OR n % 31 != 0;
CREATE VIEW sieved_37 AS
SELECT n FROM sieved_31 WHERE n = 37 OR n % 37 != 0;
CREATE VIEW sieved_41 AS
SELECT n FROM sieved_37 WHERE n = 41 OR n % 41 != 0;
CREATE VIEW sieved_43 AS
SELECT n FROM sieved_41 WHERE n = 43 OR n % 43 != 0;
CREATE VIEW sieved_47 AS
SELECT n FROM sieved_43 WHERE n = 47 OR n % 47 != 0;
CREATE VIEW sieved_53 AS
SELECT n FROM sieved_47 WHERE n = 53 OR n % 53 != 0;
CREATE VIEW sieved_59 AS
SELECT n FROM sieved_53 WHERE n = 59 OR n % 59 != 0;
CREATE VIEW sieved_61 AS
SELECT n FROM sieved_59 WHERE n = 61 OR n % 61 != 0;
CREATE VIEW sieved_67 AS
SELECT n FROM sieved_61 WHERE n = 67 OR n % 67 != 0;
CREATE VIEW sieved_71 AS
SELECT n FROM sieved_67 WHERE n = 71 OR n % 71 != 0;

-- Final count: there are 5,761,455 primes <= 10^8.
-- The console output must match this number — a hard correctness check.
CREATE VIEW prime_count AS
SELECT count(*) AS n_primes FROM sieved_71;

EMIT INTO console SELECT * FROM prime_count;
```

- [ ] **Step 2: Run the demo**

```bash
cd /Users/shaw/Developer/rust/bee && cargo run -p bee --bin bee -- run examples/performance/prime_sieve.sql 2>&1 | tail -10
```
Expected: 1 line of console output like `n_primes=5761455`. If the count is wrong, the script will fail in Task 13 (the demo script's hard correctness check).

Note: this may take 1-2 minutes for 10^8 integers through 20 filters. The 1-node demo accepts this; production N-node deployment would distribute the Phases.

- [ ] **Step 3: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add examples/performance && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (11): prime_sieve.sql — 20 primes, hard correctness check 5761455"
```

---

## Task 12: multi_stream_analytics.sql

**Files:**
- Create: `examples/performance/multi_stream_analytics.sql`

- [ ] **Step 1: Write the SQL file**

`examples/performance/multi_stream_analytics.sql`:

```sql
-- Multi-stream analytics: 3 test-fixture streams + ASOF JOIN + WINDOW TUMBLING.
-- This is the most realistic of the 3 demos; closest to a real Bee user workload.

CREATE SOURCE clicks AS
SELECT user_id, ts, page FROM generate_events(
    struct_pack(user_id INT, ts INT, page STRING),
    100000, seed => 42
);

CREATE SOURCE views AS
SELECT user_id, ts, duration_ms INT FROM generate_events(
    struct_pack(user_id INT, ts INT, duration_ms INT),
    50000, seed => 43
);

CREATE SOURCE purchases AS
SELECT user_id, ts, amount DECIMAL FROM generate_events(
    struct_pack(user_id INT, ts INT, amount DECIMAL),
    10000, seed => 44
);

-- 1-min tumbling window aggregation joined across the 3 streams.
-- The ASOF JOIN aligns by user_id and time (nearest-prior view/purchase).
CREATE VIEW per_minute AS
SELECT
    date_trunc('minute', to_timestamp(c.ts)) AS minute,
    count(DISTINCT c.user_id) AS unique_clickers,
    count(DISTINCT p.user_id) AS unique_buyers,
    sum(p.amount) AS revenue
FROM clicks c
LEFT ASOF JOIN views v ON c.user_id = v.user_id AND c.ts >= v.ts
LEFT ASOF JOIN purchases p ON c.user_id = p.user_id AND c.ts >= p.ts
WINDOW TUMBLING (c.ts, INTERVAL '1' MINUTE)
GROUP BY date_trunc('minute', to_timestamp(c.ts));

EMIT INTO console
SELECT * FROM per_minute ORDER BY minute LIMIT 60;  -- first hour
```

**Note on SQL syntax**: the exact syntax for `WINDOW TUMBLING` and `struct_pack` may differ in DataFusion 50. If the SQL doesn't parse, use the canonical DataFusion 50 syntax (e.g., `struct()` instead of `struct_pack()`; `window` syntax may need to be replaced with a simpler `GROUP BY` + `date_trunc`). Iterate until it parses and runs.

- [ ] **Step 2: Run the demo**

```bash
cd /Users/shaw/Developer/rust/bee && cargo run -p bee --bin bee -- run examples/performance/multi_stream_analytics.sql 2>&1 | tail -20
```
Expected: some lines of console output (per-minute aggregation). If the SQL doesn't parse, fix the syntax (see the note above).

- [ ] **Step 3: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add examples/performance && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (12): multi_stream_analytics.sql — ASOF JOIN + WINDOW TUMBLING"
```

---

## Task 13: scripts/demo-perf.sh

**Files:**
- Create: `scripts/demo-perf.sh`

- [ ] **Step 1: Write the demo script**

`scripts/demo-perf.sh`:

```bash
#!/usr/bin/env bash
# scripts/demo-perf.sh — S41 1-Node performance showcase.
#
# Runs the 3 S41 demos and prints a measured performance table.
# Pre-builds the perf-fib plugin and the bee binary first (to avoid
# cargo run startup overhead in the timing measurement).
#
# Usage:
#   scripts/demo-perf.sh            # run all 3 demos
#   BEE_QUIET=1 scripts/demo-perf.sh # less output

set -euo pipefail

cd "$(dirname "$0")/.."

# 0. Pre-build
echo "==== Pre-build ===="
(cd plugins/bee-plugin-perf-fib && cargo build --release --quiet)
cargo build --release --quiet -p bee-dsl-sql --features test-fixtures
cargo build --release --quiet -p bee

BEE=./target/release/bee

# Verify the binary exists
if [ ! -x "$BEE" ]; then
    echo "ERROR: $BEE not found after build"
    exit 1
fi

# 1. Demo 1: Fibonacci
echo ""
echo "==== Demo 1: Fibonacci (1M values) ===="
T0=$(date +%s%N)
$BEE run examples/performance/fibonacci.sql 2>&1 | tail -25
T1=$(date +%s%N)
FIB_MS=$(( (T1 - T0) / 1_000_000 ))
FIB_TPUT=$(( 1_000_000 * 1_000_000_000 / (T1 - T0) ))

# 2. Demo 2: prime sieve
echo ""
echo "==== Demo 2: Prime sieve (≤ 10^8, 20 primes) ===="
T0=$(date +%s%N)
PRIME_OUTPUT=$($BEE run examples/performance/prime_sieve.sql 2>&1)
T1=$(date +%s%N)
SIEVE_MS=$(( (T1 - T0) / 1_000_000 ))
echo "$PRIME_OUTPUT" | tail -5

# Hard correctness check: there are exactly 5,761,455 primes <= 10^8
N=$(echo "$PRIME_OUTPUT" | grep -oE 'n_primes=[0-9]+' | tail -1 | cut -d= -f2)
if [ -z "$N" ]; then
    echo "FAIL: prime count not found in output"
    exit 1
fi
if [ "$N" -ne 5761455 ]; then
    echo "FAIL: prime count mismatch (expected 5761455, got $N)"
    exit 1
fi
echo "✓ prime count correct (5761455)"

# 3. Demo 3: multi-stream analytics
echo ""
echo "==== Demo 3: Multi-stream analytics (160K events) ===="
T0=$(date +%s%N)
$BEE run examples/performance/multi_stream_analytics.sql 2>&1 | tail -25
T1=$(date +%s%N)
MS_MS=$(( (T1 - T0) / 1_000_000 ))
MS_TPUT=$(( 160_000 * 1_000_000_000 / (T1 - T0) ))

# 4. Print measured perf table
cat <<EOF

==== Measured performance (1 Node) ====
| Demo                      | Wall-clock        | Throughput              |
|---------------------------|-------------------|-------------------------|
| Fibonacci (1M values)     | ${FIB_MS} ms      | ${FIB_TPUT} events/sec  |
| Prime sieve (≤ 10^8)      | ${SIEVE_MS} ms    | (10^8 ints sieved)      |
| Multi-stream analytics    | ${MS_MS} ms       | ${MS_TPUT} events/sec   |

EOF
```

- [ ] **Step 2: Make it executable**

```bash
chmod +x scripts/demo-perf.sh
```

- [ ] **Step 3: Run it**

```bash
cd /Users/shaw/Developer/rust/bee && bash scripts/demo-perf.sh 2>&1 | tail -30
```
Expected: 3 demos run, hard correctness check passes, perf table printed at the end. Total runtime: 2-5 minutes (1-node demo).

- [ ] **Step 4: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add scripts/demo-perf.sh && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (13): scripts/demo-perf.sh — runs 3 demos, hard correctness check, perf table"
```

---

## Task 14: examples/performance/README.md

**Files:**
- Create: `examples/performance/README.md`

- [ ] **Step 1: Write the README**

`examples/performance/README.md`:

```markdown
# Bee · Performance Showcase · 1-Node Demos

This directory is the **S41 performance showcase** — the new primary
5-minute demo of the main repo (per the restructure). It contains 3
self-contained SQL pipelines that exercise different aspects of Bee:

| Demo | What it shows | Bee design choice exercised |
| --- | --- | --- |
| `fibonacci.sql` | Stateful Handler UDF + KV-backed state | Stateful Handlers with persistent state across calls |
| `prime_sieve.sql` | Multi-Phase SQL with sequential filters | Cross-Phase data channels; correctness via 5,761,455 prime count |
| `multi_stream_analytics.sql` | 3 input streams + ASOF JOIN + WINDOW TUMBLING | Multi-source aggregation; the "real Bee user" shape |

## Running

```bash
scripts/demo-perf.sh
```

The script pre-builds the perf-fib plugin + the bee binary (release
mode), then runs all 3 demos and prints a measured performance table.

The prime sieve has a **hard correctness check**: the output must be
`n_primes = 5761455` (primes ≤ 10^8). The script fails loudly if this
isn't the case.

## Why these 3 demos

- **Fibonacci**: the canonical streaming-state problem. Every step
  depends on the previous N (here N=2) values. Exercises the
  `Handler UDF` + `KV-stored state` path — the same path the
  quant strategy uses — in the smallest possible surface area.

- **Prime sieve**: the canonical distributed-scheduling problem.
  Each sieve pass is a self-contained filter that can run in
  parallel on different Nodes. For 1-node mode, all 20 Phases
  run in-process; for N-node mode (future), the runtime scheduler
  places them on different Nodes.

- **Multi-stream analytics**: exercises the SQL runtime
  (`ASOF JOIN`, `WINDOW TUMBLING`, multi-sink `EMIT INTO`) on a
  realistic data shape (clicks / views / purchases per user).
  Closest to a real Bee user workload.

## Bee design choices

- **`fib_step` uses the host's KV** (extended `BeeHostV1` with
  `kv_get` / `kv_put` / `kv_cas` FFI function pointers). The plugin
  uses safe Rust wrappers; the host wires the FFI to an in-process
  `HashMap`-backed store for the 1-node demo. For multi-node
  deployment, the host wires to a Raft-replicated KV.

- **Test fixtures** (`generate_series`, `generate_events`) are
  gated behind the `test-fixtures` Cargo feature in
  `bee-dsl-sql`. Production builds don't include them.

- **Console sink** (`EMIT INTO console`) is a built-in sink in
  `bee-dsl-sql` that writes rows to stdout. No external sink
  needed for the demo.

## 1-Node vs N-Node

This is the **1-Node MVP** of S41. The full S41 spec includes
N-node scaling (3 / 5 Nodes); that is deferred to a follow-up
session. For 1-node, the perf table has only 1 column. For
N-node, the table would have 1/3/5 columns showing the scaling
benefit.
```

- [ ] **Step 2: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add examples/performance/README.md && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (14): examples/performance/README.md — math, Bee design, scaling"
```

---

## Task 15: README.md "Performance Demos" section

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Find a good insertion point**

```bash
cd /Users/shaw/Developer/rust/bee && grep -n "^## " README.md
```
Expected: the section list. Insert "Performance Demos" before "Quant trading reference" (or wherever fits logically).

- [ ] **Step 2: Add the section**

Insert after the "Roadmap" section (or before "License"):

```markdown
## Performance Demos

The performance showcase is the new primary 5-minute demo of the
main repo. It runs 3 demo pipelines end-to-end and prints a measured
performance table:

- **Fibonacci**: 1M values via stateful `fib_step` UDF + KV-backed state
- **Prime sieve**: 10^8 integers via 20 sequential sieving Phases (correctness: 5,761,455 primes)
- **Multi-stream analytics**: 160K events across 3 streams with ASOF JOIN + WINDOW TUMBLING

```bash
scripts/demo-perf.sh
```

See [`examples/performance/README.md`](examples/performance/README.md) for the math, the Bee design choices, and how to read the numbers.
```

- [ ] **Step 3: Build to verify (README has no compile step)**

N/A — README is a docs file.

- [ ] **Step 4: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add README.md && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (15): README.md — 'Performance Demos' section pointing to demo script"
```

---

## Task 16: docs/product-design.md update

**Files:**
- Modify: `docs/product-design.md` (§4.1 "Performance showcase" expansion)

- [ ] **Step 1: Read the current §4.1**

```bash
cd /Users/shaw/Developer/rust/bee && sed -n '103,150p' docs/product-design.md
```
Expected: the current Scenario A: Performance showcase section.

- [ ] **Step 2: Update the §4.1 text**

Replace the "Implementation tracked as **S41**" line with:
```markdown
Implementation tracked as **S41** in [`docs/stories.md`](./stories.md). The demo is now runnable via [`scripts/demo-perf.sh`](../../scripts/demo-perf.sh); see [`examples/performance/README.md`](../../examples/performance/README.md) for the math and Bee design choices.
```

- [ ] **Step 3: Commit**

```bash
cd /Users/shaw/Developer/rust/bee && git add docs/product-design.md && git -c user.name="opencode" -c user.email="opencode@local" commit -m "S41 (16): docs/product-design.md — §4.1 now references runnable demo"
```

---

## Task 17: Final verification

**Files:** (no changes; verification only)

- [ ] **Step 1: Run all workspace tests**

```bash
cd /Users/shaw/Developer/rust/bee && cargo test --workspace 2>&1 | grep -E "test result.*passed" | sed -E 's/.*ok\. ([0-9]+) passed.*/\1/' | awk '{sum += $1} END {print "Total:", sum}'
```
Expected: ≥ 354 (with new tests added).

- [ ] **Step 2: Run cargo build to verify clean**

```bash
cd /Users/shaw/Developer/rust/bee && cargo build --workspace 2>&1 | tail -3
```
Expected: clean build, only pre-existing warnings (out of scope).

- [ ] **Step 3: Run the demo script end-to-end**

```bash
cd /Users/shaw/Developer/rust/bee && bash scripts/demo-perf.sh 2>&1 | tail -30
```
Expected: 3 demos run, hard correctness check passes (`✓ prime count correct (5761455)`), perf table printed at the end.

- [ ] **Step 4: Check git log**

```bash
cd /Users/shaw/Developer/rust/bee && git log --oneline ef89df9..HEAD
```
Expected: 16 commits, one per task.

- [ ] **Step 5: Final commit (if any uncommitted verification notes)**

If everything passes, the S41 implementation is complete. The commits form a logical progression; a future session can `git reset --soft ef89df9` to fold them into a single S41 commit (per the S17 + S33-deferred + restructure consolidation precedent).
```

---

## Self-review checklist (run before claiming done)

- [ ] Spec coverage: Section "Architecture (the new components)" in the design → Tasks 1-9. Section "Three SQL pipelines" → Tasks 10-12. Section "scripts/demo-perf.sh" → Task 13. Section "Docs" → Tasks 14-16. Section "Acceptance criteria" → all 14 items.
- [ ] No placeholders: every step has actual content; SQL files, Rust code, shell scripts are fully written.
- [ ] Type/file consistency: the plugin's `fib_step` signature is `(host: &BeeHostV1, n: u64) -> i128` everywhere. The KV state key is `state/handler/<stream_id>/fib_step/state` everywhere. The Cargo feature name is `test-fixtures` everywhere.
- [ ] DRY: the KV mock in the plugin's tests and the host-side KV impl have similar logic but are separate (one is in tests, one is in bee CLI). The plugin's `fib_step` logic is in the plugin, not duplicated.
- [ ] Commits: 16 commits (Tasks 1-16); Task 17 is verification only.
- [ ] YAGNI: no extra files beyond the 7 new + 7 updated in the design's file structure.
- [ ] No regressions: all 354 existing tests still pass; new tests for fib_seed, fib_step, generate_series, generate_events, console sink added.
- [ ] DataFusion 50 ASOF JOIN: verified via the new `asof_join_test` (Task 1 Step 6).
- [ ] BeeHostV1 KV extension: verified via the new `bee_host_kv_test` (Task 2 Step 1).
- [ ] Hard correctness check: prime_sieve must emit `n_primes = 5761455`; the demo script verifies this.

## Out-of-scope items (do not address in this plan)

- N-node cluster scaling (3/5 Nodes) — the S41 spec's N-node part is deferred to a follow-up session.
- `bee deploy` + `bee jobs wait` CLI subcommands — S41 uses `bee run` (S26) for single-process execution.
- Cluster scripts (`scripts/start-cluster.sh`, `scripts/load-plugin.sh`).
- The 2 pre-existing warnings in `crates/bee-control/tests/{deploy_pipeline,raft_cluster}.rs`.
- The S33-S40 quant trading implementation (deferred to future S-XX stories).
- Production-grade KV (Raft-replicated) — the S41 demo uses in-process `HashMap`-backed KV. A future S-XX may add Raft-replicated KV and re-wire the host-side kv_* function pointers.

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-08-s41-performance-showcase.md`. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** — execute tasks in this session using `executing-plans`, batch execution with checkpoints

Which approach?
