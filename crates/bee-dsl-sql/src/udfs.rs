//! S41 (9c): UDF registration for the SQL execution path.
//!
//! Two kinds of UDFs are registered here:
//!
//! 1. **Test-fixture UDFs** (gated on the `test-fixtures` feature):
//!    `generate_series` and `generate_events` from
//!    [`crate::test_fixtures`]. These are always-on in the
//!    `test-fixtures` build; production builds skip the
//!    registration.
//!
//! 2. **Perf-fib plugin UDFs** (S41 follow-up 4): `fib_seed` and
//!    `fib_step` are loaded from the `bee-plugin-perf-fib`
//!    `cdylib` via [`crate::plugin_loader::load_plugin`] and
//!    dispatched through the plugin's `HandlerVtable` function
//!    pointers. The `LoadedPlugin` is leaked (mem::forget) so
//!    the cdylib lives for the program's lifetime — DataFusion
//!    UDFs hold raw pointers into the cdylib, so unloading
//!    mid-process would be UB.
//!
//! ## FFI dispatch (per the vtable contract)
//!
//! For each Handler advertised in the plugin's `PluginManifest`,
//! `register_perf_fib` looks up its `*const HandlerVtable` in the
//! plugin's handle. If found, it registers a DataFusion
//! `ScalarUDF` whose dispatcher:
//!
//! - bincode-encodes the input row as the handler's `event`
//!   (a `u64` for the perf-fib handlers);
//! - passes the per-UDF state blob (an opaque `Vec<u8>` rolled
//!   forward across invocations);
//! - calls the vtable's `handle` function;
//! - bincode-decodes the result (an `i128`) and returns it as an
//!   `Int64` cell.
//!
//! Handlers whose vtable is not present in the plugin's `handlers`
//! map are skipped with a warning. (The current `bee-plugin-perf-fib`
//! scaffold does not populate this map; the UDF is therefore
//! not registered, and SQL referencing it will fail at plan
//! time. The follow-up plugin fix is to add a vtable shim for
//! each handler — out of scope for the host-side loader work.)

use std::sync::{Arc, Mutex};

use datafusion::arrow::array::Int64Array;
use datafusion::arrow::datatypes::{DataType, Field};
use datafusion::common::Result as DfResult;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::{create_udf, ColumnarValue, ScalarUDF, Volatility};
use datafusion::prelude::SessionContext;

use crate::plugin_loader;

/// S41 follow-up 4: register `fib_seed` and `fib_step` as DataFusion
/// scalar UDFs on the given `SessionContext`.
///
/// The plugin's cdylib is located at one of:
///
/// - `./target/release/libbee_plugin_perf_fib.dylib`
/// - `./target/release/deps/libbee_plugin_perf_fib.dylib`
///
/// (the `cargo build` output). The first existing path is used.
/// If neither exists, the function returns an error describing
/// the missing file (so `cargo run -p bee -- run ...` fails with
/// a clear message instead of a generic dlopen error).
///
/// For each Handler in the plugin's manifest, the function
/// looks up the corresponding vtable pointer in the loaded
/// plugin's `handlers` map. If present, it registers a
/// `ScalarUDF` whose dispatcher calls the vtable's `handle`
/// function. If absent, the Handler is skipped with a warning
/// (see the module docs for the vtable-map caveat).
pub fn register_perf_fib(ctx: &SessionContext) -> DfResult<()> {
    let path = plugin_loader::find_perf_fib_cdylib().ok_or_else(|| {
        DataFusionError::Plan(
            "register_perf_fib: cdylib not found. Searched:\n  \
             target/release/libbee_plugin_perf_fib.dylib\n  \
             target/release/deps/libbee_plugin_perf_fib.dylib\n  \
             target/debug/libbee_plugin_perf_fib.dylib\n  \
             target/debug/deps/libbee_plugin_perf_fib.dylib\n\
             Run `cargo build -p bee-plugin-perf-fib` first."
                .to_string(),
        )
    })?;
    let loaded = plugin_loader::load_plugin(&path).map_err(|e| {
        DataFusionError::Plan(format!("register_perf_fib: {e}"))
    })?;

    let manifest = loaded.manifest().clone();
    eprintln!(
        "register_perf_fib: loaded plugin `{}` (abi={}, handlers={})",
        manifest.name,
        manifest.abi_version,
        manifest.handlers.len(),
    );

    // Per-UDF state blob (opaque bincode bytes, rolled forward
    // by the vtable's `handle` function across invocations).
    // The first call passes an empty blob; subsequent calls
    // pass whatever the vtable wrote to `new_state_out` on
    // the previous call.
    let state: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

    // Resolve each Handler's vtable pointer BEFORE leaking the
    // LoadedPlugin (the lookup borrows the loaded plugin's
    // internal `handlers` map). After `leak()`, the library is
    // kept alive for the program's lifetime; the vtable
    // pointers remain valid forever.
    let handler_vtables: Vec<(String, plugin_loader::SendVtable)> = manifest
        .handlers
        .iter()
        .filter_map(|h| {
            let vtable = loaded.handler_vtable(&h.name)?;
            Some((h.name.clone(), plugin_loader::SendVtable::new(vtable)))
        })
        .collect();
    for handler in &manifest.handlers {
        if loaded.handler_vtable(&handler.name).is_none() {
            eprintln!(
                "register_perf_fib: handler `{name}` has no vtable \
                 in the loaded plugin; skipping UDF registration \
                 (the plugin's init() must call \
                  host.register_handler_vtable for this handler)",
                name = handler.name,
            );
        }
    }

    // Leak the LoadedPlugin: keep the cdylib mapped for the
    // program's lifetime. DataFusion UDFs hold raw pointers
    // into the cdylib (vtable functions, manifest strings),
    // so unloading mid-process would be UB.
    loaded.leak();

    for (handler_name, vtable_ptr) in handler_vtables {

        // Per the perf-fib demo, both `fib_seed` and `fib_step`
        // are declared `Int64 -> Int64` in SQL. DataFusion
        // invokes a scalar UDF with a `ColumnarValue::Array`
        // (NOT per-row scalars) when the argument is a
        // column. The dispatcher iterates the input array row
        // by row, calling the vtable once per row, and
        // accumulates the results into an output array.
        let state_for_udf = Arc::clone(&state);
        let handler_name_for_udf = handler_name.clone();
        ctx.register_udf(create_udf(
            &handler_name,
            vec![DataType::Int64],
            DataType::Int64,
            Volatility::Volatile,
            Arc::new(move |args: &[ColumnarValue]| -> DfResult<ColumnarValue> {
                let arg = &args[0];
                // Extract the input Int64 array.
                let in_arr = match arg {
                    ColumnarValue::Scalar(s) => s.to_array_of_size(1)?,
                    ColumnarValue::Array(a) => a.clone(),
                };
                let in_arr = in_arr
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| {
                        DataFusionError::Plan(format!(
                            "{}: arg must be Int64, got {:?}",
                            handler_name_for_udf,
                            in_arr.data_type()
                        ))
                    })?
                    .clone();

                let mut state_guard = state_for_udf.lock().map_err(|e| {
                    DataFusionError::Plan(format!(
                        "{}: state lock poisoned: {e}",
                        handler_name_for_udf
                    ))
                })?;

                let mut out: Vec<i64> = Vec::with_capacity(in_arr.len());
                for i in 0..in_arr.len() {
                    let n = in_arr.value(i);
                    // SAFETY: `vtable_ptr` was produced by the
                    // loaded cdylib's `init()` and is valid for
                    // the program's lifetime (the `LoadedPlugin`
                    // is leaked above). The state's lifetime is
                    // the same as `state_guard`'s scope. The
                    // vtable's `handle` function reads `state`
                    // and `event`; the new state is written
                    // back into `state_guard` (we copy the
                    // bytes after the call returns).
                    let (value, new_state_bytes) = unsafe {
                        plugin_loader::call_handler_vtable(
                            vtable_ptr.as_ptr(),
                            n,
                            &state_guard,
                        )
                    }?;
                    // Roll the state forward so the next
                    // iteration sees the previous call's
                    // `new_state_out`. The plugin's contract
                    // is that the new state is a fresh
                    // bincode-encoded blob; for a stateless
                    // handler (like `fib_seed`) the plugin
                    // writes back the same sentinel, so the
                    // assignment is a no-op.
                    *state_guard = new_state_bytes;
                    out.push(value);
                }
                drop(state_guard);

                let out_arr = Arc::new(Int64Array::from(out));
                Ok(ColumnarValue::Array(out_arr))
            }),
        ));
    }

    Ok(())
}

/// S41 (9c): register test-fixture UDFs (`generate_series`,
/// `generate_events`) on the given `SessionContext`. Only available
/// when the `test-fixtures` feature is enabled; the function is
/// stubbed out (no-op) in production builds.
#[cfg(feature = "test-fixtures")]
pub fn register_test_fixtures(ctx: &SessionContext) -> DfResult<()> {
    // generate_series(start: Int64, end: Int64) -> List<Int64>.
    // Returns a single-row `ListArray` containing the inclusive
    // range [start, end]. Use as
    // `FROM UNNEST(generate_series(1, N))` to expand into rows
    // (DataFusion 50 has no UDTF support; UNNEST is the canonical
    // way to turn a scalar UDF's list result into a table source).
    ctx.register_udf(ScalarUDF::from(create_udf(
        "generate_series",
        vec![DataType::Int64, DataType::Int64],
        DataType::List(std::sync::Arc::new(Field::new(
            "item",
            DataType::Int64,
            true,
        ))),
        Volatility::Immutable,
        Arc::new(|args: &[ColumnarValue]| -> DfResult<ColumnarValue> {
            crate::test_fixtures::generate_series_impl(args)
        }),
    )));

    // generate_events(schema: Int64, count: Int64, seed: Int64)
    //   -> List<Struct<user_id, ts>>.
    // The impl returns a single-row ListArray whose element type
    // is `Struct<user_id: Int64, ts: Int64>`; UNNEST then flattens
    // it to `count` rows. For the S41 demo, the schema arg is
    // accepted but ignored (the output struct is always
    // { user_id, ts } — the only columns the multi_stream_analytics
    // SQL references).
    ctx.register_udf(ScalarUDF::from(create_udf(
        "generate_events",
        vec![DataType::Int64, DataType::Int64, DataType::Int64],
        DataType::List(std::sync::Arc::new(Field::new(
            "item",
            DataType::Struct(datafusion::arrow::datatypes::Fields::from(vec![
                Field::new("user_id", DataType::Int64, false),
                Field::new("ts", DataType::Int64, false),
            ])),
            false,
        ))),
        Volatility::Immutable,
        Arc::new(|args: &[ColumnarValue]| -> DfResult<ColumnarValue> {
            crate::test_fixtures::generate_events_impl(args)
        }),
    )));

    Ok(())
}

/// S41 (9c): no-op stub for production builds (the
/// `test-fixtures` feature is off). The compile-time `#[cfg]`
/// above ensures this branch is the one that compiles.
#[cfg(not(feature = "test-fixtures"))]
pub fn register_test_fixtures(_ctx: &SessionContext) -> DfResult<()> {
    Ok(())
}
