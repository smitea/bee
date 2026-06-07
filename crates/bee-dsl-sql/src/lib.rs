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

use datafusion::error::Result as DfResult;

pub use compile::{compile_to_dag, CompileError};
pub use handlers::{
    AggregateHandler, DatasourceHandler, FilterHandler, ProjectionHandler,
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

    let ctx = datafusion::prelude::SessionContext::new();
    let schema = Arc::new(Schema::new(vec![Field::new("a", DataType::Int64, false)]));
    let provider = MemTable::try_new(
        schema.clone(),
        vec![vec![RecordBatch::new_empty(schema.clone())]],
    )?;
    ctx.register_table("stream", Arc::new(provider))?;

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
