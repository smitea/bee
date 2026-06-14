//! S33.5.3: extract a phase DAG from a SQL
//! text. MVP: every top-level `SELECT` is a
//! Phase; no inter-phase dependencies (the
//! `dependencies` vec is always empty). A
//! S33.5.x will add `WITH` chain / multi-CTE
//! support and full topological analysis.

use sha2::{Digest, Sha256};

/// One phase in a pipeline DAG. A phase is
/// a single top-level `SELECT` (a row-
/// producing query). Phases are
/// 1-indexed; `phase_id` matches the order
/// in the SQL text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    pub phase_id: u32,
    /// The SQL text for this phase. The MVP
    /// stores a placeholder string; a S33.5.x
    /// will extract the actual SELECT body
    /// from the AST and store it here.
    pub sql: String,
}

/// A pipeline DAG extracted from a SQL text.
/// `phases[i]` has `phase_id = (i + 1)`.
/// `dependencies` is a list of
/// `(phase_id, depends_on_phase_id)` pairs.
/// The MVP always returns an empty
/// `dependencies` vec (phases are treated as
/// independent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseDag {
    pub phases: Vec<Phase>,
    pub dependencies: Vec<(u32, u32)>,
    /// sha256 of the original SQL text.
    pub dag_hash: String,
}

/// Walk the parsed SQL and identify every
/// top-level `SELECT`. Each becomes a Phase.
/// `Statement::Query(_)` matches a top-level
/// `SELECT`; other `Statement` variants
/// (`SetVariable`, `CreateTable`, etc.) are
/// not phases and are skipped.
pub fn extract_phase_dag(
    sql_text: &str,
) -> Result<PhaseDag, String> {
    let stmts = crate::parse_sql(sql_text)
        .map_err(|e| format!("dag: parse failed: {e}"))?;
    let mut phases = Vec::new();
    let mut next_id = 1u32;
    for stmt in stmts {
        // datafusion's `Statement` enum wraps a
        // `Box<sqlparser::ast::Statement>`. We
        // match the inner `Query` variant to
        // identify a top-level `SELECT`.
        // (Other variants like `SetVariable`
        // are not phases.)
        if let datafusion::sql::parser::Statement::Statement(
            inner,
        ) = &stmt
        {
            // `inner` is `&Box<sqlparser::ast::Statement>`.
            // Deref to the inner Statement.
            let inner_ref: &datafusion::sql::sqlparser::ast::Statement =
                &**inner;
            if matches!(
                inner_ref,
                datafusion::sql::sqlparser::ast::Statement::Query(_)
            ) {
                phases.push(Phase {
                    phase_id: next_id,
                    sql: format!(
                        "<phase {}: parsed query>",
                        next_id
                    ),
                });
                next_id += 1;
            }
        }
    }
    if phases.is_empty() {
        return Err(
            "dag: no SELECT statements found".to_string()
        );
    }
    let dag_hash = {
        let mut h = Sha256::new();
        h.update(sql_text.as_bytes());
        format!("{:x}", h.finalize())
    };
    Ok(PhaseDag {
        phases,
        dependencies: Vec::new(),
        dag_hash,
    })
}
