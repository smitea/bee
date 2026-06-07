//! `bee-dsl-sql` — Bee SQL DSL。
//!
//! 基于 Apache DataFusion 的 SQL parser / planner,扩展 `EMIT INTO` 与
//! `ASOF JOIN` 等流式语义,编译为 Bee `Dag`。
//!
//! S13 引入 DataFusion:S13 仅做 `parse_sql` + `analyze` 烟测,
//! S14 起实现 `LogicalPlan → Bee Dag` 编译,S15 起实现端到端 executor + CLI。

use datafusion::error::Result as DfResult;

/// 解析一条 SQL 语句,返回 DataFusion 的 Statement AST 列表。
///
/// MVP 范围:SELECT/EMIT INTO 等 DataFusion 标准语法;S14/S15 在此基础上
/// 扩展 ASOF JOIN 与 EMIT INTO 的自定义 Statement 形态。
pub fn parse_sql(source: &str) -> DfResult<Vec<datafusion::sql::parser::Statement>> {
    let stmts: std::collections::VecDeque<_> =
        datafusion::sql::parser::DFParserBuilder::new(source).build()?.parse_statements()?;
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
        // LogicalPlan 的 Display 实现按 tree 形打印,应包含三类节点:
        //   Projection (a + 1 AS b)
        //   Filter     (a > 0)
        //   TableScan  (stream)
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
        // analyze 默认只注册 'stream' 表,引用 'no_such' 应该失败。
        let res = analyze("SELECT x FROM no_such").await;
        assert!(
            res.is_err(),
            "expected error for unknown table, got: {res:?}"
        );
    }
}
