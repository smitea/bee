# S33.5.2 — RegisterDatasource 完整校验 (the S33.5 follow-up)

**Date:** 2026-06-10
**Type:** AFK
**Blocked by:** S33.5 (leader-side Raft-log apply)
**Status:** Approved (2026-06-10, from S33.5.1 sign-off)

## Why this story exists

S33.3 added the `AdminRequest::RegisterDatasource` arm and S33.5 wired the leader-side apply, but the validation is minimal — the arm accepts the request, builds an `Op::RegisterDatasourceProducer`, and submits. It does not check:

- name format (empty, length, charset)
- version spec parses
- config is valid JSON + has no per-call args (symbol, interval, query)
- tenant is a legal u16
- the named **adapter is actually loaded** (a Plugin that provides it is registered with `PluginManager`)

The local CLI (`bee datasource create`) already does steps 5-6 + plugin_id hashing, but the remote CLI (`bee --connect <addr> datasource create`) hits `AdminServer::dispatch(RegisterDatasource)` which does **none** of these. The user can register a Datasource that points at an adapter that does not exist; the failure only surfaces when a Pipeline tries to start a Phase on it.

This story closes that gap: make the AdminServer's `RegisterDatasource` arm validate everything before submitting the op, and use the **real `Datasource` struct** (per S29 / ADR-0010) instead of the placeholder `datasource/{name}` signature.

## Scope

### In scope (3 deliverables)

1. **Validation chain in `AdminServer::dispatch_with_apply(RegisterDatasource)`**
   - 9 checks, all returning `AdminResponse::RegisterDatasourceAck { ok: false, error_msg: "..." }` on failure:
     1. `name` is non-empty
     2. `name` length ≤ 64
     3. `name` matches `[a-zA-Z0-9_.-]+`
     4. `tenant` is in `0..=65535` (struct field is `u16`, so wire-side; a malformed value past 65535 cannot reach here, but we keep the check for the local `Datasource::new` path)
     5. `plugin_version` parses as `bee_plugin_sdk::VersionSpec`
     6. `config_json` is valid JSON
     7. `config_json` has no per-call-arg keys (delegate to `bee_dsl_sql::preprocess::validate_datasource_config`)
     8. `adapter` is in the loaded Plugins' adapter list
     9. `plugin_manager.resolve(adapter, &version_spec)` returns `Some(PluginId)` (i.e., the version spec actually matches a loaded Plugin)
   - On all-pass: build a `Datasource` struct and store under KV key `ds/{tenant}/{name}` via `Op::Put`. Return `RegisterDatasourceAck { ok: true, error_msg: "" }`.

2. **Wire `PluginManager` into `AdminServer` + `dispatch_with_apply`**
   - `AdminServer::start` gains an 8th arg: `plugin_manager: Arc<PluginManager>`. (The `run_node` binary already constructs a `PluginManager`; it threads the same Arc into both the AdminServer and the plugin loader.)
   - `dispatch_with_apply` takes `plugin_manager: &PluginManager` (or `Option<&PluginManager>` for tests; `None` skips the plugin-existence checks with a clear error).
   - The `RegisterDatasource` arm calls `plugin_manager.list_adapters()` (step 8) and `plugin_manager.resolve(adapter, &version_spec)` (step 9).
   - The `Deploy` arm's `Op::RegisterDatasourceProducer` path (which uses `RegisterDatasource`'s same machinery) goes through the same validation.

3. **Test seam**: `AdminServer::start` also accepts `plugin_manager: Option<Arc<PluginManager>>` — `None` for tests that don't exercise plugin-existence. The existing 3 `admin_write_roundtrip` tests (and the 4 follow-ups) pass `None` and bypass steps 8-9 (the validation chain returns `Error("plugin_manager not wired; cannot validate adapter")` for `RegisterDatasource` only).

### Out of scope (deferred)

- Full `bee-dsl-sql` runner behind `Deploy` (DAG → Tasks) — S33.5.3.
- Tenant ACL enforcement (Job can only `use` Datasources in its own tenant or tenant 0) — 1.x per ADR-0010.
- `bee datasource test <name>` (probe via Plugin's `test_connection` method) — S40+ per S29 docs.
- `bee datasource pause` triggers Draining on referencing Jobs — S31 per S29 docs.
- `Datasource::update` (the `PUT` semantic) — S30+ per S29 docs.

## Design

### Validation order

The 9 checks are independent (no cross-check dependencies) but ordered for fail-fast + helpful error messages:

1-4: cheap, no I/O. Fail-fast on bad name.
5-7: cheap, no I/O. Fail-fast on bad input format.
8: in-memory lookup (O(N_plugins)). `plugin_manager.list_adapters()` returns `(PluginId, AdapterDescriptor)` for every adapter in every loaded plugin. Check if **any plugin's `manifest.name` equals the request's `adapter`** (the "adapter" in the request is actually the plugin's logical name per ADR-0010; the `AdapterDescriptor.name` field is the method name, e.g. "subscribe"). Fail with a "load plugin first" message that lists loaded plugin names.
9: `plugin_manager.resolve(adapter, &version_spec)` returns `Some(PluginId)` (a Plugin with matching name + a version that satisfies the spec exists). Fail with a "no plugin matches 'X' @ 'Y'" message.

(Steps 8 and 9 both lookup by plugin name, not adapter-descriptor name. The binance.subscribe() example: "binance" is the plugin name / datasource name; "subscribe" is the adapter method on the binance plugin.)

### Wire types

No changes. `AdminRequest::RegisterDatasource { name, adapter, plugin_version, config_json, tenant, owner_node }` already has all 9 fields needed. `AdminResponse::RegisterDatasourceAck { ok, error_msg }` already supports the error path.

### KV key format

`ds/{tenant}/{name}` per ADR-0010 ("stored in KV at `ds/{tenant}/{name}`"). The value is the bincode-serialized `Datasource` struct.

**Implementation note**: `Datasource` currently derives `Debug, Clone, PartialEq, Eq` but **not** `Serialize, Deserialize`. The implementation must add `serde::Serialize, serde::Deserialize` derives (and the `serde` feature on `bee-plugin-sdk` if not already enabled, since `PluginId` and `VersionSpec` need to be serializable). A new `tests/serde_compat.rs` (or extend an existing test) asserts bincode round-trip works for the struct.

The S33.3 MVP wrote to `soak/datasource/{name}` (a flat namespace, not tenant-scoped). S33.5.2 changes this to `ds/{tenant}/{name}`. The old flat keys are abandoned (no migration; the MVP had no real users).

### Error message format

Each check returns a human-readable string. Examples:

- `name must be non-empty`
- `name 'foo!' has invalid chars; allowed: a-z A-Z 0-9 _ . -`
- `invalid plugin-version '1.x': expected semver (e.g. 1.2.3)`
- `config is not valid JSON: <serde error>`
- `config: key 'symbol' is a per-call arg; it belongs at the call site (e.g. 'binance.subscribe('symbol', ...)')`
- `adapter 'unknown' is not loaded; load plugin first (e.g. bee plugin load <path>)`
- `no plugin matches adapter 'binance' @ '1.0.0'; loaded plugins: <list>`

The CLI surface (in `bee/src/main.rs`) surfaces the error string verbatim.

### PluginManager plumbing

`bee/src/run_node.rs` currently does:
1. `let mut plugin_manager = PluginManager::new();`
2. `plugin_manager.load_directory(&plugin_dir)?;` (loads .so / .dylib)
3. (PluginAdapterRegistry::global() is populated as a side effect)

S33.5.2 adds: `let plugin_manager = Arc::new(plugin_manager);` (after load) and threads the `Arc<PluginManager>` into both:
- The PluginManager consumers that already exist (the Adapter registry is already populated, so no further wiring needed)
- The new `AdminServer::start(addr, kv, cp, state, stats, transport, register_reply, plugin_manager)` 8th arg

The in-memory tests (admin_forwarding_inmem, admin_write_roundtrip) pass `None` for `plugin_manager`; the validation chain returns `Error("plugin_manager not wired; cannot validate adapter")` if `RegisterDatasource` is called. The `KvPut` and `Deploy` arms are unaffected.

### Test seam

`AdminServer::start` signature becomes:

```rust
pub async fn start(
    addr: SocketAddr,
    kv: Arc<Mutex<KVStateMachine>>,
    cp: Arc<Mutex<ControlPlaneStateMachine>>,
    state: Arc<Mutex<NodeState>>,
    stats: Option<Arc<Mutex<HashMap<u32, TaskRuntimeStats>>>>,
    node_transport: Option<Arc<dyn NodeTransport>>,
    register_reply: Option<AdminReplyRegistrar>,
    plugin_manager: Option<Arc<PluginManager>>,
) -> Result<Self, String>
```

The 3 existing `admin_write_roundtrip` tests + the 1 `admin_forward_smoke` test + the new `admin_forwarding_inmem` test all update their `AdminServer::start` calls to pass `None` for `plugin_manager`. The `RegisterDatasource` roundtrip test in `admin_write_roundtrip.rs` (Test 2) is replaced: it now tests the validation failure path (since with `plugin_manager = None`, the happy path returns the "plugin_manager not wired" error). The happy path moves to a new test file `admin_datasource_validation.rs`.

### Edge cases

- **Empty plugin_manager + RegisterDatasource**: returns the "plugin_manager not wired" error. This is the S33.5.2 MVP behavior for tests; production (run_node) always wires it.
- **Two RegisterDatasource calls with the same (tenant, name)**: the second one overwrites the first in KV. The DatasourceRegistry would catch this in-process, but the AdminServer path goes directly to KV (S33.5.2 MVP). A S33.5.x follow-up adds the registry pre-check (returns `RegisterDatasourceAck { ok: false, error_msg: "datasource 'binance' already exists in tenant 0" }`).
- **config_json is "{}"**: passes (empty JSON object has no per-call-arg keys).
- **config_json is null**: passes `validate_datasource_config` (it checks `if let Some(obj) = config.as_object()`; null is not an object, no keys to check). The `serde_json::from_str` succeeds, the struct field is set to the literal "null" string (acceptable MVP behavior; the Plugin receives it as `"null"` and can decide).
- **plugin_version is "*" or ">=0"**: `VersionSpec::parse` accepts these. `PluginManager::resolve` with a wide spec may return any matching plugin; the test asserts the resolved PluginId matches what the loaded plugin registered.

## Test plan (4 tests in `crates/bee-control/tests/admin_datasource_validation.rs`)

### 1. `register_datasource_validates_name_empty`

```rust
// name = ""
// plugin_manager = None
// expected: RegisterDatasourceAck { ok: false, error_msg: "name must be non-empty" }
```

### 2. `register_datasource_validates_name_chars`

```rust
// name = "binance!"
// plugin_manager = None
// expected: RegisterDatasourceAck { ok: false, error_msg contains "invalid chars" }
```

### 3. `register_datasource_validates_adapter_loaded`

```rust
// name = "binance", adapter = "unknown_adapter"
// plugin_manager = Some(Arc::new(PluginManager::new())) // empty
// expected: RegisterDatasourceAck { ok: false, error_msg contains "is not loaded" }
```

### 4. `register_datasource_full_happy_path`

```rust
// Build a PluginManager with a fake plugin registered
// (PluginManager::register_plugin(...) with a stub manifest
// that lists adapter = "binance" with version "1.0.0").
// name = "binance", adapter = "binance",
// plugin_version = "1.0.0", config = "{}"
// expected: RegisterDatasourceAck { ok: true }
// Then ListKv("ds/0/") returns 1 entry with key
// "ds/0/binance" and value = bincode(Datasource{...}).
```

The test in `admin_write_roundtrip.rs` (test 2, `admin_register_datasource_roundtrip`) is updated to pass `plugin_manager: None` and assert on the "plugin_manager not wired" error (the old happy-path assertion is removed; moved to test 4 here).

## Dependencies

- `bee-control` already depends on `bee-plugin-sdk` and `bee-dsl-sql` (per S29).
- New dep: `bee-registry = { path = "../bee-registry" }` in `crates/bee-control/Cargo.toml` (currently absent; verify in implementation).
- `bee-dsl-sql` is already a dep; `preprocess::validate_datasource_config` is the public API.

## Sign-off matrix

| Item | Code-level (this story) | Production-level (1.x) |
|------|------------------------|------------------------|
| Validation chain 1-9 (admin RPC) | ✓ (4 tests) | N — 24h wall-clock run on real cluster |
| KV key `ds/{tenant}/{name}` | ✓ (verified in test 4) | N — same |
| PluginManager plumbing in run_node | ✓ (8th arg) | N — same |
| Tenant ACL enforcement (use only own tenant) | N (deferred to 1.x per ADR-0010) | N |
| `bee datasource test` probe | N (deferred to S40+) | N |
| Datasource pause triggers Draining | N (deferred to S31) | N |
| Conflict detection (same name twice) | N (MVP overwrites; S33.5.x follow-up) | N |

## Related work

- S29: original Datasource data model + `DatasourceRegistry` (in-process, not persisted to KV).
- S30: KV persistence (originally planned; S33.5.2 closes the gap for the `RegisterDatasource` write path).
- S33.3: added the `RegisterDatasource` arm + wrote to flat `soak/datasource/{name}`.
- S33.5: leader-side apply path (the wire up to this story's validation).
- S33.5.1: cross-node forwarding (the path the validation runs on).
- ADR-0010: Datasource as a managed Provider, `use` syntax, tenant namespace.
