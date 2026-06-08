//! Test fixture functions, gated behind the `test-fixtures` feature.
//!
//! These functions are intended for demos and tests only; they produce
//! deterministic data streams without external services.

#![cfg(feature = "test-fixtures")]

use datafusion::arrow::array::{Array, Int64Array};
use datafusion::common::Result as DfResult;
use datafusion::logical_expr::ColumnarValue;

/// `generate_series(start, end) -> Stream<i64>`: emits one event per
/// integer in `[start, end]`. Returns a single Int64Array containing
/// all values.
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
    let count = (end - start + 1) as usize;
    let values: Vec<i64> = (start..=end).collect();
    let _ = count;
    let array = std::sync::Arc::new(Int64Array::from(values));
    Ok(ColumnarValue::Array(array))
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
                let arr = a.as_any().downcast_ref::<Int64Array>().unwrap();
                assert_eq!(arr.len(), 5);
                assert_eq!(arr.value(0), 1);
                assert_eq!(arr.value(4), 5);
            }
            _ => panic!("expected array"),
        }
    }
}
