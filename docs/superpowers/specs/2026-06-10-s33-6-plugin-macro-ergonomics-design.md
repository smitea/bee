# S33.6 — Plugin Macro Ergonomics

**Date:** 2026-06-10
**Type:** AFK
**Blocked by:** S33.5.2 (Datasource validation)
**Status:** Approved (2026-06-10)

## Why this story exists

A third-party plugin author writing a `cdylib` plugin for Bee today has to write ~150 lines of boilerplate per adapter:

1. **For each `InputAdapter`**: 3 `unsafe extern "C" fn` (`open` / `next` / `close`) plus a `static FOO_VTABLE: InputAdapterVtable = ...` constant. Each `extern "C" fn` is a hand-written byte-shuffling wrapper that deserializes config bytes, calls the underlying async impl, serializes Event bytes, and writes the result into an out-pointer. 4 chunks of code × 7 lines each = ~30 lines per InputAdapter just for the FFI.
2. **For each `OutputAdapter`**: 3 `unsafe extern "C" fn` (`open` / `emit` / `close`) + vtable constant. ~30 lines.
3. **For each `Handler`**: 2 `unsafe extern "C" fn` (`handle` / `init_state`) + vtable constant. ~25 lines.
4. **`PluginHandle::init()` plumbing**: hand-fill 3 `HashMap<String, *const Vtable>` (input_adapters, output_adapters, handlers), one insert per adapter.
5. **`PluginManifest` construction**: hand-fill the `adapters: Vec<AdapterDescriptor>` and `handlers: Vec<HandlerDescriptor>`.

A typical "binance" plugin with 1 InputAdapter + 1 OutputAdapter + 1 Handler is **~150 lines of FFI/glue before any business logic**. S33.6 introduces a `#[bee_adapter]` proc-macro that turns each method into the vtable + the registration glue, so the plugin author writes **only the async Rust trait impl** (~30 lines total).

The same `MockInputAdapter` in `crates/bee-runtime/src/test_utils.rs` (used to verify the runtime path) is already a clean native `async fn next()` impl. S33.6 brings the cdylib plugin author experience to parity with the in-process test-fixture experience: write the async fn, decorate, done.

## Scope

### In scope (3 deliverables)

1. **New crate `crates/bee-plugin-macro/`** (proc-macro crate):
   - `#[bee_adapter(input, name = "subscribe")]` — applied to the `async fn open(config) -> AdapterResult<Self>` method of a `impl` block. The macro generates:
     - A `*const InputAdapterVtable` static named `{STRUCT_NAME}_VTABLE` (or a user-provided name).
     - The 3 `unsafe extern "C" fn` (`open` / `next` / `close`) that bridge the FFI boundary to the underlying async impl.
     - Helper functions that serialize/deserialize the bincode event/state blobs.
   - `#[bee_adapter(output, name = "ohlcv")]` — same shape but for `OutputAdapter` (`open` / `emit` / `close`).
   - `#[bee_adapter(handler, name = "fib")]` — same shape but for `Handler` (`handle` / `init_state`).
   - `#[bee_method(name = "next")]` — applied to the body methods (`async fn next(&mut self) -> AdapterResult<Option<Event>>`, etc.). Lets the author name the FFI method differently from the Rust method name (default: same name).
   - Compile-time signature checks via `syn`:
     - `#[bee_adapter(input)]` on `open`: must be `async fn open(config: $CFG) -> AdapterResult<Self>`, where `$CFG: DeserializeOwned`.
     - `#[bee_method(name = "next")]` on `next`: must be `async fn next(&mut self) -> AdapterResult<Option<Event>>`.
     - etc. (one check per FFI method).
   - Friendly compile errors via `syn::Error::to_compile_error()` with file:line:col.

2. **Sub-macro `register_vtable!`** (in `crates/bee-plugin-sdk/src/macros.rs`, alongside `cdylib_plugin!`):
   - A `macro_rules!` (NOT a proc-macro) that takes a sequence of `(kind, name, vtable_static)` tuples and builds the `PluginHandle` HashMaps.
   - The plugin author writes:
     ```rust
     register_vtable! {
         PluginHandle,
         input  "subscribe" => SUBSCRIBE_INPUT_VTABLE,
         output "ohlcv"     => OHLCV_OUTPUT_VTABLE,
         handler "fib"      => FIB_HANDLER_VTABLE,
     }
     ```
   - The macro expands to the 3 HashMap inserts.

3. **Example refactor + 1 new test fixture**:
   - Refactor `crates/bee-plugin-sdk/src/lib.rs::tests::MockBinancePlugin` (in-process test fixture) to use `#[bee_adapter]` for its 1 InputAdapter. Proves the macro works end-to-end.
   - New `crates/bee-plugin-macro/tests/` integration tests (5 tests):
     - `macro_expands_input_adapter`: a sample `impl` block with `#[bee_adapter(input)]` + `#[bee_method]` compiles and produces a valid `*const InputAdapterVtable`. Calling the vtable's `open` then `next` then `close` returns the expected sequence.
     - `macro_expands_output_adapter`: same for output.
     - `macro_expands_handler`: same for handler.
     - `macro_registration_round_trip`: use the generated vtable to build a `PluginHandle`, register with `PluginManager`, and verify `pm.resolve(adapter_name, &version_spec)` returns `Some`.
     - `trybuild`-style compile-fail test: `#[bee_adapter(input)]` on a sync fn returns a compile error (snapshot test on the error message).

### Out of scope (deferred)

- Proc-macro for `PluginManifest` construction (`#[plugin(name = "binance", version = "1.0.0")]` on the factory struct). Plugin authors continue to hand-write the manifest.
- Auto-`cdylib_plugin!` integration: the macro does NOT generate the `bee_plugin_init` / `bee_plugin_drop` symbols. Plugin authors continue to use `cdylib_plugin!(MyFactory)` separately.
- Cross-crate type checking: the macro does NOT verify that the plugin's `Config` type matches the host's expected `Config` type. The bincode wire format is the only contract.
- IDE-level error messages (rust-analyzer integration beyond what `syn` gives by default).
- Non-Rust plugins (C/C++/Python/Go). These continue to write vtables by hand (the FFI is C ABI by design).
- Async runtime choice: the macro hard-codes `tokio::runtime::Handle::current()` (or `try_current()` with a clear error if no runtime). Plugin authors can override with a different runtime later if needed.

## Design

### Macro surface

```rust
// In a plugin's lib.rs:
use bee_plugin_sdk::*;
use bee_adapter::{Event, AdapterError, AdapterResult};

/// The plugin's "Binance Subscribe" InputAdapter.
pub struct BinanceSubscribeAdapter {
    topic: String,
    received: u32,
}

/// Manually-written impl block. The macro DECORATES the
/// methods; it does NOT touch the struct definition.
impl BinanceSubscribeAdapter {
    /// The InputAdapter factory. The macro turns this
    /// into an `extern "C" fn open` that deserializes
    /// the config bytes, calls this async fn, and
    /// returns a `*mut c_void` ctx.
    #[bee_adapter(input, name = "subscribe")]
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        // Parse the config; in real plugins, this is
        // a bincode-decoded plugin-specific struct.
        let cfg: SubscribeConfig = bincode::deserialize(&config)
            .map_err(|e| AdapterError::Open(format!("bad config: {e}")))?;
        Ok(Self { topic: cfg.topic, received: 0 })
    }

    /// The InputAdapter poll method. The macro turns
    /// this into `extern "C" fn next` that wraps the
    /// async call in a runtime block_on + bincode
    /// round-trip of the Event. The FFI slot name
    /// is fixed (`next`) by the vtable struct; the
    /// `#[bee_method]` attribute just marks this as
    /// "this is the next slot" (vs. the `close`
    /// slot, which is also `async`).
    #[bee_method]
    pub async fn next_event(&mut self) -> AdapterResult<Option<Event>> {
        if self.received >= 100 {
            return Ok(None);
        }
        self.received += 1;
        Ok(Some(Event {
            timestamp: Event::now_timestamp(),
            sequence: self.received as u64,
            payload: format!("tick#{} on {}", self.received, self.topic).into_bytes(),
        }))
    }

    /// The InputAdapter close method.
    #[bee_method]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}
```

`#[bee_method(slot = "next")]` is an explicit slot marker: the macro requires the slot name to match the FFI struct field (`open` / `next` / `close` for input; `open` / `emit` / `close` for output; `handle` / `init_state` for handler). The plugin author can use any method name on the Rust side; the `slot` arg is what binds the FFI wire.

**Generated code** (per `#[bee_adapter(input, name = "subscribe")]` on `open` + 2 `#[bee_method]` on `next_event` and `close`):

```rust
// ==== Generated by #[bee_adapter] (do not edit by hand) ====

// The per-instance state stored in the `*mut c_void` ctx.
// Holds a `Box<dyn Any + Send>` so the same ctx type
// works for input / output / handler.
struct BinanceSubscribeAdapterCtx {
    // For input/output: an in-flight `open` future, OR
    // the constructed `Self`, OR nothing. Macro emits
    // a `tokio::sync::Mutex<Option<...>>` here.
    inner: tokio::sync::Mutex<Option<BinanceSubscribeAdapter>>,
}

unsafe extern "C" fn binance_subscribe_adapter_open(
    config_ptr: *const u8,
    config_len: usize,
    _err_out: *mut EventBytes,
) -> *mut std::ffi::c_void {
    let config = std::slice::from_raw_parts(config_ptr, config_len).to_vec();
    let fut = async move {
        <BinanceSubscribeAdapter>::open(config).await
    };
    let adapter = match tokio::runtime::Handle::try_current() {
        Ok(h) => tokio::task::block_in_place(|| h.block_on(fut)),
        Err(_) => futures::executor::block_on(fut),
    };
    let adapter = match adapter {
        Ok(a) => a,
        Err(e) => return std::ptr::null_mut(),
    };
    let ctx = Box::new(BinanceSubscribeAdapterCtx {
        inner: tokio::sync::Mutex::new(Some(adapter)),
    });
    Box::into_raw(ctx) as *mut std::ffi::c_void
}

unsafe extern "C" fn binance_subscribe_adapter_next(
    ctx: *mut std::ffi::c_void,
    out: *mut EventBytes,
) -> i32 {
    let ctx = &*(ctx as *const BinanceSubscribeAdapterCtx);
    let mut guard = match ctx.inner.try_lock() {
        Ok(g) => g,
        Err(_) => return -1, // another next() in flight; reject
    };
    let adapter = match guard.as_mut() {
        Some(a) => a,
        None => return 0, // already closed
    };
    let fut = adapter.next_event();
    let result = match tokio::runtime::Handle::try_current() {
        Ok(h) => tokio::task::block_in_place(|| h.block_on(fut)),
        Err(_) => futures::executor::block_on(fut),
    };
    match result {
        Ok(Some(event)) => {
            let bytes = match bincode::serialize(&event) {
                Ok(b) => b,
                Err(_) => return -1,
            };
            let len = bytes.len();
            let ptr = bytes.as_ptr();
            std::mem::forget(bytes);
            *out = EventBytes { ptr, len };
            1
        }
        Ok(None) => {
            *out = EventBytes::EMPTY;
            0
        }
        Err(_) => -1,
    }
}

unsafe extern "C" fn binance_subscribe_adapter_close(
    ctx: *mut std::ffi::c_void,
) -> i32 {
    if ctx.is_null() { return 0; }
    let ctx = Box::from_raw(ctx as *mut BinanceSubscribeAdapterCtx);
    let mut guard = match ctx.inner.try_lock() {
        Ok(mut g) => g.take(),
        Err(_) => return -1,
    };
    match guard {
        Some(adapter) => {
            let fut = adapter.close();
            match tokio::runtime::Handle::try_current() {
                Ok(h) => tokio::task::block_in_place(|| h.block_on(fut)),
                Err(_) => futures::executor::block_on(fut),
            };
        }
        None => {}
    }
    0
}

pub static BINANCE_SUBSCRIBE_ADAPTER_VTABLE: InputAdapterVtable = InputAdapterVtable {
    open: binance_subscribe_adapter_open,
    next: binance_subscribe_adapter_next,
    close: binance_subscribe_adapter_close,
};
```

(Generated code above is approximate — the actual proc-macro will factor out the `block_on` boilerplate into a helper trait, but the shape is correct.)

### Why `block_in_place` + `block_on`?

The host calls the vtable's `next` from a worker thread. If the plugin's `next` is `async fn`, the worker thread must wait for it. Three options:

- **`tokio::task::block_in_place(|| h.block_on(fut))`** (recommended): if the host is a multi-thread tokio runtime, the worker thread blocks (still inside the runtime) and another runtime worker picks up the await. This is what `tokio` docs recommend for sync→async bridges.
- **`h.block_on(fut)`** (no `block_in_place`): would only work for the single-threaded runtime flavor; for multi-thread it panics.
- **Spawn a task + return immediately + notify via callback** (the "proper" async-over-FFI): would require a new vtable shape (callback-based). S33.6's MVP uses `block_in_place` for simplicity; a follow-up story can add the callback-based vtable.

The macro detects the runtime flavor at the call site: if no tokio runtime is active, it falls back to `futures::executor::block_on` (a no-op runtime for plugin unit tests). This makes the macro testable in isolation.

### `register_vtable!` sub-macro

```rust
// In bee-plugin-sdk/src/macros.rs:
#[macro_export]
macro_rules! register_vtable {
    (
        $handle:ident,
        $( $kind:ident $name:literal => $vtable:expr ),* $(,)?
    ) => {
        $handle.input_adapters.insert(
            $name.into(),
            $vtable,
        );
        // ... etc per kind
    };
}
```

The macro is straight macro_rules! (no proc-macro needed). The plugin author's `Factory::init()` looks like:

```rust
impl Factory for BinanceFactory {
    fn init() -> PluginResult<PluginHandle> {
        let manifest = Self::manifest();
        let mut input_adapters = std::collections::HashMap::new();
        let mut output_adapters = std::collections::HashMap::new();
        let mut handlers = std::collections::HashMap::new();
        input_adapters.insert("subscribe".into(), &BINANCE_SUBSCRIBE_ADAPTER_VTABLE);
        // ... 2 more lines for the others
        Ok(PluginHandle {
            manifest,
            inner: Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}
```

After the sub-macro:

```rust
impl Factory for BinanceFactory {
    fn init() -> PluginResult<PluginHandle> {
        let manifest = Self::manifest();
        let mut input_adapters = std::collections::HashMap::new();
        let mut output_adapters = std::collections::HashMap::new();
        let mut handlers = std::collections::HashMap::new();
        register_vtable! {
            input_adapters, output_adapters, handlers;
            input  "subscribe" => &BINANCE_SUBSCRIBE_ADAPTER_VTABLE,
            output "ohlcv"     => &BINANCE_OHLCV_OUTPUT_VTABLE,
            handler "fib"      => &BINANCE_FIB_HANDLER_VTABLE,
        }
        Ok(PluginHandle {
            manifest,
            inner: Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}
```

The `register_vtable!` macro takes the 3 HashMap bindings (passed by the plugin author) + a sequence of `(kind, name, vtable_static)` tuples, and emits the 3 HashMap inserts. No new constructors on `PluginHandle` are required.

### Compile-time signature checks

The proc-macro uses `syn` to inspect the method's signature and emits a `syn::Error` (rendered as a compile error) if the shape doesn't match. The shapes are:

| Attribute | Required method signature |
|-----------|---------------------------|
| `#[bee_adapter(input, name = "X")]` on `fn open` | `async fn open(config: $CFG) -> AdapterResult<Self>` where `$CFG: DeserializeOwned + Send + 'static` |
| `#[bee_method(name = "next")]` on `fn next` | `async fn next(&mut self) -> AdapterResult<Option<Event>>` |
| `#[bee_method(name = "close")]` on `fn close` | `async fn close(self) -> AdapterResult<()>` |
| `#[bee_adapter(output, name = "X")]` on `fn open` | same as input's `open` |
| `#[bee_method(name = "emit")]` on `fn emit` | `async fn emit(&mut self, event: Event) -> AdapterResult<()>` |
| `#[bee_method(name = "close")]` on `fn close` | same as input's `close` |
| `#[bee_adapter(handler, name = "X")]` on `fn handle` | `async fn handle(&mut self, state: $S, event: $E) -> AdapterResult<($S, $R)>` (no `init_state` requirement — the macro generates a default `init_state` that returns an empty `Vec<u8>`) |
| `#[bee_method(name = "init_state")]` on `fn init_state` (optional) | `async fn init_state() -> AdapterResult<$S>` where `$S: Default + Serialize + Send + 'static` |

The macro supports ONE method per FFI slot (so `open` is always the factory, `close` is always the destructor, etc.). Plugin authors who want to add custom helpers (e.g., a `pause` method) can add plain async methods to the impl block without any `#[bee_*]` attribute.

### Where the macro lives

New crate: `crates/bee-plugin-macro/`. Dependencies: `proc-macro2`, `quote`, `syn`. Workspace dep update: add `bee-plugin-macro` to `[workspace.dependencies]`. The `cdylib_plugin!` macro stays where it is (in `bee-plugin-sdk`).

`bee-plugin-macro` exports one proc-macro: `bee_adapter`. The `register_vtable!` sub-macro stays in `bee-plugin-sdk` (it's a `macro_rules!`, not a proc-macro).

### Test plan (5 tests in `crates/bee-plugin-macro/tests/`)

#### 1. `macro_expands_input_adapter`

```rust
use bee_plugin_macro::bee_adapter;
use bee_plugin_sdk::vtable::InputAdapterVtable;
use bee_adapter::{Event, AdapterError, AdapterResult};
use std::sync::Arc;

pub struct MockInput {
    count: u32,
    emitted: u32,
}

impl MockInput {
    #[bee_adapter(input, name = "mock")]
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let c: u32 = bincode::deserialize(&config).unwrap_or(3);
        Ok(Self { count: c, emitted: 0 })
    }

    #[bee_method(name = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        if self.emitted >= self.count { return Ok(None); }
        self.emitted += 1;
        Ok(Some(Event {
            timestamp: 0,
            sequence: self.emitted as u64,
            payload: self.emitted.to_string().into_bytes(),
        }))
    }

    #[bee_method(name = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

// The macro should generate a `pub static MOCK_INPUT_VTABLE: InputAdapterVtable`.
// The test calls open/next/close through the vtable and asserts the events.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_input_adapter_through_vtable() {
    let config = bincode::serialize(&3u32).unwrap();
    let ctx = unsafe {
        ((*MOCK_INPUT_VTABLE).open)(
            config.as_ptr(), config.len(), std::ptr::null_mut()
        )
    };
    assert!(!ctx.is_null());
    // ... call next 3 times, assert events
    // ... call close, assert rc == 0
}
```

#### 2. `macro_expands_output_adapter`

Same shape, OutputAdapter variant.

#### 3. `macro_expands_handler`

Same shape, Handler variant. Tests `init_state` + `handle` round-trip.

#### 4. `macro_registration_round_trip`

Build a `PluginHandle` using `register_vtable!` + the macro-generated vtable, register with `PluginManager`, assert `pm.resolve("binance", &VersionSpec::Latest)` returns `Some(PluginId)`.

#### 5. `trybuild` compile-fail test (snapshot)

A test source file that uses `#[bee_adapter(input)]` on a non-async fn. The expected stderr snapshot (e.g., "method `open` must be `async`") is committed to the repo. `trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs")` runs it.

### Refactor of existing test fixture

`crates/bee-plugin-sdk/src/lib.rs::tests::MockBinancePlugin` (in-process test plugin that registers with `PluginManager::register_plugin`) currently has hand-written `Plugin` trait impl. After S33.6:

- The `MockBinancePlugin::init()` uses the macro-generated vtable instead of returning an empty `PluginHandle { input_adapters: HashMap::new(), ... }`.
- The `bee-plugin-macro` crate is added as a `[dev-dependencies]` of `bee-plugin-sdk` (so the tests can use `#[bee_adapter]`).

This proves the macro works in the in-process test path (not just cdylib). The cdylib path is exercised by the integration tests in `bee-plugin-macro/tests/`.

### Edge cases

- **Plugin author uses no `#[bee_method(name = "next")]` for an input adapter**: the macro requires it; emits "method `next` is required for `#[bee_adapter(input)]`".
- **Plugin author has multiple `next` methods**: compile error.
- **Plugin author uses `#[bee_adapter(input)]` on a `fn open` that returns `Self` (not `AdapterResult<Self>`)**: compile error.
- **Plugin author's `close` takes `&mut self` instead of `self`**: compile error (the macro needs ownership to drop the `Box<Ctx>`).
- **No tokio runtime in the call site of the vtable**: `block_in_place` would panic; the macro falls back to `futures::executor::block_on` (which always works but spins up a new runtime per call — slow but correct). The MVP uses this fallback.
- **Multiple `#[bee_adapter(input)]` on the same impl block (one InputAdapter + one OutputAdapter)**: the macro generates 2 separate vtables + 2 separate ctx structs. The user names them via `name = "subscribe"` / `name = "emit"` etc.

### Sign-off matrix

| Item | Code-level (this story) | Production-level (1.x) |
|------|------------------------|------------------------|
| `#[bee_adapter(input/output/handler)]` proc-macro generates correct vtable | ✓ (5 tests) | N — third-party plugin author trial |
| `register_vtable!` sub-macro | ✓ (1 test) | N |
| In-process test fixture uses macro | ✓ (refactor + still passes) | N |
| Compile-fail snapshot tests | ✓ (trybuild) | N |
| `tokio::block_in_place` + `Handle::try_current` fallback | ✓ (works in tests) | N — actual plugin author measures |
| Macro error messages are friendly | ✓ (syn::Error to_compile_error) | N — author feedback |

## Related work

- S19: `cdylib_plugin!` macro (already exists; covers the `bee_plugin_init` / `bee_plugin_drop` FFI entry points).
- S20: `AbiVersion` check on plugin load.
- S29: `DatasourceRegistry` (in-process, not the plugin macro story).
- S33.5.2: `RegisterDatasource` validation (closes the gap where a user could register a Datasource that points at a non-existent adapter; after S33.6, the validation checks the macro-generated vtable's `name` matches the Datasource's `adapter` field).
- S41: `BeeHostV1` extension (KV get/put/cas function pointers) — orthogonal to the macro story; plugins can already use these via the `host: *mut BeeHostV1` arg to `bee_plugin_init`.
