# S33 · Mock plugin crates + libloading (spike foundation)

**Date**: 2026-06-07
**Status**: design — pending approval
**Owner**: S33

## Scope (this session)

1. Wire `libloading` in `bee-registry` so `PluginManager` can load
   `cdylib` plugin binaries from disk.
2. Add a `cdylib_plugin!` macro in `bee-plugin-sdk` so plugin
   authors can declare their FFI entry point with one line.
3. Build 5 mock plugin crates (separate workspace members) with
   real `InputAdapter` / `OutputAdapter` / `Handler` logic and
   unit tests.

## Out of scope (deferred)

- The runtime dispatching to plugin adapters (runtime still uses
  the in-process `MockInputAdapter`).
- 2 SQL pipelines + `scripts/demo-quant.sh`.
- README / product-design updates.
- HITL review (the seed-user walkthrough).
- Production wire format for adapter events across the FFI.

## Architecture

### `bee-plugin-sdk` — add FFI entry macro

```rust
// crates/bee-plugin-sdk/src/macros.rs
#[macro_export]
macro_rules! cdylib_plugin {
    ($factory:expr) => {
        #[no_mangle]
        pub extern "C" fn bee_plugin_init(
            _host: *mut bee_plugin_sdk::BeeHostV1,
        ) -> *mut bee_plugin_sdk::PluginHandle {
            match $factory.init() {
                Ok(h) => std::sync::Arc::into_raw(
                    std::sync::Arc::new(h),
                ) as *mut _,
                Err(_) => std::ptr::null_mut(),
            }
        }

        #[no_mangle]
        pub extern "C" fn bee_plugin_drop(
            handle: *mut bee_plugin_sdk::PluginHandle,
        ) {
            if !handle.is_null() {
                unsafe {
                    drop(std::sync::Arc::from_raw(handle));
                }
            }
        }
    };
}
```

The plugin crate provides a `$factory: impl PluginFactory` that
has the same shape as the in-process `Plugin` trait minus the
`plugin_content()` (the host computes that from the binary bytes).

### `bee-registry` — add `load_library`

```rust
// crates/bee-registry/src/loader.rs
pub struct LoadedPlugin {
    pub id: PluginId,
    pub handle: Arc<PluginHandle>,
    _lib: Library,  // keeps the .so alive
}

pub fn load_library<P: AsRef<Path>>(
    path: P,
) -> PluginResult<LoadedPlugin> {
    let path = path.as_ref();
    let content = std::fs::read(path)
        .map_err(|e| PluginError::Init(e.to_string()))?;
    let id = compute_plugin_id(&content);
    unsafe {
        let lib = Library::new(path)
            .map_err(|e| PluginError::Init(e.to_string()))?;
        let init: Symbol<
            unsafe extern "C" fn(*mut BeeHostV1) -> *mut PluginHandle,
        > = lib.get(b"bee_plugin_init")
            .map_err(|e| PluginError::Init(e.to_string()))?;
        let handle_ptr = init(std::ptr::null_mut());
        if handle_ptr.is_null() {
            return Err(PluginError::Init("null handle".into()));
        }
        let handle = Arc::from_raw(handle_ptr);
        Ok(LoadedPlugin { id, handle, _lib: lib })
    }
}
```

`PluginManager::register_library(path)` wraps it: ABI check,
idempotent, returns the `PluginId`.

### Plugin crate shape

Each of the 5 plugins is a workspace member under `plugins/`:

```
plugins/
├── bee-plugin-binance-mock/
│   ├── Cargo.toml         # crate-type = ["cdylib", "rlib"]
│   └── src/lib.rs
├── bee-plugin-google-news-mock/
├── bee-plugin-influxdb-mock/
├── bee-plugin-mongodb-mock/
└── bee-plugin-ta-lib-mock/
```

Each `Cargo.toml`:
```toml
[package]
name = "bee-plugin-binance-mock"
version = "0.1.0"
edition.workspace = true
license.workspace = true
publish = false

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
bee-plugin-sdk = { path = "../../crates/bee-plugin-sdk" }
bee-adapter = { path = "../../crates/bee-adapter" }
tokio = { workspace = true }
thiserror = "2"
```

Each `src/lib.rs` exports:
- A `BinanceMockFactory: PluginFactory` impl with the manifest
- A `make_binance_input(config) -> BinanceMockInput` in-process
  factory (used by tests + future runtime path)
- `cdylib_plugin!(BinanceMockFactory)` to generate the FFI entry
- A `BinanceMockInput: InputAdapter` impl with mock sine-wave
  price generation
- Unit tests for the adapter behavior

### The 5 plugins

| Crate | Role | Behavior |
| --- | --- | --- |
| `bee-plugin-binance-mock` | Input `binance_subscribe` | Sine-wave prices; configurable `symbol`, `interval`; emits 1 event/sec |
| `bee-plugin-google-news-mock` | Input `google_news_search` | Synthetic news articles; query string; emits 1 event/min |
| `bee-plugin-influxdb-mock` | Output `influxdb_emit` | Append to `/tmp/bee_demo_influxdb.log`; `database`, `measurement` |
| `bee-plugin-mongodb-mock` | Output `mongodb_emit` | Append to `/tmp/bee_demo_mongodb.jsonl`; `database`, `collection` |
| `bee-plugin-ta-lib-mock` | Handlers `MACD`, `EMA`, `decision_tree`, `sentiment_analyzer` | Pure-compute UDFs over in-crate state; deterministic outputs |

Each plugin:
- Has its own `Plugin Manifest` (name / version / `abi_version`)
- Builds independently as `cdylib` (proves FFI surface compiles)
- Builds independently as `rlib` (tests use the in-process path)
- Has ≥ 1 unit test for the adapter/handler logic
- Has no cross-plugin imports
- Has no external network / DB deps

### Workspace changes

```toml
# /Cargo.toml
members = [
    "crates/*",
    "bee",
    "plugins/bee-plugin-binance-mock",
    "plugins/bee-plugin-google-news-mock",
    "plugins/bee-plugin-influxdb-mock",
    "plugins/bee-plugin-mongodb-mock",
    "bee-plugin-ta-lib-mock",
]
```

Wait — the wildcard `crates/*` would also match `crates/bee-...`.
We need explicit listing or a different glob.

Actually `crates/*` does NOT match nested dirs. The current
`members = ["crates/bee-...", "bee"]` is explicit. I'll add the
5 plugin paths explicitly.

### `bee-registry` deps

```toml
# crates/bee-registry/Cargo.toml
[dependencies]
bee-plugin-sdk = { workspace = true }
libloading = "0.8"
```

## Acceptance criteria

- [ ] `cargo build --workspace` clean (0 warnings)
- [ ] `cargo test --workspace` all pass; 5 new test files
- [ ] Each plugin crate has a working `bee_plugin_init` symbol
- [ ] An end-to-end test in `bee-registry` builds one of the
      plugins, loads it via `load_library`, and verifies the
      manifest matches
- [ ] `cdylib_plugin!` macro documented in `docs/internals.md`
- [ ] All 5 plugin manifests declare `abi_version = "v1"`

## Risks

1. **macOS sandboxing**: `cdylib` loading on macOS may need
   `LD_LIBRARY_PATH` or `DYLD_LIBRARY_PATH` set. Tests should
   `cargo build` the plugin first so the `.dylib` is at a known
   path.
2. **`tokio` runtime in `cdylib`**: Plugins that need to spawn
   background tasks (binance/google-news) need a `tokio` runtime.
   Strategy: store a `tokio::runtime::Handle` in `PluginHandle.inner`,
   use it to spawn the event-generation task. The mock plugins
   will use the test runtime.

3. **Workspace glob conflict**: `crates/*` + `plugins/*` could
   conflict if I use wildcards. Use explicit listing.

## Implementation order

1. Add `libloading` to `bee-registry` + `load_library` function + tests
2. Add `cdylib_plugin!` macro to `bee-plugin-sdk` + tests
3. Build 5 plugin crates (parallel via subagents)
4. End-to-end smoke test
5. `cargo build --workspace` + `cargo test --workspace` clean
6. Commit per step
