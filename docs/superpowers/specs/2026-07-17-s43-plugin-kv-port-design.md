# S43 — Plugin KV Port + Adapters

**Date:** 2026-07-17
**Type:** AFK
**Blocked by:** S29 (Datasource managed entity — for the plugin-side use case of Producer HWM)
**ADRs:** 0004 (KV cluster), 0005 (plugin FFI), 0009 (multi-version)
**Status:** Draft (pending review)
**Source WIP:** `stash@{0}^3` — `crates/bee-plugin-sdk/src/kv.rs` (untracked, 228 lines)

## Why this story exists

Plugins that need per-stream state (e.g. a Producer's high-water mark for backfill-on-subscribe, per-Subscriber offset tracking, transient aggregation buffers) require read/write access to a key-value store. Two designs have coexisted in the codebase:

1. **`BeeHostV1::safe_kv_get` / `safe_kv_put`** — the lower-level FFI wrapper, already at HEAD. Plugin authors call `host.safe_kv_get(key)` / `host.safe_kv_put(key, value)` directly. This works but mixes two concerns: the plugin must know it's calling FFI, and there's no abstraction over "where the KV lives".

2. **Hand-written `kv_stub` per plugin** — the prior practice (per the stash comment "two plugins had drifted between `OnceLock` and `LazyLock`"). Each plugin wrote its own process-global `HashMap<String, Vec<u8>>` wrapper. Two plugins had drifted between `OnceLock` and `LazyLock`. The drift is a maintenance smell.

S43 introduces a **port + adapters** pattern that gives plugin authors a single, uniform `Kv` trait with two adapters:

- **`InProcessKv`** — a process-global `HashMap<String, Vec<u8>>`. For tests + plugin MVP when the host's `BeeHostV1` doesn't provide KV slots.
- **`HostKv`** — wraps the host's `BeeHostV1::kv_get` / `kv_put` FFI function pointers. For production: writes go to the host's cluster KV (per ADR-0004).

The two adapters justify the seam — per the project's LANGUAGE.md: *one adapter = hypothetical seam; two adapters = real one*. Plugin authors hold an `Arc<dyn Kv>` and call `.get(key)` / `.put(key, value)` through the trait, regardless of which adapter is in use.

## What already exists at HEAD (non-changes)

`crates/bee-plugin-sdk/src/lib.rs` already exposes:

- `BeeHostV1::kv_get` / `kv_put` / `kv_cas` slots (FFI function pointers)
- `BeeHostV1::safe_kv_get` / `safe_kv_put` (safe Rust wrappers that bincode-decode + copy the value out)
- The `SdkError::HostFnMissing("kv_get")` + `SdkError::KvError("kv_get failed")` variants
- A compile-time test (`bee_host_v1_has_kv_function_pointers`) that asserts the slots exist

**S43 does not change `BeeHostV1`.** It adds a new module + the `Kv` trait + adapters alongside the existing safe wrappers. The safe wrappers remain the lower-level API for plugin authors who want direct FFI access; the `Kv` trait is the higher-level abstraction.

## Scope

### In scope

1. **`crates/bee-plugin-sdk/src/kv.rs`** (new module, 228 lines from the stash):
   - `pub trait Kv: Send + Sync + 'static` with `fn get(&self, key: &str) -> Option<Vec<u8>>` + `fn put(&self, key: &str, value: Vec<u8>)`
   - `pub struct InProcessKv` (process-global `OnceLock<Mutex<HashMap<String, Vec<u8>>>>`; `Default::default()` returns a fresh per-instance map for test isolation)
   - `pub struct HostKv` (wraps `BeeHostV1::kv_get` / `kv_put`; `unsafe impl Send + Sync`; constructed via `unsafe fn new(host: *const BeeHostV1, ctx: *mut c_void) -> Arc<Self>`)
   - `mod tests` with 3 in-process tests (round-trip, shared-across-instances, default-isolated)
2. **Add a `HostKv` round-trip test** (NEW; not in the stash): a mock `BeeHostV1` with function pointers that read/write a `Mutex<HashMap<String, Vec<u8>>>` (the mock KV store); assert `HostKv::get` / `HostKv::put` round-trip through the FFI.
3. **Add a `HostKv` not-found test**: assert `HostKv::get("missing")` returns `None` when the FFI returns 1 (not-found).
4. **`crates/bee-plugin-sdk/src/lib.rs` doc note**: add a 1-paragraph comment at the top of the file explaining the port-vs-adapter pattern ("1 adapter = hypothetical seam; 2 adapters = real one"). Reference the new module.
5. **Add `pub mod kv;` to `crates/bee-plugin-sdk/src/lib.rs`** so the new module is exported.
6. **Stash apply** — apply `kv.rs` from `stash@{0}^3` (untracked file) on top of HEAD with no manual merge work (the file is new).

### Out of scope (deferred)

- **A real `HostKv::get` that frees the host-allocated bytes** — the stash leaks them ("the bytes are leaked (the plugin process exits shortly); a S33.6.x follow-up will thread the host's free fn pointer"). S43 inherits this MVP leak; threading the free fn pointer is a S43.x follow-up.
- **Plugin-side migration** — no existing plugin is updated to use the `Kv` trait. Migration of `bee-plugin-binance` / `bee-plugin-google-news` / etc. to `Arc<dyn Kv>` is a separate story once the runtime semantics are validated.
- **`HostKv::cas`** — the stash's trait has only `get` / `put`; `kv_cas` is in `BeeHostV1` but not wrapped by the trait. A CAS-aware trait (`trait KvCas: Kv { fn cas(...) -> ... }`) is a follow-up.
- **`current_stream_id`** — `BeeHostV1` exposes this slot too (returns a 32-byte stream ID hash); not wrapped by `Kv`. Out of scope.

## File structure

| File | Action | Purpose |
|---|---|---|
| `crates/bee-plugin-sdk/src/kv.rs` | Create | New module with `Kv` trait + `InProcessKv` + `HostKv` |
| `crates/bee-plugin-sdk/src/lib.rs` | Modify | Add `pub mod kv;` + 1-paragraph doc note about port-vs-adapter |
| `crates/bee-plugin-sdk/src/kv.rs::tests` | Modify | Add 2 tests (`HostKv` round-trip + not-found) |

## Acceptance criteria

- [ ] `cargo build --workspace` green
- [ ] `cargo test --workspace` ≥ 420 passed, 0 failed
- [ ] `cargo test -p bee-plugin-sdk` — new + refreshed tests pass:
  - `kv::tests::in_process_kv_roundtrip`
  - `kv::tests::in_process_kv_is_shared_across_adapters`
  - `kv::tests::in_process_kv_default_is_isolated`
  - `kv::tests::host_kv_round_trip_through_mock_ffi`
  - `kv::tests::host_kv_returns_none_on_not_found`
- [ ] `kv_get` / `kv_put` exposed in `BeeHostV1` (already true; verified by `bee_host_v1_has_kv_function_pointers` test)
- [ ] `HostKv` adapter round-trips through the FFI without panicking
- [ ] Stash's `kv.rs` applied on top of HEAD; existing plugin code paths unaffected (no plugin imports the new module yet)
- [ ] Doc note at top of `crates/bee-plugin-sdk/src/lib.rs` mentions the port-vs-adapter pattern with a 1-line reference to the new `kv` module

## Sign-off matrix

| Item | Code-level | Production-level |
|---|---|---|
| `Kv` trait + `InProcessKv` + `HostKv` adapters | ✓ (S43 + 5 tests) | N — third-party plugin migration |
| `HostKv` round-trip through mock FFI | ✓ | N |
| Doc note explaining port-vs-adapter | ✓ | N |
| Real `HostKv::get` that frees host bytes | — | N — S43.x follow-up |
| Plugin-side migration to `Arc<dyn Kv>` | — | N — separate migration story |
| `KvCas` trait for `kv_cas` | — | N — follow-up |
| `current_stream_id` wrapper | — | N — follow-up |

## Related work

- **ADR-0004** (KV cluster) — `HostKv::put` in production writes to the cluster KV via the host's `BeeHostV1::kv_put`; the bytes flow through the same path as the host's own KV operations.
- **ADR-0005** (plugin FFI) — `kv_get` / `kv_put` are part of the `BeeHostV1` C struct; `HostKv` wraps them with safe Rust.
- **S33.5.2** (`RegisterDatasource` validation) — independent of `Kv`; the Datasource validation doesn't touch plugin state.
- **S42** (Sink DSL) — independent; S42 routes rows to plugins but doesn't use the `Kv` trait for sink state.
- **S44** (S41 demo cleanup) — independent.

## Decision matrix (for the user)

| Question | Choice | Notes |
|---|---|---|
| Where does the `Kv` trait live? | **New module `crates/bee-plugin-sdk/src/kv.rs`** | Matches the stash; one feature = one module |
| Deprecate `BeeHostV1::safe_kv_get`? | **No** — keep both | `safe_kv_get` is the lower-level API; `Kv` is the higher-level abstraction. Plugin authors pick. |
| Add `KvCas` trait for CAS? | **Defer** — S43 ships `get`/`put` only | CAS is rare in plugins; add later if a real use case shows up |
| Free host-allocated bytes in `HostKv::get`? | **Defer** — inherit the stash's leak for MVP | S43.x follow-up threads the host's free fn pointer |
| Migrate any existing plugin? | **No — S43 only** | Plugin migration is a separate story once the trait is battle-tested |

If any of these decisions should change, the user can override during the spec review.
