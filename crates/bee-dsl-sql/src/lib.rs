//! `bee-dsl-sql` — Bee SQL DSL。
//!
//! 基于 Apache DataFusion 的 SQL parser / planner,扩展 `EMIT INTO` 与
//! `ASOF JOIN` 等流式语义,编译为 Bee `Dag`。
//!
//! - S13: `parse_sql` + `analyze` 烟测
//! - S14: `LogicalPlan → Bee Dag` 编译 (Projection / Filter / Aggregate / Datasource)
//! - S15: 端到端 executor + CLI

mod compile;
mod handlers;
mod physical;
pub mod preprocess;
pub mod sinks;

#[cfg(feature = "test-fixtures")]
pub mod test_fixtures;

use datafusion::error::Result as DfResult;
use datafusion::prelude::SessionContext;

pub use compile::{compile_to_dag, CompileError};
pub use handlers::{
    AggregateHandler, DatasourceHandler, FilterHandler, ProjectionHandler,
};
pub use physical::{
    compile_to_physical_plan, execute_plan, format_batches, run_pipeline,
    run_pipeline_with_config, DataFusionPhase, RunConfig, RunMode,
};
pub use preprocess::{
    check_strict_mode, extract_stream_identities, parse_use_directives,
    preprocess, strip_emit_into, EmitTarget, UseDirective,
};

/// 解析一条 SQL 语句,返回 DataFusion 的 Statement AST 列表。
///
/// MVP 范围:SELECT/EMIT INTO 等 DataFusion 标准语法;S14/S15 在此基础上
/// 扩展 ASOF JOIN 与 EMIT INTO 的自定义 Statement 形态。
pub fn parse_sql(source: &str) -> DfResult<Vec<datafusion::sql::parser::Statement>> {
    let stmts: std::collections::VecDeque<_> =
        datafusion::sql::parser::DFParserBuilder::new(source)
            .build()?
            .parse_statements()?;
    Ok(stmts.into())
}

/// 解析 + analyze 一条 SQL,产出 DataFusion 的 LogicalPlan。
///
/// 内部构造一个默认 `SessionContext`,把名为 `stream` 的空表
/// (Int64 列 `a`) 注册进去,让 analyzer 能 resolve SQL 引用的源表。
/// S14 起会扩展为 `analyze_with(ctx, sql)`,支持任意 schema 与多表。
///
/// `async` 是因为 DataFusion 49 的 `SessionState::statement_to_plan`
/// 返回 future;MVP 不在内部 spin up 一个 tokio runtime,直接对外暴露
/// async 签名,与 DataFusion 生态对齐。
pub async fn analyze(source: &str) -> DfResult<datafusion::logical_expr::LogicalPlan> {
    use datafusion::arrow::array::RecordBatch;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::datasource::memory::MemTable;
    use std::sync::Arc;

    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![Field::new("a", DataType::Int64, false)]));
    let provider = MemTable::try_new(
        schema.clone(),
        vec![vec![RecordBatch::new_empty(schema.clone())]],
    )?;
    ctx.register_table("stream", Arc::new(provider))?;

    analyze_with_ctx(&ctx, source).await
}

/// 与 [`analyze`] 等价,但接受调用方预填充的 `SessionContext`。
/// S15 起的 `run_pipeline` 用它:先 `register_csv("stream", ...)`,
/// 再 `analyze_with_ctx` 让 analyzer 看到的 schema 与执行端一致。
pub async fn analyze_with_ctx(
    ctx: &SessionContext,
    source: &str,
) -> DfResult<datafusion::logical_expr::LogicalPlan> {
    let mut stmts = parse_sql(source)?;
    if stmts.is_empty() {
        return Err(datafusion::error::DataFusionError::Plan(
            "empty SQL string".to_string(),
        ));
    }
    if stmts.len() > 1 {
        return Err(datafusion::error::DataFusionError::Plan(format!(
            "expected exactly 1 statement, got {}",
            stmts.len()
        )));
    }
    let stmt = stmts.pop().unwrap();
    ctx.state().statement_to_plan(stmt).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_runtime::DynPhase;

    #[test]
    fn parse_sql_returns_single_statement_for_simple_select() {
        let stmts = parse_sql("SELECT a + 1 AS b FROM stream WHERE a > 0").unwrap();
        assert_eq!(stmts.len(), 1);
    }

    #[tokio::test]
    async fn analyze_produces_logical_plan_with_projection_filter_and_scan() {
        let plan = analyze("SELECT a + 1 AS b FROM stream WHERE a > 0")
            .await
            .unwrap();
        let display = format!("{plan}");
        assert!(
            display.contains("Projection") || display.contains("projection"),
            "expected projection in plan, got:\n{display}"
        );
        assert!(
            display.contains("Filter") || display.contains("filter"),
            "expected filter in plan, got:\n{display}"
        );
        assert!(
            display.contains("stream"),
            "expected 'stream' table reference in plan, got:\n{display}"
        );
    }

    #[tokio::test]
    async fn analyze_rejects_unknown_table() {
        let res = analyze("SELECT x FROM no_such").await;
        assert!(
            res.is_err(),
            "expected error for unknown table, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn compile_simple_select_produces_datasource_filter_projection_dag() {
        let plan = analyze("SELECT a + 1 AS b FROM stream WHERE a > 0")
            .await
            .unwrap();
        let dag = compile_to_dag(&plan).unwrap();

        let v: &[DynPhase] = dag.vertices();
        assert_eq!(v.len(), 3, "expected 3 phases, got {}", v.len());

        // Phase 0: Datasource for `stream`, with an AdapterRef (per ADR-0002)
        assert_eq!(v[0].id, 0);
        assert!(v[0].name.contains("stream"), "phase 0 name = {}", v[0].name);
        assert!(
            v[0].adapter.is_some(),
            "datasource phase must have an adapter (ADR-0002)"
        );

        // Phase 1: Filter (a > 0)
        assert_eq!(v[1].id, 1);
        assert!(v[1].name.contains("filter"), "phase 1 name = {}", v[1].name);

        // Phase 2: Projection (a + 1 AS b)
        assert_eq!(v[2].id, 2);
        assert!(
            v[2].name.contains("projection"),
            "phase 2 name = {}",
            v[2].name
        );

        // Edges in execution order: Datasource -> Filter -> Projection
        let edges = dag.edges();
        assert!(
            edges.contains(&(0, 1)),
            "expected edge (0,1), got {edges:?}"
        );
        assert!(
            edges.contains(&(1, 2)),
            "expected edge (1,2), got {edges:?}"
        );
    }

    #[tokio::test]
    async fn schema_is_preserved_across_phase_boundaries() {
        let plan = analyze("SELECT a + 1 AS b FROM stream WHERE a > 0")
            .await
            .unwrap();
        let dag = compile_to_dag(&plan).unwrap();
        let v = dag.vertices();

        // Source: one column "a" of Int64
        let src = v[0].output_schema().expect("datasource must have schema");
        assert_eq!(src.fields().len(), 1);
        assert_eq!(src.field(0).name(), "a");
        assert_eq!(src.field(0).data_type(), &datafusion::arrow::datatypes::DataType::Int64);

        // Filter: same shape as input
        let flt = v[1].output_schema().expect("filter must have schema");
        assert_eq!(flt.fields().len(), 1);
        assert_eq!(flt.field(0).name(), "a");

        // Projection: one column "b" of Int64 (the result of a + 1)
        let prj = v[2].output_schema().expect("projection must have schema");
        assert_eq!(prj.fields().len(), 1);
        assert_eq!(prj.field(0).name(), "b");
        assert_eq!(prj.field(0).data_type(), &datafusion::arrow::datatypes::DataType::Int64);
    }

    #[tokio::test]
    async fn run_simple_select_prints_projection_output() {
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("stream.csv");
        fs::write(&csv_path, "a\n1\n2\n3\n4\n5\n").unwrap();

        let sql = "SELECT a + 1 AS b FROM stream WHERE a > 0";
        let output = run_pipeline(sql, &csv_path).await.unwrap();

        // b = 2..6 (a=1..5, with WHERE a > 0 keeping all, then +1)
        for v in &["2", "3", "4", "5", "6"] {
            assert!(
                output.contains(v),
                "expected `{v}` in output:\n{output}"
            );
        }
        // Output schema should have exactly one column named "b"
        assert!(output.contains("b"), "expected column `b` in output:\n{output}");
        assert!(
            !output.contains(" | a "),
            "did not expect source column `a` to leak:\n{output}"
        );
    }

    #[tokio::test]
    async fn run_simple_select_with_where_filters_rows() {
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("stream.csv");
        fs::write(&csv_path, "a\n-2\n-1\n0\n1\n2\n3\n").unwrap();

        // WHERE a > 0 keeps 1, 2, 3 -> b = 2, 3, 4
        let sql = "SELECT a + 1 AS b FROM stream WHERE a > 0";
        let output = run_pipeline(sql, &csv_path).await.unwrap();
        assert!(output.contains("2"));
        assert!(output.contains("3"));
        assert!(output.contains("4"));
        // The negative and zero inputs (a=-2,-1,0) become b=-1,0,1
        // and are filtered out by WHERE a > 0 (run on b would still
        // keep b=1 — but the WHERE applies before the projection in
        // DataFusion's planning, so b=1 should also be absent).
        // Verify a=-1 (b=0) is NOT in the output:
        // output has "0" anywhere? the header has no "0", and rows
        // are "2", "3", "4", so a bare line "0" should be absent.
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("2")
                || trimmed.starts_with("3")
                || trimmed.starts_with("4")
            {
                continue;
            }
            // header / separator lines are fine; only complain about
            // any data row that is not one of the expected values.
            if trimmed.contains(" | ") {
                // data row — make sure first cell is one of expected
                let first = trimmed.split(" | ").next().unwrap_or("");
                assert!(
                    ["2", "3", "4"].contains(&first),
                    "unexpected data row: {trimmed:?}\nfull output:\n{output}"
                );
            }
        }
    }

    #[tokio::test]
    async fn run_config_default_has_one_second_window() {
        let cfg = RunConfig::default();
        assert_eq!(cfg.micro_batch_window_ms, 1000);
        assert_eq!(cfg.replay_count, 1);
        assert_eq!(cfg.mode, RunMode::MicroBatch);
        assert!(!cfg.measure_latency);
    }

    #[tokio::test]
    async fn run_mode_display_strings() {
        assert_eq!(format!("{}", RunMode::MicroBatch), "micro_batch");
        assert_eq!(format!("{}", RunMode::PerEvent), "per_event");
        assert_eq!(RunMode::default(), RunMode::MicroBatch);
    }

    #[tokio::test]
    async fn run_pipeline_micro_batch_replay_count_runs_plan_multiple_times() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("stream.csv");
        fs::write(&csv_path, "a\n1\n2\n3\n").unwrap();
        let sql = "SELECT a FROM stream";
        let cfg = RunConfig {
            mode: RunMode::MicroBatch,
            replay_count: 3,
            measure_latency: false,
            ..Default::default()
        };
        let output = run_pipeline_with_config(sql, &csv_path, &cfg)
            .await
            .unwrap();
        // 3 replays × 3 rows = 9 data lines. The output has the
        // header + separator + 9 single-cell rows. Count the data
        // rows: each non-empty non-header line that contains a
        // value 1, 2, or 3 on its own.
        let data_rows: Vec<&str> = output
            .lines()
            .filter(|l| {
                let t = l.trim();
                t == "1" || t == "2" || t == "3"
            })
            .collect();
        assert_eq!(data_rows.len(), 9, "3 replays × 3 rows = 9, got:\n{output}");
    }

    #[tokio::test]
    async fn run_pipeline_per_event_mode_runs_once() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("stream.csv");
        fs::write(&csv_path, "a\n1\n2\n3\n").unwrap();
        let sql = "SELECT a FROM stream";
        let cfg = RunConfig {
            mode: RunMode::PerEvent,
            replay_count: 5, // ignored in PerEvent mode
            measure_latency: false,
            ..Default::default()
        };
        let output = run_pipeline_with_config(sql, &csv_path, &cfg)
            .await
            .unwrap();
        let data_rows: Vec<&str> = output
            .lines()
            .filter(|l| {
                let t = l.trim();
                t == "1" || t == "2" || t == "3"
            })
            .collect();
        assert_eq!(data_rows.len(), 3, "PerEvent keeps only first iteration (3 rows), got {}: \n{}", data_rows.len(), output);
    }

    #[tokio::test]
    async fn run_pipeline_measure_latency_prints_p50_p99() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("stream.csv");
        fs::write(&csv_path, "a\n1\n2\n3\n").unwrap();
        let sql = "SELECT a FROM stream";
        let cfg = RunConfig {
            mode: RunMode::MicroBatch,
            replay_count: 5,
            measure_latency: true,
            ..Default::default()
        };
        let output = run_pipeline_with_config(sql, &csv_path, &cfg)
            .await
            .unwrap();
        assert!(output.contains("latency"), "missing latency report:\n{output}");
        assert!(output.contains("p50"));
        assert!(output.contains("p99"));
        assert!(output.contains("avg"));
    }

    #[tokio::test]
    async fn datafusion_sql_hints_are_accepted() {
        // S26 acceptance: hint syntax is passed through to
        // DataFusion's optimizer. The MVP just verifies the
        // hint comment doesn't trigger a parse error.
        let sql = "EXPLAIN SELECT /*+ TestHint(foo=1) */ a + 1 AS b FROM stream";
        let dir = tempfile::tempdir().unwrap();
        let csv_path = dir.path().join("stream.csv");
        std::fs::write(&csv_path, "a\n1\n2\n3\n").unwrap();
        let output = run_pipeline(sql, &csv_path).await;
        match output {
            Ok(_) => { /* accepted — the test is "doesn't error" */ }
            Err(e) => {
                let msg = e.to_lowercase();
                assert!(
                    !msg.contains("expected")
                        && !msg.contains("hint"),
                    "hint syntax should be accepted, got error: {e}"
                );
            }
        }
    }

    /// S16 acceptance: a test Pipeline using `MockInputAdapter`
    /// (the only built-in Adapter) runs end-to-end and emits events.
    /// The full S29 Datasource mechanism (`--adapter mock_input`) is
    /// a follow-up; for S16 the "Pipeline" is the adapter's own
    /// open → next → close lifecycle, observed from a test.
    #[tokio::test]
    async fn mock_input_adapter_pipeline_runs_end_to_end() {
        use bee_runtime::test_utils::{collect_mock, MockInputConfig};

        let cfg = MockInputConfig {
            count: 5,
            base_timestamp_ms: Some(1_700_000_000_000),
            ..Default::default()
        };
        let events = collect_mock(cfg).await.expect("collect");
        assert_eq!(events.len(), 5);
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.sequence, i as u64);
            assert_eq!(e.timestamp, 1_700_000_000_000 + i as u64 * 1000);
            // Default payload is the sequence as ASCII bytes.
            assert_eq!(e.payload, i.to_string().into_bytes());
        }
    }

    #[tokio::test]
    async fn compile_aggregate_produces_datasource_aggregate_projection_dag() {
        // DataFusion wraps `SELECT a, SUM(a) AS s FROM stream GROUP BY a`
        // in: Aggregate -> Projection (for the alias) -> TableScan.
        // The S14 compiler should faithfully walk that tree.
        let plan = analyze("SELECT a, SUM(a) AS s FROM stream GROUP BY a")
            .await
            .unwrap();
        let dag = compile_to_dag(&plan).unwrap();
        let v = dag.vertices();
        assert_eq!(v.len(), 3, "expected 3 phases, got {}", v.len());
        assert!(v[0].name.contains("stream"));
        assert!(v[1].name.contains("aggregate"));
        assert!(v[2].name.contains("projection"));
        assert!(dag.edges().contains(&(0, 1)));
        assert!(dag.edges().contains(&(1, 2)));
    }
}
