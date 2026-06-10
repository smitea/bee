//! Test fixture functions, gated behind the `test-fixtures` feature.
//!
//! These functions are intended for demos and tests only; they produce
//! deterministic data streams without external services.

#![cfg(feature = "test-fixtures")]

use std::sync::Arc;

use datafusion::arrow::array::{
    Array, Int64Array, ListArray, StructArray,
};
use datafusion::arrow::buffer::OffsetBuffer;
use datafusion::arrow::datatypes::{DataType, Field, Fields};
use datafusion::common::Result as DfResult;
use datafusion::logical_expr::ColumnarValue;

/// `generate_series(start, end) -> List<i64>`: emits one event per
/// integer in `[start, end]`. Returns a single-row `ListArray`
/// containing all values. Use as `FROM UNNEST(generate_series(1, N))`
/// to expand into rows (DataFusion 50 has no UDTF support; UNNEST
/// is the canonical way to turn a scalar UDF's list result into a
/// table source).
pub fn generate_series_impl(args: &[ColumnarValue]) -> DfResult<ColumnarValue> {
    if args.len() != 2 {
        return Err(datafusion::error::DataFusionError::Plan(
            "generate_series expects 2 arguments: (start, end)".into(),
        ));
    }
    let start = extract_i64(&args[0], "start")?;
    let end = extract_i64(&args[1], "end")?;
    if end < start {
        return Err(datafusion::error::DataFusionError::Plan(
            "generate_series: end must be >= start".into(),
        ));
    }
    let values: Vec<i64> = (start..=end).collect();
    let values_array = Int64Array::from(values);
    let field = Arc::new(Field::new("item", DataType::Int64, true));
    let offsets = OffsetBuffer::<i32>::new(vec![0, values_array.len() as i32].into());
    let list = ListArray::try_new(
        field,
        offsets,
        Arc::new(values_array),
        None,
    )?;
    Ok(ColumnarValue::Array(std::sync::Arc::new(list)))
}

/// `generate_events(schema, count, seed) -> List<Struct<user_id, ts>>`:
/// emits `count` deterministic pseudo-random events as a single-row
/// `ListArray` whose element type is `Struct<user_id: Int64, ts: Int64>`.
/// The schema arg is accepted but ignored — the output is always a
/// `(user_id: Int64, ts: Int64)` struct.
///
/// The List-wrapping is required so that
/// `UNNEST(generate_events(0, N, seed))` (the preprocessor-rewritten
/// form) expands into `N` rows. A flat `StructArray` of `N` rows would
/// not: `UNNEST` requires an array-valued scalar (per DataFusion 50),
/// and the preprocessor's `AS t(user_id, ts)` rename only works on the
/// element type of a list (not on the row labels of a flat struct).
pub fn generate_events_impl(args: &[ColumnarValue]) -> DfResult<ColumnarValue> {
    if args.len() != 3 {
        return Err(datafusion::error::DataFusionError::Plan(
            "generate_events expects 3 arguments: (schema, count, seed)".into(),
        ));
    }
    let count = extract_i64(&args[1], "count")? as usize;
    let seed = extract_i64(&args[2], "seed")? as u64;

    // LCG: x_{n+1} = (a * x_n + c) mod m (Numerical Recipes constants)
    const A: u64 = 1664525;
    const C: u64 = 1013904223;
    const M: u64 = 1u64 << 32;

    // First column: user_id in [1, 1000] — deterministic from seed
    let mut x = seed;
    let user_ids: Vec<i64> = (0..count)
        .map(|_| {
            x = (A.wrapping_mul(x).wrapping_add(C)) % M;
            ((x % 1000) + 1) as i64
        })
        .collect();

    // Second column: ts — one event per second, starting from epoch 1700000000
    let timestamps: Vec<i64> = (0..count).map(|i| 1_700_000_000i64 + i as i64).collect();

    // Build a flat StructArray of N rows (the inner content), then
    // wrap it in a single-row ListArray so `UNNEST(generate_events(...))`
    // flattens to N rows.
    let user_id_array = std::sync::Arc::new(Int64Array::from(user_ids));
    let ts_array = std::sync::Arc::new(Int64Array::from(timestamps));
    let struct_array = StructArray::try_new(
        Fields::from(vec![
            Field::new("user_id", DataType::Int64, false),
            Field::new("ts", DataType::Int64, false),
        ]),
        vec![user_id_array, ts_array],
        None,
    )?;
    let struct_field = Field::new(
        "item",
        DataType::Struct(struct_array.fields().clone()),
        false,
    );
    let offsets = OffsetBuffer::<i32>::new(vec![0, count as i32].into());
    let list = ListArray::try_new(
        Arc::new(struct_field),
        offsets,
        Arc::new(struct_array),
        None,
    )?;
    Ok(ColumnarValue::Array(Arc::new(list)))
}

fn extract_i64(cv: &ColumnarValue, name: &str) -> DfResult<i64> {
    let arr = match cv {
        ColumnarValue::Scalar(s) => s.to_array_of_size(1)?,
        ColumnarValue::Array(a) => a.clone(),
    };
    let arr = arr.as_any().downcast_ref::<Int64Array>().ok_or_else(|| {
        datafusion::error::DataFusionError::Plan(format!(
            "generate_series: {} must be Int64, got {:?}",
            name,
            arr.data_type()
        ))
    })?;
    if arr.is_empty() {
        return Err(datafusion::error::DataFusionError::Plan(
            "generate_series: empty argument".into(),
        ));
    }
    Ok(arr.value(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::scalar::ScalarValue;

    #[test]
    fn generate_series_emits_values_in_range() {
        let args = vec![
            ColumnarValue::Scalar(ScalarValue::Int64(Some(1))),
            ColumnarValue::Scalar(ScalarValue::Int64(Some(5))),
        ];
        let result = generate_series_impl(&args).unwrap();
        match result {
            ColumnarValue::Array(a) => {
                let arr = a.as_any().downcast_ref::<ListArray>().unwrap();
                assert_eq!(arr.len(), 1, "ListArray has one row");
                let values = arr.value(0);
                let int_arr = values.as_any().downcast_ref::<Int64Array>().unwrap();
                assert_eq!(int_arr.len(), 5);
                assert_eq!(int_arr.value(0), 1);
                assert_eq!(int_arr.value(4), 5);
            }
            _ => panic!("expected array"),
        }
    }
}

#[cfg(test)]
mod generate_events_tests {
    use super::*;
    use datafusion::scalar::ScalarValue;

    #[test]
    fn generate_events_is_deterministic() {
        let args = vec![
            ColumnarValue::Scalar(ScalarValue::Int64(Some(0))),  // schema (ignored)
            ColumnarValue::Scalar(ScalarValue::Int64(Some(100))),  // count
            ColumnarValue::Scalar(ScalarValue::Int64(Some(42))),  // seed
        ];
        let r1 = generate_events_impl(&args).unwrap();
        let r2 = generate_events_impl(&args).unwrap();
        let a1 = match r1 {
            ColumnarValue::Array(a) => a,
            _ => panic!("expected array"),
        };
        let a2 = match r2 {
            ColumnarValue::Array(a) => a,
            _ => panic!("expected array"),
        };
        // Single-row ListArray (UNNEST expands to `count` rows).
        let l1 = a1.as_any().downcast_ref::<ListArray>().unwrap();
        let l2 = a2.as_any().downcast_ref::<ListArray>().unwrap();
        assert_eq!(l1.len(), 1, "ListArray has one outer row");
        assert_eq!(l1.len(), l2.len());
        let inner1 = l1.value(0);
        let inner2 = l2.value(0);
        let s1 = inner1.as_any().downcast_ref::<StructArray>().unwrap();
        let s2 = inner2.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(s1.len(), 100, "inner StructArray has `count` rows");
        assert_eq!(s1.len(), s2.len());

        // Same seed → same data; verify by checking the first 10 user_ids
        let col1 = s1.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let col2 = s2.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        for i in 0..10 {
            assert_eq!(col1.value(i), col2.value(i), "user_id mismatch at index {}", i);
        }
    }

    #[test]
    fn generate_events_has_user_id_and_ts_columns() {
        let args = vec![
            ColumnarValue::Scalar(ScalarValue::Int64(Some(0))),
            ColumnarValue::Scalar(ScalarValue::Int64(Some(5))),
            ColumnarValue::Scalar(ScalarValue::Int64(Some(1))),
        ];
        let result = generate_events_impl(&args).unwrap();
        let arr = match result {
            ColumnarValue::Array(a) => a,
            _ => panic!("expected array"),
        };
        // Outer ListArray → 1 row, containing a StructArray of 5
        // rows × 2 columns (user_id, ts).
        let list_arr = arr.as_any().downcast_ref::<ListArray>().unwrap();
        assert_eq!(list_arr.len(), 1);
        let inner = list_arr.value(0);
        let struct_arr = inner.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(struct_arr.num_columns(), 2);
        assert_eq!(struct_arr.column(0).data_type(), &DataType::Int64);
        assert_eq!(struct_arr.column(1).data_type(), &DataType::Int64);
        assert_eq!(struct_arr.len(), 5);
    }
}
