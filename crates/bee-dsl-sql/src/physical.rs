//! `DataFusionPhase` Handler + end-to-end SQL pipeline runner (S15).
//!
//! Wraps a `datafusion::physical_plan::ExecutionPlan` and exposes the
//! Bee `Handler` trait, and provides `run_pipeline` — the end-to-end
//! helper the `bee run <sql>` CLI uses to take a SQL string + a CSV
//! path, produce an `ExecutionPlan`, execute it, and format the
//! results.
//!
//! ## S15 scope (MVP)
//! - Input is a single CSV file registered as the SQL's FROM clause
//!   table (the spec calls this a "mock stream"). The CSV is read once
//!   per `run_pipeline` call.
//! - The micro-batch window is exposed in the API as
//!   `RunConfig { micro_batch_window_ms }` (default 1000ms per ADR-0006)
//!   but the MVP runner executes the plan once and exits. Wiring the
//!   timer-driven loop is a follow-up gated on a real Adapter feeding
//!   events into the input channel (S16+).
//! - `DataFusionPhase::handle` runs the plan and returns the first
//!   output `RecordBatch`. The Handler is type-erased for future use
//!   inside a multi-phase Bee Pipeline; the MVP CLI does not exercise
//!   the runtime path.

use std::path::Path;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use bee_runtime::{Handler, RuntimeError};
use datafusion::error::Result as DfResult;
use datafusion::execution::TaskContext;
use datafusion::physical_plan::{collect, ExecutionPlan};
use datafusion::prelude::{CsvReadOptions, SessionContext};

#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Micro-batch window in milliseconds (ADR-0006 default = 1000).
    /// S15 MVP does not drive a timer loop; this is a config field
    /// reserved for the S16+ micro-batch executor.
    pub micro_batch_window_ms: u64,
    /// Number of times the input stream is "replayed" to simulate a
    /// continuous source. The MVP CLI replays exactly once.
    pub replay_count: u32,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            micro_batch_window_ms: 1000,
            replay_count: 1,
        }
    }
}

/// Compile a SQL string into a DataFusion `ExecutionPlan`, with the
/// input source registered as a CSV file at `csv_path` under the table
/// name `stream` (the S13+ default; tests that use a different name
/// should call `analyze_with` + `register_csv` themselves).
pub async fn compile_to_physical_plan(
    sql: &str,
    csv_path: &Path,
) -> DfResult<Arc<dyn ExecutionPlan>> {
    let ctx = SessionContext::new();
    ctx.register_csv(
        "stream",
        csv_path.to_str().ok_or_else(|| {
            datafusion::error::DataFusionError::Plan(format!(
                "csv path {csv_path:?} is not valid UTF-8"
            ))
        })?,
        CsvReadOptions::default(),
    )
    .await?;

    let logical = super::analyze_with_ctx(&ctx, sql).await?;
    ctx.state().create_physical_plan(&logical).await
}

/// Execute a physical plan and collect all output `RecordBatch`es into
/// memory. The MVP runner materializes the result; a streaming
/// consumer is a follow-up.
pub async fn execute_plan(plan: &Arc<dyn ExecutionPlan>) -> DfResult<Vec<RecordBatch>> {
    collect(plan.clone(), Arc::new(TaskContext::default())).await
}

/// Format a sequence of `RecordBatch`es as a Markdown-style table for
/// CLI printing. Used by `run_pipeline`'s string output and (via the
/// `bee run` CLI) printed to stdout.
pub fn format_batches(batches: &[RecordBatch]) -> String {
    if batches.is_empty() {
        return String::from("(no rows)\n");
    }
    let schema: SchemaRef = batches[0].schema();
    let mut out = String::new();
    // header
    let header: Vec<String> = schema
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    out.push_str(&header.join(" | "));
    out.push('\n');
    out.push_str(
        &header
            .iter()
            .map(|h| "-".repeat(h.len()))
            .collect::<Vec<_>>()
            .join("-+-"),
    );
    out.push('\n');
    // rows
    for batch in batches {
        for row in 0..batch.num_rows() {
            let cells: Vec<String> = (0..batch.num_columns())
                .map(|col| {
                    let arr = batch.column(col);
                    if arr.is_null(row) {
                        return "NULL".to_string();
                    }
                    arrow_array_value_to_string(arr, row)
                })
                .collect();
            out.push_str(&cells.join(" | "));
            out.push('\n');
        }
    }
    out
}

fn arrow_array_value_to_string(
    arr: &dyn arrow_array::Array,
    row: usize,
) -> String {
    use arrow_array::{BooleanArray, Float64Array, Int64Array, StringArray};
    if let Some(a) = arr.as_any().downcast_ref::<Int64Array>() {
        a.value(row).to_string()
    } else if let Some(a) = arr.as_any().downcast_ref::<Float64Array>() {
        a.value(row).to_string()
    } else if let Some(a) = arr.as_any().downcast_ref::<StringArray>() {
        a.value(row).to_string()
    } else if let Some(a) = arr.as_any().downcast_ref::<BooleanArray>() {
        a.value(row).to_string()
    } else {
        format!("<{:?}>", arr.data_type())
    }
}

/// End-to-end: SQL + CSV → formatted string output.
///
/// This is the function the `bee run` CLI calls. It uses
/// `RunConfig::default()` (1s micro-batch window, 1 replay).
///
/// S16 redo: Bee core is business-agnostic. The MVP CLI is a thin
/// DataFusion wrapper; concrete Datasource Adapters (Binance,
/// CoinGecko, InfluxDB, etc.) ship as **external plugins** in
/// their own crates and are not part of the `bee run` path.
pub async fn run_pipeline(sql: &str, csv_path: &Path) -> Result<String, String> {
    let _ = RunConfig::default();
    let plan = compile_to_physical_plan(sql, csv_path)
        .await
        .map_err(|e| format!("compile: {e}"))?;
    let batches = execute_plan(&plan).await.map_err(|e| format!("execute: {e}"))?;
    Ok(format_batches(&batches))
}

/// `Bee` Handler that wraps a DataFusion `ExecutionPlan`.
///
/// `Input = Output = RecordBatch`. The MVP behavior is to ignore the
/// incoming `RecordBatch` and execute the wrapped plan against its
/// registered source (the CSV / Datasource registered at compile time).
/// S16+ will replace this body with the per-event micro-batch loop
/// that mixes the input stream with the registered source.
pub struct DataFusionPhase {
    plan: Arc<dyn ExecutionPlan>,
    schema: SchemaRef,
}

impl DataFusionPhase {
    pub fn new(plan: Arc<dyn ExecutionPlan>) -> Self {
        let schema = plan.schema();
        Self { plan, schema }
    }

    pub fn output_schema(&self) -> SchemaRef {
        self.schema.clone()
    }
}

impl Handler for DataFusionPhase {
    type Input = RecordBatch;
    type Output = RecordBatch;

    fn handle(
        &mut self,
        _input: RecordBatch,
    ) -> impl std::future::Future<Output = Result<Option<RecordBatch>, RuntimeError>> + Send {
        let plan = self.plan.clone();
        async move {
            let batches = collect(plan, Arc::new(TaskContext::default()))
                .await
                .map_err(|e| RuntimeError::Handler(format!("datafusion: {e}")))?;
            // Concatenate all output batches into a single one for
            // downstream Phases. For the MVP CLI we don't go through
            // this Handler, so the simple `first batch` fallback is
            // fine.
            Ok(batches.into_iter().next())
        }
    }

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}
