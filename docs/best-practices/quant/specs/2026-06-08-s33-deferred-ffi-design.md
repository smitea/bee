# S33 deferred · FFI wire format + runtime dispatching + 2 SQL pipelines + demo

**Date**: 2026-06-08
**Status**: design — pending approval
**Owner**: S33 (deferred items)
**Supersedes**: S33 design's "Out of scope" list (`docs/superpowers/specs/2026-06-07-s33-plugin-crates-design.md`)
**Blocked by**: S19 (Plugin trait + BeeHostV1) — already done
**Story**: `docs/stories.md` §S33 (lines 865–936) + S40 (S40 is the e2e deploy; S33 deferred items are its prerequisites)

## Scope (this session)

The four deferred items from the S33 design:

1. **Production wire format for adapter events across the FFI** —
   how `Event` (from `crates/bee-adapter/src/lib.rs`) crosses the
   `cdylib` boundary, with stable ABI + zero host-side knowledge of
   the plugin's payload encoding.

2. **Runtime plugin loading + dispatching** — `PluginManager`
   already loads `.so`/`.dylib`/`.dll`; the runtime currently uses
   an in-process `MockInputAdapter` (`crates/bee-runtime/src/test_utils.rs`).
   The runtime now needs to dispatch to loaded plugin Adapters and
   Handlers so a Pipeline referencing `binance.subscribe(...)`
   actually invokes the binance plugin's adapter at run time.

3. **2 SQL pipelines** — `examples/quant_btc_macd.sql` and
   `examples/quant_btc_sentiment.sql`, exercising different
   Handler plugins (technical-analysis only vs technical + ML).

4. **`scripts/demo-quant-prod.sh`** — a one-click end-to-end
   script: build, start 3-node cluster, load the 6 production
   plugins, register Datasources, deploy both SQL pipelines, verify
   Producer sharing (one Producer for BTC, both Pipelines
   subscribe), assert all 11 ADRs' Consequences, teardown.

Out of scope (deferred to S34–S39 + S40–S41 in subsequent sessions):
- Replacing the 5 mock plugins with production-grade real-external-system
  implementations (Binance WS, NewsAPI, InfluxDB v2, MongoDB,
  yata/ta-lib, tract + FinBERT).
- `examples/performance/` Fibonacci + prime sieve + multi-stream
  analytics (S41).
- HITL seed-user walkthrough (S33 close-out, after S40).

## Decisions (locked in via brainstorming)

- **FFI event wire format**: `bincode` (decision 1). Events are
  serialized to bytes on one side, deserialized on the other.
- **Plugin invocation surface**: per-adapter function pointer
  tables (decision 2). `PluginHandle` carries
  `input_adapters: HashMap<String, *const InputAdapterVtable>`,
  `output_adapters: HashMap<String, *const OutputAdapterVtable>`,
  `handlers: HashMap<String, *const HandlerVtable>`.
- **2 SQL pipelines**: two different strategies (decision 3).
  - `examples/quant_btc_macd.sql` — BTC K-line + MACD/EMA
    (technical only) → InfluxDB.
  - `examples/quant_btc_sentiment.sql` — BTC K-line + news
    sentiment + decision tree (with FinBERT) → InfluxDB.

## Architecture

### §1. Event wire format

**`crates/bee-plugin-sdk/src/event.rs`** (new):

```rust
/// Bincode-serialized `Event` (from `bee-adapter`). The `len` is
/// the encoded byte length; the `ptr` is non-null and points to
/// `len` valid bytes. The producer allocates (via `Vec` → leak or
/// `Box::into_raw`); the consumer reads + drops via the same
/// allocator. A function pointer `event_drop(ptr, len)` is
/// provided in the vtable so the consumer can free with the
/// producer's allocator.
pub struct EventBytes {
    pub ptr: *const u8,
    pub len: usize,
}
```

`Event` (in `bee-adapter`) is already `#[derive(Serialize, Deserialize)]`-able — we add the derive. The wire format is `bincode::serialize(&event)` on the producer side and `bincode::deserialize(&bytes)` on the consumer side.

The `Plugin` SDK exposes a single encode/decode helper:

```rust
pub fn encode_event(event: &Event) -> Vec<u8>;
pub fn decode_event(bytes: &[u8]) -> Result<Event, bincode::Error>;
```

All plugins use these. The host uses `decode_event` on every event it receives across the FFI.

### §2. Plugin invocation surface (vtables)

**`crates/bee-plugin-sdk/src/vtable.rs`** (new):

```rust
/// Function pointer table for an `InputAdapter` instance.
/// All function pointers take a `ctx: *mut c_void` (the adapter's
/// per-instance state, allocated by `open`) as the first arg.
#[repr(C)]
pub struct InputAdapterVtable {
    /// Open the adapter with a config (bincode-encoded plugin-
    /// specific config blob). Returns a `*mut c_void` ctx for
    /// subsequent calls, or null on error (with `err_out` filled).
    pub open: unsafe extern "C" fn(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes, // optional error message
    ) -> *mut c_void,

    /// Pull the next event. Returns 1 if an event was produced
    /// (written to `out`), 0 for end-of-stream, -1 on error.
    /// The producer owns the bytes; the consumer must call
    /// `drop_bytes` (or the same function on the vtable) to free.
    pub next: unsafe extern "C" fn(
        ctx: *mut c_void,
        out: *mut EventBytes,
    ) -> i32,

    /// Close the adapter; free the ctx. Returns 0 on success.
    pub close: unsafe extern "C" fn(ctx: *mut c_void) -> i32,
}

#[repr(C)]
pub struct OutputAdapterVtable {
    pub open: unsafe extern "C" fn(
        config_ptr: *const u8,
        config_len: usize,
        err_out: *mut EventBytes,
    ) -> *mut c_void,

    pub emit: unsafe extern "C" fn(
        ctx: *mut c_void,
        event_ptr: *const u8,
        event_len: usize,
    ) -> i32,

    pub close: unsafe extern "C" fn(ctx: *mut c_void) -> i32,
}

#[repr(C)]
pub struct HandlerVtable {
    /// Compute the result of `handler(state, event) -> (new_state,
    /// result)`. The state is bincode-encoded. Returns 0 on
    /// success; -1 on error (with `err_out` filled). The output
    /// `new_state` and `result` (both bincode-encoded) are
    /// written to the provided `*mut EventBytes`.
    pub handle: unsafe extern "C" fn(
        state_ptr: *const u8,
        state_len: usize,
        event_ptr: *const u8,
        event_len: usize,
        new_state_out: *mut EventBytes,
        result_out: *mut EventBytes,
        err_out: *mut EventBytes,
    ) -> i32,

    /// Initialize a fresh state blob (e.g. for a new Subscriber's
    /// first event). Returns the bincode-encoded default state in
    /// `out`.
    pub init_state: unsafe extern "C" fn(out: *mut EventBytes) -> i32,
}
```

**`PluginHandle` extension** (in `crates/bee-plugin-sdk/src/lib.rs`):

```rust
pub struct PluginHandle {
    pub manifest: PluginManifest,
    pub inner: Arc<dyn std::any::Any + Send + Sync + 'static>,

    // S33 deferred: per-adapter vtable registries. Populated by the
    // plugin in `init()` and frozen for the plugin's lifetime.
    pub input_adapters:
        std::collections::HashMap<String, *const InputAdapterVtable>,
    pub output_adapters:
        std::collections::HashMap<String, *const OutputAdapterVtable>,
    pub handlers:
        std::collections::HashMap<String, *const HandlerVtable>,
}
```

The plugin's `init()` constructs these maps from its concrete
adapters + handlers, then returns the `PluginHandle`. The host
stores the handle and never dereferences the raw pointers outside
of calling the vtable functions.

**BeeHostV1 extension** (in `crates/bee-plugin-sdk/src/lib.rs`): the existing function pointer table grows three more `register_*` functions so the plugin can register vtables alongside descriptors. (The current `register_adapter` only registers the descriptor, not the vtable.)

```rust
#[repr(C)]
pub struct BeeHostV1 {
    pub ctx: *mut std::ffi::c_void,
    pub register_adapter: Option<...>,  // existing: descriptor only
    // S33 deferred additions:
    pub register_input_adapter_vtable: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            name: *const std::ffi::c_char,
            vtable: *const InputAdapterVtable,
        )>,
    >,
    pub register_output_adapter_vtable: Option<...>,
    pub register_handler_vtable: Option<...>,
}
```

### §3. Runtime plugin loading + dispatching

**`crates/bee-registry/src/loader.rs`** (already has `load_library`): add a directory-walker:

```rust
/// Load every `.so`/`.dylib`/`.dll` in `dir` (non-recursive) and
/// register it. Returns the loaded `PluginId`s in load order.
pub fn load_directory(
    pm: &mut PluginManager,
    dir: &Path,
) -> Result<Vec<PluginId>, PluginError>;
```

**`crates/bee-runtime/src/plugin_adapter.rs`** (new): a "plugin-backed adapter" the runtime can use to satisfy a `Dag::InputAdapterKind::Plugin(name)` reference.

```rust
/// An InputAdapter that delegates to a loaded plugin's
/// InputAdapterVtable. Implements the existing
/// `InputAdapter` trait (from `bee-adapter`) by calling through
/// the vtable.
pub struct PluginInputAdapter {
    vtable: *const InputAdapterVtable,
    ctx: *mut c_void, // from vtable.open()
    closed: bool,
}

impl InputAdapter for PluginInputAdapter {
    type Config = Vec<u8>; // raw config blob (bincode-encoded by plugin)

    async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let vtable = lookup_vtable(&config.name)?;
        let ctx = unsafe { (vtable.open)(config.ptr, config.len, ...) };
        if ctx.is_null() { return Err(...); }
        Ok(Self { vtable, ctx, closed: false })
    }

    async fn next(&mut self) -> AdapterResult<Option<Event>> {
        let mut out = EventBytes { ptr: std::ptr::null(), len: 0 };
        let rc = unsafe { (self.vtable.next)(self.ctx, &mut out) };
        match rc {
            1 => {
                let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
                let event = decode_event(bytes)?;
                // copy out of the plugin's buffer
                Ok(Some(event))
            }
            0 => Ok(None),
            _ => Err(AdapterError::Next("plugin returned -1".into())),
        }
    }

    async fn close(self) -> AdapterResult<()> {
        let rc = unsafe { (self.vtable.close)(self.ctx) };
        if rc != 0 { return Err(AdapterError::Close("plugin returned non-zero".into())); }
        Ok(())
    }
}
```

(Symmetric `PluginOutputAdapter` and `PluginHandler` for Output and Handler.)

**`crates/bee-runtime/src/lib.rs`**: a new `PluginAdapterRegistry` (process-global, behind a `OnceLock`) that maps `(plugin_name, adapter_name)` → `*const InputAdapterVtable` etc. The runtime consults this when starting a Phase:

```rust
fn resolve_input_adapter(name: &str) -> Result<Box<dyn DynInputAdapter>, AdapterError> {
    if name == "mock" { return Ok(Box::new(MockInputAdapter::open(...))); }
    if let Some(vtable) = PLUGIN_ADAPTER_REGISTRY.lookup_input(name) {
        return Ok(Box::new(PluginInputAdapter::open(vtable, ...)));
    }
    Err(AdapterError::Open(format!("unknown input adapter: {name}")))
}
```

**`crates/bee-control/src/deployer.rs`**: `Deployer::deploy` consults the registry at deploy time to verify the Pipeline's adapter references resolve. The verification is a pre-flight check; the actual dispatch happens in the runtime.

### §4. 2 SQL pipelines

**`examples/quant_btc_macd.sql`** (new, ~30 lines):

```sql
use binance;
use influxdb;
SELECT
  b.timestamp,
  b.symbol,
  b.price,
  ta.macd(b.price, 12, 26, 9) AS macd,
  ta.ema(b.price, 20) AS ema20
FROM binance.subscribe('BTC/USDT', '5min') AS b
EMIT INTO influxdb.measurement('btc_macd');
```

**`examples/quant_btc_sentiment.sql`** (new, ~30 lines):

```sql
use binance;
use google_news;
use influxdb;
SELECT
  b.timestamp,
  b.symbol,
  b.price,
  news.sentiment('finbert', headline) AS score,
  tree.decide(b.price, score) AS action
FROM binance.subscribe('BTC/USDT', '5min') AS b
  ASOF JOIN google_news.search('bitcoin') AS news
EMIT INTO influxdb.measurement('btc_sentiment');
```

Both files compile against the same Datasource set (binance, google_news, influxdb); they differ in which Handler plugin they pull in (technical only vs ML).

### §5. `scripts/demo-quant-prod.sh`

A bash script (~100 lines) that:
1. `cargo build --workspace` (catches compile errors first)
2. `cargo build -p bee` to produce the `bee` binary
3. For each of 6 production plugin crates: `cargo build -p bee-plugin-X` to produce the `.so`/`.dylib`/`.dll`
4. Start a 3-node cluster: 3 background `bee` processes on ports 7701/7702/7703, redirected to logs
5. Wait for leader election (poll until `/cluster/leader` responds)
6. `bee plugin register <path>` for each of the 6 plugins
7. `bee datasource create` for binance, google_news, influxdb, mongodb
8. `bee run examples/quant_btc_macd.sql` → asserts `job_mode == Producer` (BTC K-line is the unique stream)
9. `bee run examples/quant_btc_sentiment.sql` → asserts the BTC stream still has exactly 1 Producer (S17 Producer sharing), this Job is a Subscriber
10. `bee jobs list` → asserts the table has a MODE column and the right cells
11. Wait a configurable period for the strategies to emit
12. `bee diagnostics <TaskId>` for the InfluxDB sink → asserts the measurement has rows
13. Teardown: `kill` the 3 background processes
14. Print a summary table: each step pass/fail; pass count vs total

The script is idempotent: re-running cleans up any prior run. It uses `set -euo pipefail` so any failure aborts.

The script does NOT verify the plugins' external connections (Binance WS, NewsAPI, InfluxDB) — those are S34–S36's job to wire up. For S33 deferred, the plugins stay as the 5 mock crates (sine-wave prices, log-file sinks). The demo verifies the S33 architecture: FFI dispatching, runtime loading, SQL compilation, deployer wiring, jobs-list rendering, diagnostics. The 11 ADRs' Consequences are verified against the mock outputs.

## Acceptance criteria

- [ ] `cargo build --workspace` clean (0 warnings beyond the 2 pre-existing)
- [ ] `cargo test --workspace` all green
- [ ] `Event` is `Serialize`/`Deserialize`; `encode_event`/`decode_event` roundtrip
- [ ] `InputAdapterVtable`, `OutputAdapterVtable`, `HandlerVtable` exist with `#[repr(C)]`
- [ ] `PluginHandle` carries the 3 vtable maps; the mock plugins populate them
- [ ] `BeeHostV1` gains `register_input_adapter_vtable`, `register_output_adapter_vtable`, `register_handler_vtable`
- [ ] `load_directory(pm, dir)` walks a directory and registers all plugins
- [ ] `PluginInputAdapter::next` round-trips a bincode event through a vtable (unit test with an in-process fake vtable)
- [ ] `Deployer::deploy` resolves adapter names via the registry
- [ ] `examples/quant_btc_macd.sql` compiles + deploys
- [ ] `examples/quant_btc_sentiment.sql` compiles + deploys
- [ ] `scripts/demo-quant-prod.sh` runs end-to-end on a single host; `bee jobs list` shows Producer (Job 1) + Subscriber (Job 2) for the shared BTC stream; InfluxDB measurement file has rows after the wait period
- [ ] S33 working tree (everything committed at `22a9e39`) untouched
- [ ] S17 + refactor commits (`b680d3b`) untouched
- [ ] All work lands in a single `S33-deferred: ...` commit (per the S17 consolidation precedent)

## Risks

1. **FFI memory safety**: `*const u8` + `len` is the simplest C ABI but
   leaves lifetime management to the producer/consumer pair. The
   vtable design includes explicit `drop_bytes` semantics (via the
   `close` path) and the unit tests exercise the round-trip. Plugin
   authors must follow the contract; a bad plugin is a process
   crash, not silent corruption. **Mitigation**: a debug-only
   `validate_bytes` flag in the vtable that the host can spot-check.

2. **Mock plugins don't have real FFI yet**: the 5 mock crates
   currently implement the `Plugin` trait in-process (no FFI
   loading). They need a `cdylib_plugin!` shim that constructs the
   vtables from the in-process impls. **Mitigation**: the shim is
   mechanical; the mock crate itself stays single-source.

3. **Demo script timing flakiness**: cluster startup, leader
   election, plugin registration, deploy, and emit all have
   wall-clock variability. The script polls with timeouts at every
   step. **Mitigation**: configurable `BEE_DEMO_TIMEOUT_S` env
   var; default 30s for startup steps, 10s for data steps.

4. **Mock plugins can't satisfy real external connections**: the
   demo uses mock plugins, not production plugins. S34–S39 will
   replace them. The demo will be re-run against the production
   plugins as part of S40. **Mitigation**: the demo's success
   criteria are architecture-level (FFI dispatch, registry
   resolution, jobs-list rendering), not data-level (specific
   price values, specific sentiment scores). S40's demo will
   add the data-level criteria.

5. **The bee binary's plugin loading command is unimplemented in
   the CLI**: the `bee plugin register` subcommand may need a
   new surface. **Mitigation**: check `bee/src/main.rs:707` (the
   `bee plugin list` exists) and add a `register` subcommand; the
   script falls back to a direct HTTP call if the CLI is missing.

## Implementation order (TDD)

1. §1 event wire format: add Serialize/Deserialize to `Event` +
   `encode_event`/`decode_event` in `bee-plugin-sdk` + 4 unit tests
   (roundtrip, empty payload, large payload, partial slice).
2. §2 vtable types: define `InputAdapterVtable`,
   `OutputAdapterVtable`, `HandlerVtable` in
   `crates/bee-plugin-sdk/src/vtable.rs` + 1 compile-time FFI-safety
   test (`size_of` checks).
3. §2 mock plugin shim: extend each of the 5 mock plugin crates
   to construct vtables from their in-process impls + a unit test
   per crate (e.g., `BinanceMockInput::next` round-trips a bincode
   event through the vtable).
4. §2 `PluginHandle` extension: add the 3 vtable maps + update
   existing unit tests + 1 new test (an in-process plugin
   populates the maps in `init()`).
5. §2 `BeeHostV1` extension: add the 3 `register_*_vtable` function
   pointer slots + ABI compile-time check.
6. §3 `load_directory` + `PluginAdapterRegistry` +
   `PluginInputAdapter` + `PluginOutputAdapter` + `PluginHandler`
   + unit tests (one round-trip per type).
7. §3 runtime dispatch: `resolve_input_adapter` +
   `resolve_output_adapter` + `resolve_handler` +
   pre-flight check in `Deployer::deploy` (refuse to deploy a
   Pipeline referencing an unknown adapter).
8. §4 2 SQL pipelines: write the 2 files; verify they compile
   with `bee run --check` (or whatever the dry-run command is).
9. §5 `scripts/demo-quant-prod.sh`: write the script; run it on
   a single host; iterate until green.
10. `cargo build --workspace` + `cargo test --workspace` clean.
11. Single `S33-deferred: ...` commit.

## Open questions (resolved during design)

- ❓ Event format? **Resolved: bincode (decision 1).**
- ❓ Invocation surface? **Resolved: per-adapter vtables (decision 2).**
- ❓ 2 SQL scenarios? **Resolved: two different strategies (decision 3).**
- ❓ Single commit? **Resolved: yes, per the S17 precedent (`b680d3b`).**
- ❓ Mock vs production plugins? **Resolved: mock for S33 deferred;
  production in S34–S39.**
- ❓ HITL review? **Resolved: deferred to S33 close-out (user's
  manual step after S40).**
