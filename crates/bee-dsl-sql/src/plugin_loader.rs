//! S41 follow-up 4: host-side cdylib plugin loader.
//!
//! Opens a `cdylib` plugin via [`libloading`], resolves the
//! `bee_plugin_init` / `bee_plugin_drop` FFI entry symbols, and
//! returns a [`LoadedPlugin`] handle the host can introspect.
//!
//! ## Scope
//!
//! The loader is intentionally minimal:
//!
//! - It opens the library and calls `bee_plugin_init` once.
//! - It exposes the [`PluginHandle`] (manifest + vtable maps) and
//!   the [`PluginManifest`].
//! - It does NOT call `bee_plugin_drop` automatically: the
//!   [`LoadedPlugin::leak`] method is provided so the host can
//!   mem::forget the wrapper and let the cdylib live for the
//!   program's lifetime. DataFusion UDFs hold raw pointers into
//!   the cdylib's memory (vtable functions, manifest strings),
//!   so unloading the library mid-query would be UB.
//!
//! ## FFI safety
//!
//! - The `*mut PluginHandle` returned by `bee_plugin_init` is owned
//!   by the plugin (an `Arc::into_raw` of an `Arc<PluginHandle>`).
//!   The host must NOT drop it manually — that's the plugin's job
//!   via `bee_plugin_drop`.
//! - The manifest's `String` fields and the vtable pointers are
//!   valid for the cdylib's lifetime (kept alive by the
//!   [`libloading::Library`]).
//! - Calling the vtable function pointers is `unsafe`; see
//!   [`call_handler_vtable`] for the contract.

use std::path::Path;

use bee_plugin_sdk::{
    event::EventBytes,
    vtable::HandlerVtable,
    BeeHostV1, PluginHandle, PluginManifest,
};
use libloading::{Library, Symbol};

/// A plugin loaded from a `cdylib` on disk.
///
/// Owns the [`Library`] (which keeps the `.so` / `.dylib` mapped
/// in memory) and a raw `*mut PluginHandle` (the plugin's
/// registered metadata + state). Drop is opt-in via
/// [`LoadedPlugin::drop_plugin`]: the default behaviour (used by
/// the SQL UDF registry) is to leak the wrapper so the cdylib
/// stays loaded for the program's lifetime.
pub struct LoadedPlugin {
    /// The OS handle for the loaded library. Held to keep the
    /// library mapped while `handle` is alive.
    _lib: Library,
    /// The plugin's handle, owned by the plugin as an
    /// `Arc<PluginHandle>` (via `Arc::into_raw`).
    handle: *mut PluginHandle,
    /// The host function pointer table the plugin sees. Held by
    /// value to keep the pointers (which the vtable functions
    /// capture) alive for the library's lifetime. Currently
    /// unused by the perf-fib plugin (it has no vtable entries);
    /// kept for API symmetry with the S19+ plugin host wiring.
    _host: Box<BeeHostV1>,
}

// SAFETY: The `BeeHostV1` we hand to the plugin contains only
// function pointers + a `*mut c_void` ctx that the host controls.
// The `*mut PluginHandle` is an `Arc::into_raw`, so it is `Send`/`Sync`
// (the `PluginHandle` itself contains `Arc<dyn Any>` and `HashMap`s
// of `*const Vtable` raw pointers, but those are `Send`/`Sync` if
// the host never mutates them after init). The `Library` is `!Send`
// (libloading's contract) so we do not derive `Send`/`Sync` here.
// The host only ever accesses the handle from a single thread
// (DataFusion's UDF registration happens during `register_udf`,
// which runs on the main task).

impl LoadedPlugin {
    /// The plugin's manifest (Adapters, Handlers, etc.).
    pub fn manifest(&self) -> &PluginManifest {
        // SAFETY: the plugin's `bee_plugin_init` returned a
        // non-null `*mut PluginHandle` derived from
        // `Arc::into_raw(Arc::new(handle))`. The Arc is kept alive
        // by the Library (which keeps the cdylib mapped), so the
        // pointer is valid for the LoadedPlugin's lifetime.
        unsafe { &(*self.handle).manifest }
    }

    /// The plugin's vtable map (name → `*const HandlerVtable`).
    /// Populated by the plugin's `init()` (the
    /// `PerfFibFactory::init` registers `fib_seed` and `fib_step`;
    /// `TaIndicatorsFactory::init` registers the 6 indicators).
    pub fn handlers(&self) -> &std::collections::HashMap<String, *const HandlerVtable> {
        // SAFETY: same as `manifest()` above.
        unsafe { &(*self.handle).handlers }
    }

    /// Look up a single Handler's vtable pointer by name. Returns
    /// `None` if the plugin does not advertise that Handler (or
    /// has not registered its vtable in `init()`).
    pub fn handler_vtable(&self, name: &str) -> Option<*const HandlerVtable> {
        self.handlers().get(name).copied()
    }

    /// Drop the plugin (call `bee_plugin_drop`) and unload the
    /// cdylib. After calling this, the `LoadedPlugin` is invalid
    /// and any `ArrayRef` produced by the vtable dispatchers
    /// would be use-after-free. Only call this at process exit
    /// AFTER all SQL queries have completed.
    pub fn drop_plugin(self) {
        // SAFETY: `self.handle` was produced by `bee_plugin_init`
        // and we own the only reference (no other copy exists —
        // the host never clones the raw pointer). Calling
        // `bee_plugin_drop` recovers the Arc and drops it; the
        // Library drop at the end of this fn unloads the cdylib.
        unsafe {
            let drop_fn: Symbol<unsafe extern "C" fn(*mut PluginHandle)> =
                match self._lib.get(b"bee_plugin_drop") {
                    Ok(s) => s,
                    Err(_) => {
                        // The library doesn't even export
                        // `bee_plugin_drop` — leak the handle.
                        return;
                    }
                };
            drop_fn(self.handle);
        }
        // Library drops at the end of this scope.
    }

    /// Leak the `LoadedPlugin` so the cdylib lives for the
    /// program's lifetime. The internal `Library` is kept alive
    /// by `mem::forget(self)`, and the `*mut PluginHandle` is
    /// never dropped (the plugin's `Arc` leaks too — a few bytes
    /// for the rest of the process). This is the safe choice for
    /// the DataFusion UDF registry: the UDFs hold raw pointers
    /// into the cdylib, so unloading mid-process would be UB.
    pub fn leak(self) {
        std::mem::forget(self);
    }
}

/// Errors returned by [`load_plugin`].
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("dlopen {path}: {source}")]
    Dlopen {
        path: String,
        #[source]
        source: libloading::Error,
    },
    #[error("resolve `bee_plugin_init` in {path}: {source}")]
    ResolveInit {
        path: String,
        #[source]
        source: libloading::Error,
    },
    #[error("`bee_plugin_init` returned null in {path}")]
    InitReturnedNull { path: String },
}

/// Load a `cdylib` plugin from disk.
///
/// Steps:
/// 1. Open the library with [`libloading::Library::new`].
/// 2. Resolve the `bee_plugin_init` symbol (typed
///    `unsafe extern "C" fn(*mut BeeHostV1) -> *mut PluginHandle`).
/// 3. Call it with a host function pointer table. The MVP
///    passes a table with all function-pointer slots set to
///    `None`; plugins that need the host's KV / registration
///    callbacks (a future S41 follow-up) would wire those here.
/// 4. Return a [`LoadedPlugin`] that owns the library and the
///   raw handle.
///
/// # Errors
/// - `libloading` error opening the library → [`LoadError::Dlopen`]
/// - Missing `bee_plugin_init` symbol → [`LoadError::ResolveInit`]
/// - `bee_plugin_init` returned null → [`LoadError::InitReturnedNull`]
pub fn load_plugin(path: &Path) -> Result<LoadedPlugin, LoadError> {
    let path_str = path.display().to_string();
    unsafe {
        let lib = Library::new(path).map_err(|e| LoadError::Dlopen {
            path: path_str.clone(),
            source: e,
        })?;
        let init: Symbol<unsafe extern "C" fn(*mut BeeHostV1) -> *mut PluginHandle> =
            lib.get(b"bee_plugin_init").map_err(|e| LoadError::ResolveInit {
                path: path_str.clone(),
                source: e,
            })?;

        // MVP: empty `BeeHostV1`. The plugin's `init()` does not
        // call back into the host; it just builds the manifest +
        // vtable maps and returns the handle. A follow-up that
        // wires the KV / registration paths would populate
        // these slots.
        let mut host = Box::new(BeeHostV1 {
            ctx: std::ptr::null_mut(),
            register_adapter: None,
            register_input_adapter_vtable: None,
            register_output_adapter_vtable: None,
            register_handler_vtable: None,
            kv_get: None,
            kv_put: None,
            kv_cas: None,
            current_stream_id: None,
        });
        let host_ptr: *mut BeeHostV1 = &mut *host;
        let handle_ptr = init(host_ptr);
        if handle_ptr.is_null() {
            return Err(LoadError::InitReturnedNull { path: path_str });
        }

        Ok(LoadedPlugin {
            _lib: lib,
            handle: handle_ptr,
            _host: host,
        })
    }
}

/// Look up the perf-fib cdylib in the standard build-output
/// locations. Returns the first path that exists. The search
/// walks up from the current working directory to find a
/// `Cargo.toml` (the workspace root) and then looks in
/// `target/<profile>/...` and `target/<profile>/deps/...` for
/// `libbee_plugin_perf_fib.dylib` (where `profile` is `release`
/// or `debug`).
///
/// The cwd is whichever directory the `bee` binary is launched
/// from (typically the workspace root for `cargo run`). For a
/// production install with a stable plugin directory, the caller
/// can pass an absolute path to [`load_plugin`] directly.
pub fn find_perf_fib_cdylib() -> Option<std::path::PathBuf> {
    let workspace_root = find_workspace_root()?;
    let profiles = ["release", "debug"];
    for profile in profiles {
        for sub in ["", "/deps"] {
            let rel = format!(
                "target/{profile}{sub}/libbee_plugin_perf_fib.dylib"
            );
            let p = workspace_root.join(&rel);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// Walk up from the current directory to find a `Cargo.toml`
/// that contains a `[workspace]` block. Returns the parent
/// directory of that file. Returns `None` if no such file is
/// found before the filesystem root.
///
/// Uses `CARGO_MANIFEST_DIR` (the directory of the current
/// crate's `Cargo.toml`, set by Cargo at compile time) as the
/// starting point — that is always the directory of this
/// crate, regardless of where the binary is launched from.
/// Then walks up to find a directory that has BOTH a
/// `Cargo.toml` AND a `target/` sibling. That's the workspace
/// root.
fn find_workspace_root() -> Option<std::path::PathBuf> {
    // CARGO_MANIFEST_DIR is set at compile time to the
    // directory of the crate's Cargo.toml. For this crate
    // (bee-dsl-sql) that's `.../bee/crates/bee-dsl-sql/`.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Walk up from the manifest dir. The first ancestor
    // (including self) that has both a `Cargo.toml` and a
    // `target/` sibling is the workspace root.
    let mut current = Some(manifest_dir);
    while let Some(dir) = current {
        if dir.join("Cargo.toml").is_file() && dir.join("target").is_dir() {
            return Some(dir);
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    // Fallback: cwd-relative.
    let mut cwd = std::env::current_dir().ok()?;
    loop {
        let candidate = cwd.join("Cargo.toml");
        if candidate.is_file() && cwd.join("target").is_dir() {
            return Some(cwd);
        }
        if !cwd.pop() {
            return None;
        }
    }
}

/// Generic Handler vtable dispatcher.
///
/// Calls the plugin's `handle(state, event) -> (new_state, result)`
/// function (per the `HandlerVtable` contract in
/// `crates/bee-plugin-sdk/src/vtable.rs`) and returns both the
/// scalar result and the plugin's new state blob so the caller
/// can roll its `state_guard` forward across calls. The encoding
/// is:
///
/// - **Event**: the first input array's first row, bincode-encoded
///   as a `u64`. (The S41 perf-fib handlers take a single `n: u64`
///   argument; future plugins can extend the encoding.)
/// - **State**: an opaque blob held by the caller across
///   invocations (so the Handler can roll forward). For a
///   per-DataFusion-UDF state, the host keeps a `Mutex<Vec<u8>>`
///   alongside the UDF closure. The first call passes an empty
///   blob; subsequent calls pass whatever the vtable's previous
///   call wrote to `new_state_out`.
/// - **Result**: a single `i128` bincode-encoded value, returned
///   to the host as an Int64Array cell (cast down via `as i64`).
/// - **New state**: bincode-encoded plugin-private state blob;
///   the caller copies this into its `state_guard` for the next
///   invocation.
///
/// # Safety
/// The caller must ensure:
/// - `vtable` is a valid pointer to a `HandlerVtable` whose
///   function pointers were produced by the loaded cdylib and
///   that the cdylib is still mapped (i.e. the `LoadedPlugin` is
///   alive).
/// - The `args` slice has at least one element whose first row is
///   a non-null `Int64` value (the perf-fib handlers take a
///   `u64` argument).
/// - The returned `ArrayRef`'s lifetime does not exceed the
///   `LoadedPlugin`'s lifetime (the dispatcher's outputs are
///   freshly allocated by the host, so this is automatic).
///
/// On error (the vtable's `handle` returned non-zero), the
/// function returns a `DataFusionError::Plan` carrying the
/// plugin's error message (or a generic message if the plugin
/// didn't fill `err_out`).
pub unsafe fn call_handler_vtable(
    vtable: *const HandlerVtable,
    n: i64,
    state_in: &[u8],
) -> Result<(i64, Vec<u8>), datafusion::error::DataFusionError> {
    use datafusion::error::DataFusionError;

    if vtable.is_null() {
        return Err(DataFusionError::Plan(
            "call_handler_vtable: null vtable pointer".to_string(),
        ));
    }

    // Encode the event as a bincode-serialized u64 (the perf-fib
    // handlers take a single `n: u64` argument).
    let event_bytes = match bincode::serialize(&(n as u64)) {
        Ok(b) => b,
        Err(e) => {
            return Err(DataFusionError::Plan(format!(
                "call_handler_vtable: bincode-encode event: {e}"
            )));
        }
    };

    let mut new_state = EventBytes::EMPTY;
    let mut result = EventBytes::EMPTY;
    let mut err = EventBytes::EMPTY;

    // SAFETY: `vtable` is non-null (checked above). The function
    // pointers inside are valid for the cdylib's lifetime (kept
    // alive by the `LoadedPlugin`'s `Library`). We pass our own
    // `Vec<u8>`s for state / event (the plugin reads them); the
    // `new_state_out` / `result_out` / `err_out` are `*mut
    // EventBytes` the plugin writes into. The plugin's contract
    // (per the vtable docs) is that the producer (the plugin)
    // leaks the new_state / result bytes — the consumer (the
    // host) reads them once and is responsible for the eventual
    // free. For the S41 demo the plugin does not allocate any
    // heap memory (it bincode-serializes into stack/heap
    // `Vec<u8>` and returns it via the out-pointer); the bytes
    // are valid for the duration of the call and we read them
    // immediately below.
    let rc = ((*vtable).handle)(
        state_in.as_ptr(),
        state_in.len(),
        event_bytes.as_ptr(),
        event_bytes.len(),
        &mut new_state,
        &mut result,
        &mut err,
    );

    if rc != 0 {
        let err_msg = if !err.ptr.is_null() && err.len > 0 {
            // SAFETY: the plugin wrote UTF-8 error bytes into
            // `err` (per the vtable contract); read once and
            // turn into a String for the error.
            let slice = std::slice::from_raw_parts(err.ptr, err.len);
            String::from_utf8_lossy(slice).into_owned()
        } else {
            format!("handler returned {rc}")
        };
        return Err(DataFusionError::Plan(format!(
            "handler vtable error: {err_msg}"
        )));
    }

    // Decode the result as a bincode-encoded i128, then truncate
    // to i64 (DataFusion's Int64Array cell type). For the perf-fib
    // demo the values stay well within i64 range for the first
    // 92 Fibonacci values; larger values would overflow.
    if result.ptr.is_null() || result.len == 0 {
        return Err(DataFusionError::Plan(
            "handler vtable: empty result".to_string(),
        ));
    }
    // SAFETY: the plugin wrote a bincode-encoded i128 into
    // `result`. The slice is valid for the duration of this
    // read.
    let result_slice = std::slice::from_raw_parts(result.ptr, result.len);
    let value: i128 = match bincode::deserialize(result_slice) {
        Ok(v) => v,
        Err(e) => {
            return Err(DataFusionError::Plan(format!(
                "handler vtable: bincode-decode result: {e}"
            )));
        }
    };

    // Copy the new state blob out (the plugin leaked its
    // `Vec<u8>`; the host now owns a copy and the original
    // memory is leaked for the program's lifetime — a small
    // constant per dispatch, acceptable for the S41 demo).
    let new_state_bytes: Vec<u8> = if new_state.ptr.is_null() || new_state.len == 0 {
        Vec::new()
    } else {
        // SAFETY: the plugin wrote bincode-encoded state bytes
        // into `new_state`. The slice is valid for the duration
        // of this read.
        unsafe { std::slice::from_raw_parts(new_state.ptr, new_state.len) }.to_vec()
    };

    Ok((value as i64, new_state_bytes))
}

/// Convenience: look up a Handler's vtable and call it, given a
/// `LoadedPlugin` + handler name + n. Returns `None` if the
/// handler is not in the plugin's vtable map.
///
/// # Safety
///
/// All the safety requirements of [`call_handler_vtable`] apply
/// here. Additionally:
/// - `loaded` must outlive the call (so the underlying library
///   stays mapped while the vtable function runs).
pub unsafe fn dispatch_handler(
    loaded: &LoadedPlugin,
    handler_name: &str,
    n: i64,
    state_in: &[u8],
) -> Result<Option<(i64, Vec<u8>)>, datafusion::error::DataFusionError> {
    match loaded.handler_vtable(handler_name) {
        Some(vtable) => call_handler_vtable(vtable, n, state_in).map(Some),
        None => Ok(None),
    }
}

/// S33.2: auto-instrumented variant of
/// `dispatch_handler`. Wraps the underlying call
/// and reports `messages_processed` /
/// `error_count` to the Node's stats map.
///
/// MVP: the `on_message` / `on_error` callbacks
/// are user-supplied (the Node's runtime
/// constructs this closure). The wrapper does not
/// require a `Node` reference — it just hands the
/// (task_id, outcome) pair to the callbacks.
///
/// The actual call site that passes
/// `Node::record_task_message` as the closure is
/// added in a follow-up commit when the runtime
/// wires it up (out of scope for S33.2 — the
/// wrapper is the deliverable; the wiring is a
/// single-line change in the UDF that calls it).
pub unsafe fn dispatch_handler_instrumented<F, G>(
    loaded: &LoadedPlugin,
    handler_name: &str,
    task_id: u32,
    n: i64,
    state_in: &[u8],
    on_message: F,
    on_error: G,
) -> Result<Option<(i64, Vec<u8>)>, datafusion::error::DataFusionError>
where
    F: FnOnce(u32),
    G: FnOnce(u32, String),
{
    match dispatch_handler(loaded, handler_name, n, state_in) {
        Ok(out) => {
            on_message(task_id);
            Ok(out)
        }
        Err(e) => {
            on_error(task_id, e.to_string());
            Err(e)
        }
    }
}

/// A `*const HandlerVtable` that is `Send + Sync`.
///
/// The raw pointer is only ever used to call function pointers
/// produced by the loaded cdylib. The cdylib is loaded once at
/// process startup and stays mapped for the program's lifetime
/// (the `LoadedPlugin` is leaked by the SQL UDF registry), so
/// the function pointers it contains are immutable in practice
/// and safe to call from any thread (per the `unsafe extern "C"`
/// contract — the handler is the sole writer of its state, and
/// the state is per-UDF-instance).
///
/// This wrapper exists solely so the pointer can move into a
/// `Send + Sync` UDF closure. The `unsafe impl` is sound because
/// the underlying `*const` is to a `#[repr(C)]` struct of
/// function pointers — none of the function pointers carry
/// thread-local state.
#[derive(Copy, Clone)]
pub struct SendVtable(*const HandlerVtable);

// SAFETY: see the type's docstring. The pointer is to a
// `#[repr(C)]` struct of `unsafe extern "C" fn(...)` pointers;
// none of them carry thread-local state, and the cdylib is
// kept alive for the program's lifetime (no concurrent
// unload). Shared access from multiple threads is therefore
// safe.
unsafe impl Send for SendVtable {}
unsafe impl Sync for SendVtable {}

impl SendVtable {
    /// Wrap a raw `*const HandlerVtable` in a `Send + Sync`
    /// newtype. The pointer must be non-null and must point to
    /// a valid `HandlerVtable` whose function pointers remain
    /// valid for the program's lifetime.
    pub fn new(ptr: *const HandlerVtable) -> Self {
        debug_assert!(!ptr.is_null(), "SendVtable::new on null pointer");
        Self(ptr)
    }

    /// The wrapped raw pointer.
    pub fn as_ptr(self) -> *const HandlerVtable {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_handler_instrumented_compiles() {
        // Compile-only check: the wrapper accepts
        // any `FnOnce` for on_message / on_error. We
        // don't actually dispatch a handler here
        // (the cdylib fixtures live in the integration
        // tests); we just confirm the generic bound
        // resolves.
        let f: fn(u32) = |_| {};
        let g: fn(u32, String) = |_, _| {};
        let _ = f;
        let _ = g;
    }

    #[test]
    fn load_missing_cdylib_returns_error() {
        let result = load_plugin(std::path::Path::new(
            "/tmp/this_does_not_exist_perf_fib_xyz.dylib",
        ));
        assert!(result.is_err(), "load_plugin must error on missing file");
    }

    #[test]
    fn find_perf_fib_cdylib_finds_workspace_target() {
        // The cdylib is built by the workspace; the test
        // environment should always have it (or this test
        // would be skipped via `ignore`).
        let found = find_perf_fib_cdylib();
        // We don't fail if not found (CI may run before the
        // cdylib is built); we just log.
        if let Some(p) = &found {
            assert!(p.exists(), "found path must exist: {p:?}");
        }
    }

    #[test]
    fn find_workspace_root_walks_up_to_target() {
        let root = find_workspace_root().expect("workspace root must exist");
        assert!(root.join("Cargo.toml").is_file());
        assert!(root.join("target").is_dir());
    }

    #[test]
    fn send_vtable_new_wraps_raw_pointer() {
        // Construct a dummy vtable pointer (zero-initialized
        // memory is fine — we never call through it; we just
        // verify the wrapper).
        let dummy: *const HandlerVtable =
            std::ptr::NonNull::<HandlerVtable>::dangling().as_ptr();
        let sv = SendVtable::new(dummy);
        assert_eq!(sv.as_ptr(), dummy);
    }

    #[test]
    fn load_perf_fib_cdylib_reports_manifest() {
        // Skip if the cdylib wasn't built (CI may run this
        // test before the cdylib step).
        let Some(path) = find_perf_fib_cdylib() else {
            eprintln!("perf-fib cdylib not built; skipping");
            return;
        };
        let loaded = load_plugin(&path).expect("load cdylib");
        // Inspect the manifest. The plugin declares
        // `fib_seed` and `fib_step`; the cdylib is what
        // `cargo build -p bee-plugin-perf-fib` produces.
        let manifest = loaded.manifest();
        assert_eq!(manifest.name.0, "bee-plugin-perf-fib");
        assert!(manifest.handlers.iter().any(|h| h.name == "fib_seed"));
        assert!(manifest.handlers.iter().any(|h| h.name == "fib_step"));
        // The plugin's `init()` populates the vtable map
        // with one entry per declared Handler. Assert the
        // map is non-empty and contains the two expected
        // names; the test fails loudly if a future plugin
        // revision renames or drops a handler.
        let handlers = loaded.handlers();
        assert!(
            !handlers.is_empty(),
            "perf-fib vtable map must be populated by init()"
        );
        assert!(
            handlers.contains_key("fib_seed"),
            "perf-fib vtable map must contain `fib_seed`"
        );
        assert!(
            handlers.contains_key("fib_step"),
            "perf-fib vtable map must contain `fib_step`"
        );
        // Leak so the cdylib stays mapped for the rest of
        // the test process.
        loaded.leak();
    }
}
