# S43 — Plugin KV Port + Adapters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `Kv` port trait + `InProcessKv` + `HostKv` adapters to `crates/bee-plugin-sdk`, so plugin authors have a uniform trait for KV access with two concrete adapters (one in-process, one wrapping the host's FFI).

**Architecture:** Apply the stash's `kv.rs` (228 lines, untracked in `stash@{0}^3`) as the starting point. Add `pub mod kv;` to `crates/bee-plugin-sdk/src/lib.rs` so the new module is exported. Add a 1-paragraph doc note explaining the port-vs-adapter pattern. Add 2 NEW tests (mock FFI round-trip + not-found) on top of the stash's 3 in-process tests.

**Tech Stack:** Rust, `tokio` (sync + `tokio::sync::Mutex`), `Arc<dyn Kv>`, `BeeHostV1` FFI.

---

## File Structure

| File | Action | Purpose |
|---|---|---|
| `crates/bee-plugin-sdk/src/kv.rs` | Create | New module: `Kv` trait + `InProcessKv` + `HostKv` + 3 in-process tests |
| `crates/bee-plugin-sdk/src/lib.rs` | Modify | `pub mod kv;` + 1-paragraph doc note about port-vs-adapter |
| `crates/bee-plugin-sdk/src/kv.rs::tests` | Modify | 2 new tests (mock FFI round-trip + not-found) |

3 Tasks. Task 1 applies the stash's `kv.rs` (the existing module + 3 in-process tests). Task 2 adds the 2 NEW tests with a mock FFI. Task 3 wires the module export + adds the doc note + final verification + commit + push.

---

## Task 1: Apply stash's `kv.rs` module

**Files:**
- Create: `crates/bee-plugin-sdk/src/kv.rs` (apply from stash untracked)
- Modify: `crates/bee-plugin-sdk/src/lib.rs` (add `pub mod kv;` + doc note)

- [ ] **Step 1.1: Apply `kv.rs` from the stash (untracked file)**

Run:

```bash
git show stash@{0}^3:crates/bee-plugin-sdk/src/kv.rs > crates/bee-plugin-sdk/src/kv.rs
git status
```

Expected: 1 new untracked file `crates/bee-plugin-sdk/src/kv.rs`. No other files touched.

- [ ] **Step 1.2: Add `pub mod kv;` to `crates/bee-plugin-sdk/src/lib.rs`**

In `crates/bee-plugin-sdk/src/lib.rs`, find the module declarations (they look like `pub mod event; pub mod macros; pub mod vtable;` or similar). Add `pub mod kv;` next to them.

Concretely, the file currently has:

```rust
pub mod event;
pub mod macros;
pub mod vtable;
```

Add `pub mod kv;` to make it:

```rust
pub mod event;
pub mod kv;
pub mod macros;
pub mod vtable;
```

(Alphabetical ordering.)

- [ ] **Step 1.3: Build to verify the new module compiles**

Run: `cargo build -p bee-plugin-sdk 2>&1 | tail -5`. Expected: clean build (the new module should compile standalone).

If there are errors due to missing imports inside the new module, the next step fixes them.

- [ ] **Step 1.4: Run the in-process tests to confirm 3 stash tests pass**

Run: `cargo test -p bee-plugin-sdk --lib kv:: 2>&1 | tail -10`. Expected: 3 tests pass (`in_process_kv_roundtrip`, `in_process_kv_is_shared_across_adapters`, `in_process_kv_default_is_isolated`).

- [ ] **Step 1.5: Run the full workspace test suite to confirm no regression**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 423+ failed: 0 ignored: 5` (baseline 420 + 3 new stash tests).

- [ ] **Step 1.6: Commit**

```bash
git add crates/bee-plugin-sdk/src/kv.rs crates/bee-plugin-sdk/src/lib.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S43 Task 1: add kv module (Kv trait + InProcessKv + HostKv adapters)"
```

---

## Task 2: Add `HostKv` round-trip + not-found tests (TDD)

**Files:**
- Modify: `crates/bee-plugin-sdk/src/kv.rs::tests` (append 2 tests)

These tests use a mock `BeeHostV1` with function pointers that read/write a `Mutex<HashMap<String, Vec<u8>>>`. They lock down the `HostKv` adapter's behavior across the FFI boundary.

- [ ] **Step 2.1: Write the failing tests (RED)**

In `crates/bee-plugin-sdk/src/kv.rs`, find the closing `}` of the `mod tests` block and add 2 tests just before it:

```rust
    #[test]
    fn host_kv_round_trip_through_mock_ffi() {
        // Mock KV store. The mock `kv_get` / `kv_put` functions
        // read/write this `Mutex<HashMap>` to simulate the host's
        // cluster KV.
        use std::collections::HashMap;
        use std::sync::Mutex;
        static STORE: std::sync::OnceLock<Mutex<HashMap<String, Vec<u8>>>> =
            std::sync::OnceLock::new();
        let store = STORE.get_or_init(|| Mutex::new(HashMap::new()));
        store.lock().unwrap().clear();

        // Mock FFI: kv_get reads from the store, kv_put writes to
        // the store.
        unsafe extern "C" fn mock_kv_get(
            _ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            out_value: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            let c_key = std::ffi::CStr::from_ptr(key);
            let key = c_key.to_str().unwrap();
            let store = STORE.get().unwrap();
            let store = store.lock().unwrap();
            match store.get(key) {
                Some(v) => {
                    let len = v.len();
                    let ptr = v.as_ptr();
                    *out_value = ptr as *mut u8;
                    *out_len = len;
                    0
                }
                None => 1,
            }
        }

        unsafe extern "C" fn mock_kv_put(
            _ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            value: *const u8,
            len: usize,
        ) -> i32 {
            let c_key = std::ffi::CStr::from_ptr(key);
            let key = c_key.to_str().unwrap().to_string();
            let bytes = std::slice::from_raw_parts(value, len).to_vec();
            let store = STORE.get().unwrap();
            store.lock().unwrap().insert(key, bytes);
            0
        }

        // Mock host with the FFI slots populated.
        let host = crate::BeeHostV1 {
            ctx: std::ptr::null_mut(),
            register_adapter: None,
            register_input_adapter_vtable: None,
            register_output_adapter_vtable: None,
            register_handler_vtable: None,
            kv_get: Some(mock_kv_get),
            kv_put: Some(mock_kv_put),
            kv_cas: None,
            current_stream_id: None,
        };

        let host_ptr = &host as *const crate::BeeHostV1;
        let kv = unsafe { HostKv::new(host_ptr, std::ptr::null_mut()) };

        // Round-trip: put then get.
        kv.put("hello", b"world".to_vec());
        assert_eq!(kv.get("hello"), Some(b"world".to_vec()));

        // Overwrite: put replaces.
        kv.put("hello", b"rust".to_vec());
        assert_eq!(kv.get("hello"), Some(b"rust".to_vec()));
    }

    #[test]
    fn host_kv_returns_none_on_not_found() {
        // Same mock FFI as above; the test focuses on the
        // not-found return path.
        use std::collections::HashMap;
        use std::sync::Mutex;
        static STORE: std::sync::OnceLock<Mutex<HashMap<String, Vec<u8>>>> =
            std::sync::OnceLock::new();
        let store = STORE.get_or_init(|| Mutex::new(HashMap::new()));
        store.lock().unwrap().clear();

        unsafe extern "C" fn mock_kv_get(
            _ctx: *mut std::ffi::c_void,
            key: *const std::ffi::c_char,
            out_value: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            let c_key = std::ffi::CStr::from_ptr(key);
            let key = c_key.to_str().unwrap();
            let store = STORE.get().unwrap();
            let store = store.lock().unwrap();
            match store.get(key) {
                Some(v) => {
                    let len = v.len();
                    let ptr = v.as_ptr();
                    *out_value = ptr as *mut u8;
                    *out_len = len;
                    0
                }
                None => 1,
            }
        }

        let host = crate::BeeHostV1 {
            ctx: std::ptr::null_mut(),
            register_adapter: None,
            register_input_adapter_vtable: None,
            register_output_adapter_vtable: None,
            register_handler_vtable: None,
            kv_get: Some(mock_kv_get),
            kv_put: None,
            kv_cas: None,
            current_stream_id: None,
        };
        let host_ptr = &host as *const crate::BeeHostV1;
        let kv = unsafe { HostKv::new(host_ptr, std::ptr::null_mut()) };

        // No `put` ever called — get returns None.
        assert_eq!(kv.get("missing"), None);
    }
```

- [ ] **Step 2.2: Run the tests to verify they fail (RED)**

Run: `cargo test -p bee-plugin-sdk --lib host_kv 2>&1 | tail -10`. Expected: at least one test FAILS — likely a lifetime or borrowing issue with the `mock_kv_get` closures (the closure must not capture `store` by reference since the FFI expects `unsafe extern "C" fn` with a C-compatible signature).

If the tests pass on the first try, that's fine too (the stash's design is sound); mark this step done.

- [ ] **Step 2.3: Fix any compile errors**

If Step 2.2 surfaced errors, the common issues are:

- **`unsafe extern "C" fn` cannot capture closures** — the static `STORE` is a `static`, accessed via `STORE.get().unwrap()`. The closure must use only `static` references (no captured locals).
- **Lifetime of `&host as *const BeeHostV1`** — the mock host must outlive the `HostKv`; in the test we use a stack-allocated `host` so its lifetime is the test fn body.

After the fix, re-run: `cargo test -p bee-plugin-sdk --lib host_kv 2>&1 | tail -5`. Expected: 2 tests pass.

- [ ] **Step 2.4: Run all `kv` tests to confirm 5/5 pass**

Run: `cargo test -p bee-plugin-sdk --lib kv:: 2>&1 | tail -10`. Expected: 5 tests pass (3 stash tests + 2 new mock-FFI tests).

- [ ] **Step 2.5: Commit**

```bash
git add crates/bee-plugin-sdk/src/kv.rs
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S43 Task 2: add HostKv round-trip + not-found tests with mock FFI"
```

---

## Task 3: Doc note + final verification + push

**Files:**
- Modify: `crates/bee-plugin-sdk/src/lib.rs` (add 1-paragraph doc note at top)

- [ ] **Step 3.1: Add the port-vs-adapter doc note**

In `crates/bee-plugin-sdk/src/lib.rs`, find the top of the file (above the existing module declarations or above the `use` statements). Add a 1-paragraph module-level doc note:

```rust
//! Bee Plugin SDK: the Rust contract that plugin authors implement
//! to load their `.so` / `.dylib` into a Bee host.
//!
//! # Plugin ↔ Host boundary
//!
//! Plugins are loaded via `libloading` and registered with the
//! host via the `bee_plugin_init` entry point (see `cdylib_plugin!`).
//! Plugins declare their Adapters / Handlers + a `PluginManifest`;
//! the host stores the plugin's vtables in its `PluginHandle`.
//!
//! # Port vs Adapter (the `kv` module)
//!
//! Per the project's LANGUAGE.md: *one adapter = hypothetical seam;
//! two adapters = real one*. The `kv` module introduces a `Kv` port
//! trait with two concrete adapters — `InProcessKv` (a process-global
//! `HashMap` for tests + plugin MVP) and `HostKv` (wraps the
//! `BeeHostV1::kv_get` / `kv_put` FFI function pointers for production).
//! Plugin authors hold an `Arc<dyn Kv>` and call `.get(key)` / `.put(key,
//! value)` regardless of which adapter is in use. The host-allocated
//! bytes returned by `HostKv::get` are leaked in the MVP (the plugin
//! process exits shortly); threading the host's free fn pointer is a
//! follow-up.
```

(Replace the existing top-of-file doc comment with this expanded version. If the existing comment is different, integrate the port-vs-adapter paragraph into the existing module-level doc — don't delete existing context.)

- [ ] **Step 3.2: Full workspace build**

Run: `cargo build --workspace 2>&1 | tail -5`. Expected: clean build.

- [ ] **Step 3.3: Full workspace test suite**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed:", p, "failed:", f, "ignored:", i}'`. Expected: `passed: 425+ failed: 0 ignored: 5` (baseline 420 + 5 new S43 tests: 3 from stash + 2 new mock-FFI).

- [ ] **Step 3.4: Update the S42 spec's acceptance criteria (skip if already done)**

No-op — the S43 spec is the one that flips `[ ]` to `[x]`. Edit `docs/superpowers/specs/2026-07-17-s43-plugin-kv-port-design.md` and flip all 6 acceptance criteria to `[x]`. Commit:

```bash
git add docs/superpowers/specs/2026-07-17-s43-plugin-kv-port-design.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S43: flip acceptance criteria to [x]"
```

- [ ] **Step 3.5: Update `docs/stories.md` S43 acceptance criteria**

Edit `docs/stories.md` (the S43 section, line ~1147). Flip the acceptance criteria from `[ ]` to `[x]`. Commit:

```bash
git add docs/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "stories.md: S43 acceptance criteria flipped"
```

- [ ] **Step 3.6: Push to remote**

```bash
git push origin main
```

---

## Self-Review

**1. Spec coverage:** Walked the S43 spec's in-scope items:
- New `kv.rs` module with `Kv` trait + `InProcessKv` + `HostKv`: Task 1 ✓
- Mock FFI round-trip test: Task 2 ✓
- Not-found test: Task 2 ✓
- `pub mod kv;` export: Task 1 ✓
- Doc note at top of `lib.rs`: Task 3 ✓
- Stash apply: Task 1 ✓

**2. Placeholder scan:** Searched for "TBD" / "TODO" / "implement later" — none in the plan body. The "out of scope" deferrals in the spec are intentional and listed in the Sign-off matrix.

**3. Type consistency:**
- `Kv` trait shape: `fn get(&self, key: &str) -> Option<Vec<u8>>` + `fn put(&self, key: &str, value: Vec<u8>)` — consistent across the stash's `kv.rs` and Task 2's tests.
- `HostKv::new(host: *const BeeHostV1, ctx: *mut c_void) -> Arc<Self>` — consistent with the stash.
- `unsafe extern "C" fn` signatures for the mock match `BeeHostV1::kv_get` / `kv_put` exactly.

**4. Ambiguity check:** Each test specifies concrete input + concrete expected output. The mock FFI closures use a process-global `Mutex<HashMap>` for state (matches the pattern of `InProcessKv::new()`'s `OnceLock`). The doc note text is concrete (specific callouts to "LANGUAGE.md" + the port-vs-adapter rule).

---

## Estimated Total

- 3 Tasks
- 4-6 commits (Task 1 = 1, Task 2 = 1-2 with fixes, Task 3 = 2-3 verification)
- ~250 LOC net change (228 from stash + 2 new tests + doc note)
- Estimated wall-clock: 30-60 minutes of focused TDD work
