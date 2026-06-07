//! Bee Phase Handlers derived from DataFusion LogicalPlan operators.
//!
//! Each handler is a stub for S14: it stores the operator metadata
//! (expressions, schema, table name) and forwards input → output without
//! transformation. S15 will replace the bodies with real DataFusion
//! execution.
//!
//! ## S14 scope
//! - `DatasourceHandler` — wraps a `TableScan` (per ADR-0002, a Phase
//!   with an `AdapterRef`)
//! - `FilterHandler` — wraps a `Filter` (predicate only; no projection)
//! - `ProjectionHandler` — wraps a `Projection` (expr list)
//! - `AggregateHandler` — wraps a basic `Aggregate` (group_by + aggr_exprs)
//!
//! All four use `Input = Output = RecordBatch` so the type-erased runtime
//! pipeline is uniform. S15 will plug a real `datafusion::physical_plan`
//! into the body.

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use bee_runtime::{Handler, RuntimeError};

/// Marker for phases that read from an external table (Datasource = Phase
/// with an `AdapterRef`, per ADR-0002). The `BeeHost` is responsible for
/// assigning the actual `AdapterRef` value at deploy time; for S14 the
/// compile path uses a stable per-table hash so tests can assert on it.
pub struct DatasourceHandler {
    pub table_name: String,
    pub schema: SchemaRef,
    pub fetch: Option<usize>,
}

impl DatasourceHandler {
    pub fn new(table_name: String, schema: SchemaRef, fetch: Option<usize>) -> Self {
        Self {
            table_name,
            schema,
            fetch,
        }
    }
}

impl Handler for DatasourceHandler {
    type Input = RecordBatch;
    type Output = RecordBatch;

    fn handle(
        &mut self,
        input: RecordBatch,
    ) -> impl std::future::Future<Output = Result<Option<RecordBatch>, RuntimeError>> + Send {
        async move { Ok(Some(input)) }
    }

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}

pub struct FilterHandler {
    pub predicate: String,
    pub schema: SchemaRef,
}

impl FilterHandler {
    pub fn new(predicate: String, schema: SchemaRef) -> Self {
        Self { predicate, schema }
    }
}

impl Handler for FilterHandler {
    type Input = RecordBatch;
    type Output = RecordBatch;

    fn handle(
        &mut self,
        input: RecordBatch,
    ) -> impl std::future::Future<Output = Result<Option<RecordBatch>, RuntimeError>> + Send {
        async move { Ok(Some(input)) }
    }

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}

pub struct ProjectionHandler {
    pub exprs: Vec<String>,
    pub schema: SchemaRef,
}

impl ProjectionHandler {
    pub fn new(exprs: Vec<String>, schema: SchemaRef) -> Self {
        Self { exprs, schema }
    }
}

impl Handler for ProjectionHandler {
    type Input = RecordBatch;
    type Output = RecordBatch;

    fn handle(
        &mut self,
        input: RecordBatch,
    ) -> impl std::future::Future<Output = Result<Option<RecordBatch>, RuntimeError>> + Send {
        async move { Ok(Some(input)) }
    }

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}

/// Basic `Aggregate` handler: stores the group-by and aggregate expressions
/// along with the post-aggregation schema. S15 will run the actual
/// aggregation via a DataFusion physical plan; S14 only captures the shape.
pub struct AggregateHandler {
    pub group_by: Vec<String>,
    pub aggr_exprs: Vec<String>,
    pub schema: SchemaRef,
}

impl AggregateHandler {
    pub fn new(
        group_by: Vec<String>,
        aggr_exprs: Vec<String>,
        schema: SchemaRef,
    ) -> Self {
        Self {
            group_by,
            aggr_exprs,
            schema,
        }
    }
}

impl Handler for AggregateHandler {
    type Input = RecordBatch;
    type Output = RecordBatch;

    fn handle(
        &mut self,
        input: RecordBatch,
    ) -> impl std::future::Future<Output = Result<Option<RecordBatch>, RuntimeError>> + Send {
        async move { Ok(Some(input)) }
    }

    fn finish(self) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send {
        async move { Ok(()) }
    }
}

/// Stable AdapterRef derived from a table name (FNV-1a 32-bit). Used by
/// the S14 compile path so the test can assert the datasource has an
/// adapter without needing a Plugin Manager. S15+ swaps this for the
/// real Plugin Manager lookup.
pub fn adapter_ref_for_table(table: &str) -> bee_runtime::AdapterRef {
    let mut h: u32 = 0x811c9dc5;
    for b in table.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    bee_runtime::AdapterRef(h)
}

/// Convenience: clone a DFSchema's underlying Arrow Schema into an Arc.
pub(crate) fn df_schema_to_arrow(
    df_schema: &datafusion::common::DFSchema,
) -> SchemaRef {
    df_schema.inner().clone()
}
