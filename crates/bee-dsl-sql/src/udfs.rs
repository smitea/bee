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
//! 2. **Perf-fib plugin UDFs** (always-on, in-process): `fib_seed`
//!    and `fib_step` from the `bee-plugin-perf-fib` crate. The
//!    proper architecture loads the plugin as a cdylib via FFI; the
//!    S41 MVP shortcut links it as a regular rlib and calls the
//!    plugin's pure functions directly (see "in-process shortcut"
//!    note below).
//!
//! ## In-process shortcut for perf-fib
//!
//! The plugin's `fib_step` takes a `&BeeHostV1` and reads/writes
//! state through the host's KV. For the S41 MVP, the host is not
//! available (the plugin loader is deferred to a follow-up task).
//! The dispatcher here re-implements the state evolution in
//! process, using a shared `Arc<Mutex<FibState>>` that mirrors the
//! KV-backed `FibState { prev2, prev1 }` rolling state.
//!
//! The first call to `fib_step` sees the initial state
//! `(0, 1)` (the Fibonacci seed pair) and returns `1`. Subsequent
//! calls evolve the state and return the next Fibonacci value. The
//! state persists for the lifetime of the `SessionContext` (one
//! process-global instance); the demo resets by virtue of each
//! `bee run` invocation being a fresh process.
//!
//! This shortcut is documented in the S41 plan §9c and is the
//! trade-off the S41 MVP accepts: the demo runs end-to-end without
//! the cdylib FFI loader. The migration path is to replace
//! `register_perf_fib` with a FFI-based loader that calls the
//! plugin's `bee_plugin_init`, reads the `PluginManifest`, and
//! dispatches each Handler through the `HandlerVtable` (see
//! `crates/bee-plugin-sdk/src/vtable.rs`).

use std::sync::{Arc, Mutex};

use arrow_array::Array;
use datafusion::arrow::array::Int64Array;
use datafusion::arrow::datatypes::DataType;
use datafusion::common::Result as DfResult;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::{create_udf, ColumnarValue, ScalarUDF, Volatility};
use datafusion::prelude::SessionContext;

use bee_plugin_perf_fib::{FibState, fib_seed};

/// S41 (9c): register `fib_seed` and `fib_step` as DataFusion scalar
/// UDFs on the given `SessionContext`.
///
/// `fib_seed` is stateless and delegates directly to the plugin's
/// pure function. `fib_step` is stateful and uses a shared
/// `Arc<Mutex<FibState>>` (see module docs for the in-process
/// shortcut rationale).
pub fn register_perf_fib(ctx: &SessionContext) -> DfResult<()> {
    // fib_seed(n: Int64) -> Int64: stateless, delegates to the plugin.
    // Per-row: returns a result array of the same length as the input.
    ctx.register_udf(ScalarUDF::from(create_udf(
        "fib_seed",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(|args: &[ColumnarValue]| -> DfResult<ColumnarValue> {
            let arg = &args[0];
            let in_arr = match arg {
                ColumnarValue::Scalar(s) => s.to_array_of_size(1)?,
                ColumnarValue::Array(a) => a.clone(),
            };
            let in_arr = in_arr
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| {
                    DataFusionError::Plan(format!(
                        "fib_seed: arg must be Int64, got {:?}",
                        in_arr.data_type()
                    ))
                })?
                .clone();
            let out: Vec<i64> = (0..in_arr.len())
                .map(|i| fib_seed(in_arr.value(i) as u64) as i64)
                .collect();
            Ok(ColumnarValue::Array(std::sync::Arc::new(Int64Array::from(out))))
        }),
    )));

    // fib_step(n: Int64) -> Int64: stateful, uses a shared FibState.
    // The state is initialised to (0, 1) on first use (matches the
    // plugin's `fib_step` default when no KV state exists).
    //
    // DataFusion invokes a scalar UDF with a `ColumnarValue::Array`
    // (NOT per-row scalars) when the argument is a column. The
    // dispatcher must therefore iterate the input array row by row,
    // evolving the state once per row, and return a result array.
    // Returning a scalar (even though the UDF is declared scalar)
    // would broadcast the same value to every row.
    let state: Arc<Mutex<FibState>> = Arc::new(Mutex::new(FibState { prev2: 0, prev1: 1 }));
    let state_for_udf = Arc::clone(&state);
    ctx.register_udf(ScalarUDF::from(create_udf(
        "fib_step",
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
                        "fib_step: arg must be Int64, got {:?}",
                        in_arr.data_type()
                    ))
                })?
                .clone();
            let mut s = state_for_udf.lock().map_err(|e| {
                DataFusionError::Plan(format!("fib_step state lock poisoned: {e}"))
            })?;
            // Evolve the state once per input row. The plugin's
            // `fib_step` ignores the `n` arg semantically; it only
            // uses it as a "tick" indicator. We mirror that.
            let mut out: Vec<i64> = Vec::with_capacity(in_arr.len());
            for _ in 0..in_arr.len() {
                let new_value = s.next();
                *s = FibState {
                    prev2: s.prev1,
                    prev1: new_value,
                };
                out.push(new_value as i64);
            }
            let out_arr = Arc::new(Int64Array::from(out));
            Ok(ColumnarValue::Array(out_arr))
        }),
    )));

    Ok(())
}

/// S41 (9c): register test-fixture UDFs (`generate_series`,
/// `generate_events`) on the given `SessionContext`. Only available
/// when the `test-fixtures` feature is enabled; the function is
/// stubbed out (no-op) in production builds.
#[cfg(feature = "test-fixtures")]
pub fn register_test_fixtures(ctx: &SessionContext) -> DfResult<()> {
    // generate_series(start: Int64, end: Int64) -> Int64Array.
    // Returns a single Int64Array containing the inclusive range
    // [start, end]. Use as `FROM UNNEST(generate_series(1, N))` to
    // expand into rows (DataFusion 50 has no UDTF support; UNNEST
    // is the canonical way to turn a scalar UDF's array result into
    // a table source).
    ctx.register_udf(ScalarUDF::from(create_udf(
        "generate_series",
        vec![DataType::Int64, DataType::Int64],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(|args: &[ColumnarValue]| -> DfResult<ColumnarValue> {
            crate::test_fixtures::generate_series_impl(args)
        }),
    )));

    // generate_events(schema: Int64, count: Int64, seed: Int64) -> StructArray.
    // Returns a StructArray with { user_id: Int64, ts: Int64 }.
    // For the S41 demo, the schema arg is accepted but ignored.
    ctx.register_udf(ScalarUDF::from(create_udf(
        "generate_events",
        vec![DataType::Int64, DataType::Int64, DataType::Int64],
        DataType::Struct(
            datafusion::arrow::datatypes::Fields::from(vec![
                datafusion::arrow::datatypes::Field::new("user_id", DataType::Int64, false),
                datafusion::arrow::datatypes::Field::new("ts", DataType::Int64, false),
            ]),
        ),
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

/// Extract a single i64 from a `ColumnarValue` (scalar or single-row
/// array). The first row of an array is used. Mirrors the
/// `extract_i64` helper in `test_fixtures.rs` but lives here so
/// the UDF dispatchers don't need to depend on the test-fixtures
/// module.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fib_seed_udf_returns_zero_for_zero() {
        let state = Arc::new(Mutex::new(FibState { prev2: 0, prev1: 1 }));
        let s = state.lock().unwrap();
        assert_eq!(fib_seed(0), 0, "fib_seed(0) = 0 per the plugin");
        assert_eq!(s.next(), 1, "initial FibState (0,1).next() = 1");
    }

    #[test]
    fn fib_step_state_evolves_correctly() {
        let state = Arc::new(Mutex::new(FibState { prev2: 0, prev1: 1 }));
        // 20-step sequence starting from the seed pair (0, 1):
        // 1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610,
        // 987, 1597, 2584, 4181, 6765, 10946
        let expected = [
            1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597,
            2584, 4181, 6765, 10946,
        ];
        for (i, &exp) in expected.iter().enumerate() {
            let mut s = state.lock().unwrap();
            let v = s.next();
            *s = FibState { prev2: s.prev1, prev1: v };
            assert_eq!(v, exp, "step {i}: expected {exp}, got {v}");
        }
    }
}
