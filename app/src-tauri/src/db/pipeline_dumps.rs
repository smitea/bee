use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineDump {
    pub pipeline_id: i64,
    pub dump_json: String,
    pub created_at: i64,
}

pub fn list_for_pipeline(
    conn: &Connection,
    pipeline_id: i64,
) -> Result<Vec<PipelineDump>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT pipeline_id, dump_json, created_at
             FROM pipeline_dumps WHERE pipeline_id = ?
             ORDER BY created_at DESC",
        )
        .map_err(|e| format!("pipeline_dumps.list_for_pipeline prepare: {e}"))?;
    let rows = stmt
        .query_map(params![pipeline_id], |row| {
            Ok(PipelineDump {
                pipeline_id: row.get(0)?,
                dump_json: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .map_err(|e| format!("pipeline_dumps.list_for_pipeline query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("pipeline_dumps.list_for_pipeline collect: {e}"))
}

pub fn record(
    conn: &Connection,
    pipeline_id: i64,
    dump_json: &str,
) -> Result<PipelineDump, String> {
    let created_at = now_secs();
    conn.execute(
        "INSERT INTO pipeline_dumps (pipeline_id, dump_json, created_at)
         VALUES (?, ?, ?)",
        params![pipeline_id, dump_json, created_at],
    )
    .map_err(|e| format!("pipeline_dumps.record({pipeline_id}): {e}"))?;
    Ok(PipelineDump {
        pipeline_id,
        dump_json: dump_json.to_string(),
        created_at,
    })
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
    fn list_for_pipeline_is_empty_initially() {
        run(|conn| {
            let xs = list_for_pipeline(conn, 1).unwrap();
            assert!(xs.is_empty());
        });
    }

    #[test]
    fn record_then_list_returns_dump_in_descending_created_at() {
        run(|conn| {
            let first = record(conn, 1, r#"{"v":1}"#).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
            let second = record(conn, 1, r#"{"v":2}"#).unwrap();
            let xs = list_for_pipeline(conn, 1).unwrap();
            assert_eq!(xs.len(), 2);
            assert_eq!(xs[0].dump_json, r#"{"v":2}"#);
            assert_eq!(xs[0].created_at, second.created_at);
            assert_eq!(xs[1].dump_json, r#"{"v":1}"#);
            assert_eq!(xs[1].created_at, first.created_at);
        });
    }

    #[test]
    fn list_filters_by_pipeline_id() {
        run(|conn| {
            record(conn, 1, r#"{"v":1}"#).unwrap();
            record(conn, 2, r#"{"v":2}"#).unwrap();
            let xs = list_for_pipeline(conn, 1).unwrap();
            assert_eq!(xs.len(), 1);
            assert_eq!(xs[0].pipeline_id, 1);
        });
    }
}