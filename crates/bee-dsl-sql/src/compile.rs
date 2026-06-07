//! `LogicalPlan → Dag` compiler (S14).
//!
//! Walks a DataFusion `LogicalPlan` tree and produces a Bee `Dag` whose
//! phases are typed Bee Handlers (Projection / Filter / Aggregate /
//! Datasource). Phase IDs are assigned in topological order: leaves
//! first, root last. Edges go from each operator's input to the operator.
//!
//! ## Supported operators (S14 MVP)
//! - `TableScan` → `DatasourceHandler` (with `AdapterRef` per ADR-0002)
//! - `Filter` → `FilterHandler`
//! - `Projection` → `ProjectionHandler`
//! - `Aggregate` → `AggregateHandler`
//!
//! Anything else returns `CompileError::Unsupported`.

use arrow_schema::SchemaRef;
use bee_runtime::{Dag, DynPhase};
use datafusion::logical_expr::{Expr, LogicalPlan};

use crate::handlers::{
    adapter_ref_for_table, df_schema_to_arrow, AggregateHandler, DatasourceHandler,
    FilterHandler, ProjectionHandler,
};

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("unsupported logical plan operator: {0}")]
    Unsupported(String),
    #[error("topology error: {0}")]
    Topology(#[from] bee_runtime::RuntimeError),
}

/// Compile a DataFusion `LogicalPlan` into a Bee `Dag`.
///
/// Phase IDs are assigned in tree-walk order (leaves first).
/// The output `Dag` has the same shape as a hand-built `Pipeline` from
/// the deployer: each operator becomes a `DynPhase`, and each input edge
/// becomes a `Dag` edge.
pub fn compile_to_dag(plan: &LogicalPlan) -> Result<Dag, CompileError> {
    let mut dag = Dag::new();
    let mut next_id: u32 = 0;
    compile_node(plan, &mut dag, &mut next_id)?;
    Ok(dag)
}

fn compile_node(
    plan: &LogicalPlan,
    dag: &mut Dag,
    next_id: &mut u32,
) -> Result<u32, CompileError> {
    match plan {
        LogicalPlan::TableScan(scan) => {
            let id = take_id(next_id);
            let table = scan.table_name.to_string();
            let schema: SchemaRef = df_schema_to_arrow(scan.projected_schema.as_ref());
            let phase = DatasourceHandler::new(table.clone(), schema.clone(), scan.fetch);
            let adapter = adapter_ref_for_table(&table);
            let dyn_phase = DynPhase::new(id, format!("datasource:{table}"), phase)
                .with_adapter(adapter)
                .with_output_schema(schema);
            dag.add_phase(dyn_phase);
            Ok(id)
        }
        LogicalPlan::Filter(filter) => {
            let input_id = compile_node(&filter.input, dag, next_id)?;
            let id = take_id(next_id);
            let schema: SchemaRef =
                df_schema_to_arrow(filter.input.schema().as_ref());
            let predicate = expr_to_string(&filter.predicate);
            let phase = FilterHandler::new(predicate, schema.clone());
            let dyn_phase =
                DynPhase::new(id, format!("filter:{}", input_id), phase)
                    .with_output_schema(schema);
            dag.add_phase(dyn_phase);
            dag.add_edge(input_id, id)?;
            Ok(id)
        }
        LogicalPlan::Projection(proj) => {
            let input_id = compile_node(&proj.input, dag, next_id)?;
            let id = take_id(next_id);
            let schema: SchemaRef = df_schema_to_arrow(proj.schema.as_ref());
            let exprs: Vec<String> = proj.expr.iter().map(expr_to_string).collect();
            let phase = ProjectionHandler::new(exprs, schema.clone());
            let dyn_phase =
                DynPhase::new(id, format!("projection:{}", input_id), phase)
                    .with_output_schema(schema);
            dag.add_phase(dyn_phase);
            dag.add_edge(input_id, id)?;
            Ok(id)
        }
        LogicalPlan::Aggregate(agg) => {
            let input_id = compile_node(&agg.input, dag, next_id)?;
            let id = take_id(next_id);
            let schema: SchemaRef = df_schema_to_arrow(agg.schema.as_ref());
            let group_by: Vec<String> = agg.group_expr.iter().map(expr_to_string).collect();
            let aggr_exprs: Vec<String> = agg.aggr_expr.iter().map(expr_to_string).collect();
            let phase = AggregateHandler::new(group_by, aggr_exprs, schema.clone());
            let dyn_phase =
                DynPhase::new(id, format!("aggregate:{}", input_id), phase)
                    .with_output_schema(schema);
            dag.add_phase(dyn_phase);
            dag.add_edge(input_id, id)?;
            Ok(id)
        }
        other => Err(CompileError::Unsupported(format!("{other:?}"))),
    }
}

fn take_id(next_id: &mut u32) -> u32 {
    let id = *next_id;
    *next_id += 1;
    id
}

fn expr_to_string(expr: &Expr) -> String {
    // DataFusion 49: Expr implements Display, so format! works.
    format!("{expr}")
}
