//! `console` sink: writes rows to stdout, one per line, formatted as
//! `col1=val1, col2=val2, ...`. The console sink is built-in
//! (always-on; no feature flag).

use datafusion::arrow::array::*;
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::record_batch::RecordBatch;
use std::io::Write;

/// Print a single RecordBatch to stdout. Each row is one line.
pub fn emit_to_console(batch: &RecordBatch) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for row_idx in 0..batch.num_rows() {
        let mut parts: Vec<String> = Vec::new();
        for (col_idx, field) in batch.schema().fields().iter().enumerate() {
            let col = batch.column(col_idx);
            let value = match field.data_type() {
                DataType::Int64 => {
                    let arr = col.as_any().downcast_ref::<Int64Array>().unwrap();
                    arr.value(row_idx).to_string()
                }
                DataType::Float64 => {
                    let arr = col.as_any().downcast_ref::<Float64Array>().unwrap();
                    arr.value(row_idx).to_string()
                }
                DataType::Utf8 => {
                    let arr = col.as_any().downcast_ref::<StringArray>().unwrap();
                    format!("\"{}\"", arr.value(row_idx))
                }
                DataType::Boolean => {
                    let arr = col.as_any().downcast_ref::<BooleanArray>().unwrap();
                    arr.value(row_idx).to_string()
                }
                _ => format!("<{:?}>", field.data_type()),
            };
            parts.push(format!("{}={}", field.name(), value));
        }
        writeln!(out, "{}", parts.join(", "))?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::datatypes::{Field, Schema};
    use std::sync::Arc;

    #[test]
    fn emit_empty_batch_is_ok() {
        let schema = Arc::new(Schema::empty());
        let batch = RecordBatch::new_empty(schema);
        assert!(emit_to_console(&batch).is_ok());
    }

    #[test]
    fn emit_int64_and_utf8_batch_writes_to_stdout() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec!["alice", "bob"])),
            ],
        )
        .unwrap();
        assert!(emit_to_console(&batch).is_ok());
    }
}
