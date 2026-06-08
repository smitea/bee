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

/// S26 execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunMode {
    /// S26 default: micro-batch. The plan runs in `replay_count`
    /// iterations, each iteration is one batch. Production
    /// (S18+ cross-Pipeline edges, S17 Producer/Subscriber) drives
    /// the loop on a timer keyed by `micro_batch_window_ms`.
    #[default]
    MicroBatch,
    /// Per-event mode: bypass batching for ultra-low latency. The
    /// MVP runs the plan once over the whole input (CSV is the
    /// batch); the production path (S18+ with the Adapter stream)
    /// would invoke the plan per event.
    PerEvent,
}

impl std::fmt::Display for RunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunMode::MicroBatch => f.write_str("micro_batch"),
            RunMode::PerEvent => f.write_str("per_event"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunConfig {
    /// S26 execution mode.
    pub mode: RunMode,
    /// Micro-batch window in milliseconds (ADR-0006 default = 1000).
    /// Production tightens to 10 for quant scenarios.
    pub micro_batch_window_ms: u64,
    /// Number of times the input is "replayed" to simulate a
    /// continuous source. Default 1. Set > 1 to drive the
    /// micro-batch loop; 0 = run continuously (until cancelled;
    /// MVP treats 0 as 1).
    pub replay_count: u32,
    /// S26: when true, the runner measures per-iteration latency
    /// and prints p50/p99 alongside the result. Driven by
    /// `bee run --measure`.
    pub measure_latency: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            mode: RunMode::default(),
            micro_batch_window_ms: 1000,
            replay_count: 1,
            measure_latency: false,
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
/// S26: respects `RunConfig.mode` and `RunConfig.replay_count`.
/// MicroBatch mode runs the plan `replay_count` times (each
/// iteration is one batch — production wires a timer keyed by
/// `micro_batch_window_ms`). PerEvent mode runs the plan once
/// (CSV is the whole input; production would invoke per event).
///
/// S16 redo: Bee core is business-agnostic. The MVP CLI is a thin
/// DataFusion wrapper; concrete Datasource Adapters (Binance,
/// CoinGecko, InfluxDB, etc.) ship as **external plugins** in
/// their own crates and are not part of the `bee run` path.
pub async fn run_pipeline(sql: &str, csv_path: &Path) -> Result<String, String> {
    run_pipeline_with_config(sql, csv_path, &RunConfig::default()).await
}

/// S26: variant of `run_pipeline` that takes an explicit config.
pub async fn run_pipeline_with_config(
    sql: &str,
    csv_path: &Path,
    config: &RunConfig,
) -> Result<String, String> {
    // S41 (9a): detect `EMIT INTO <target>` prefix and strip it
    // before the SQL reaches DataFusion (DataFusion's parser
    // doesn't recognize the keyword). The remaining SQL is a plain
    // SELECT that DataFusion can plan + execute.
    let (emit_target, stripped_sql) = crate::preprocess::strip_emit_into(sql);

    let plan = compile_to_physical_plan(&stripped_sql, csv_path)
        .await
        .map_err(|e| format!("compile: {e}"))?;
    // S26: per-iteration latency tracker.
    let mut latencies: Vec<std::time::Duration> = Vec::new();
    let iterations = if config.replay_count == 0 {
        1
    } else {
        config.replay_count as usize
    };

    let mut all_batches: Vec<RecordBatch> = Vec::new();
    for i in 0..iterations {
        let start = std::time::Instant::now();
        let batches = execute_plan(&plan)
            .await
            .map_err(|e| format!("execute: {e}"))?;
        if config.measure_latency {
            latencies.push(start.elapsed());
        }
        // In MicroBatch mode we accumulate across iterations; in
        // PerEvent mode we only keep the first (the per-event path
        // is conceptually one event at a time — for the MVP CSV
        // input it's one shot).
        if matches!(config.mode, RunMode::MicroBatch) || i == 0 {
            all_batches.extend(batches);
        }
    }

    // S41 (9a): if the user asked for `EMIT INTO console`, dispatch
    // the resulting batches to the console sink (one row per line,
    // `col=val, ...`) instead of formatting as a Markdown-style
    // table. The string we return is a short summary so the CLI's
    // `Ok(s)` return value is still meaningful.
    if let Some(crate::preprocess::EmitTarget::Console) = emit_target {
        let mut total_rows: usize = 0;
        for batch in &all_batches {
            crate::sinks::console::emit_to_console(batch)
                .map_err(|e| format!("console sink: {e}"))?;
            total_rows += batch.num_rows();
        }
        return Ok(format!("(emitted {total_rows} row(s) to console)\n"));
    }

    let mut out = format_batches(&all_batches);
    if config.measure_latency {
        out.push_str(&format_latency_report(&latencies));
    }
    Ok(out)
}

fn format_latency_report(latencies: &[std::time::Duration]) -> String {
    if latencies.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<std::time::Duration> = latencies.to_vec();
    sorted.sort();
    let p = |q: f64| -> std::time::Duration {
        let idx = ((sorted.len() as f64 * q).ceil() as usize).saturating_sub(1);
        sorted[idx.min(sorted.len() - 1)]
    };
    let sum: std::time::Duration = sorted.iter().sum();
    let avg = sum / sorted.len() as u32;
    let p50 = p(0.5);
    let p99 = p(0.99);
    format!(
        "\n--- latency (n={}, mode=micro_batch) ---\n\
         avg: {avg:?}\n\
         p50: {p50:?}\n\
         p99: {p99:?}\n",
        sorted.len()
    )
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
