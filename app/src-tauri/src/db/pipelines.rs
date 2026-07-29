use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineDefinition {
    pub id: i64,
    pub name: String,
    pub dag_json: String,
    pub updated_at: i64,
}

pub fn list(conn: &Connection) -> Result<Vec<PipelineDefinition>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, dag_json, updated_at
             FROM pipeline_definitions
             ORDER BY name ASC",
        )
        .map_err(|e| format!("pipelines.list prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PipelineDefinition {
                id: row.get(0)?,
                name: row.get(1)?,
                dag_json: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .map_err(|e| format!("pipelines.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("pipelines.list collect: {e}"))
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<PipelineDefinition>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, dag_json, updated_at
             FROM pipeline_definitions
             WHERE id = ?",
        )
        .map_err(|e| format!("pipelines.get prepare: {e}"))?;
    let mut rows = stmt
        .query_map(params![id], |row| {
            Ok(PipelineDefinition {
                id: row.get(0)?,
                name: row.get(1)?,
                dag_json: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .map_err(|e| format!("pipelines.get query: {e}"))?;
    match rows.next() {
        Some(Ok(p)) => Ok(Some(p)),
        Some(Err(e)) => Err(format!("pipelines.get next: {e}")),
        None => Ok(None),
    }
}

pub fn create(conn: &Connection, name: &str, dag_json: &str) -> Result<PipelineDefinition, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("pipelines.create: name must not be empty".into());
    }
    let now = now_secs();
    let outcome = conn.execute(
        "INSERT INTO pipeline_definitions (name, dag_json, updated_at)
         VALUES (?, ?, ?)",
        params![trimmed, dag_json, now],
    );
    match outcome {
        Ok(_) => {}
        Err(e) => {
            let msg = format!("{e}");
            if msg.contains("UNIQUE constraint failed: pipeline_definitions.name") {
                return Err(format!("pipelines.create: name '{trimmed}' already exists"));
            }
            return Err(format!("pipelines.create({trimmed}): {e}"));
        }
    }
    let id = conn.last_insert_rowid();
    Ok(PipelineDefinition {
        id,
        name: trimmed.to_string(),
        dag_json: dag_json.to_string(),
        updated_at: now,
    })
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM pipeline_definitions WHERE id = ?",
        params![id],
    )
    .map_err(|e| format!("pipelines.delete({id}): {e}"))
    .map(|_| ())
}

pub fn name_taken(conn: &Connection, name: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pipeline_definitions WHERE name = ?",
            params![name],
            |row| row.get(0),
        )
        .map_err(|e| format!("pipelines.name_taken: {e}"))?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Connection)>(f: F) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        let conn = db.lock().unwrap();
        f(&conn);
    }

    #[test]
    fn create_persists_and_returns_assigned_id() {
        run(|conn| {
            let p = create(conn, "alpha", r#"{"k":1}"#).unwrap();
            assert!(p.id > 0);
            assert_eq!(p.name, "alpha");
            assert_eq!(p.dag_json, r#"{"k":1}"#);
            assert!(p.updated_at > 0);
        });
    }

    #[test]
    fn get_returns_none_when_id_missing() {
        run(|conn| {
            assert!(get(conn, 999).unwrap().is_none());
        });
    }

    #[test]
    fn get_round_trips_inserted_row() {
        run(|conn| {
            let p = create(conn, "alpha", r#"{"k":1}"#).unwrap();
            let fetched = get(conn, p.id).unwrap().unwrap();
            assert_eq!(fetched, p);
        });
    }

    #[test]
    fn list_returns_empty_when_no_rows() {
        run(|conn| {
            assert!(list(conn).unwrap().is_empty());
        });
    }

    #[test]
    fn list_returns_rows_sorted_by_name() {
        run(|conn| {
            let b = create(conn, "bravo", "{}").unwrap();
            let a = create(conn, "alpha", "{}").unwrap();
            let c = create(conn, "charlie", "{}").unwrap();
            let all = list(conn).unwrap();
            assert_eq!(
                all.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
                vec!["alpha".to_string(), "bravo".to_string(), "charlie".to_string()]
            );
            assert_eq!(all, vec![a, b, c]);
        });
    }

    #[test]
    fn create_rejects_empty_name() {
        run(|conn| {
            assert!(create(conn, "   ", "{}").is_err());
            assert!(create(conn, "", "{}").is_err());
        });
    }

    #[test]
    fn name_uniqueness_is_enforced() {
        run(|conn| {
            create(conn, "alpha", "{}").unwrap();
            let err = create(conn, "alpha", "{}").unwrap_err();
            assert!(err.contains("already exists"), "got: {err}");
            assert!(name_taken(conn, "alpha").unwrap());
            assert!(!name_taken(conn, "beta").unwrap());
        });
    }

    #[test]
    fn delete_removes_row_and_list_omits_it() {
        run(|conn| {
            let a = create(conn, "alpha", "{}").unwrap();
            let b = create(conn, "bravo", "{}").unwrap();
            delete(conn, a.id).unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all, vec![b]);
            assert!(get(conn, a.id).unwrap().is_none());
        });
    }

    #[test]
    fn delete_unknown_id_is_noop_not_error() {
        run(|conn| {
            delete(conn, 999).unwrap();
        });
    }
}
