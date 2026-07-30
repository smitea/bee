use rusqlite::{params, Connection, OptionalExtension};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dashboard {
    pub application_id: i64,
    pub layout_json: String,
    pub updated_at: i64,
}

pub fn get(conn: &Connection, application_id: i64) -> Result<Option<Dashboard>, String> {
    let row = conn
        .query_row(
            "SELECT application_id, layout_json, updated_at
             FROM dashboards WHERE application_id = ?",
            params![application_id],
            |row| {
                Ok(Dashboard {
                    application_id: row.get(0)?,
                    layout_json: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("dashboards.get({application_id}): {e}"))?;
    Ok(row)
}

pub fn upsert(conn: &Connection, application_id: i64, layout_json: &str) -> Result<Dashboard, String> {
    let now = now_secs();
    conn.execute(
        "INSERT INTO dashboards (application_id, layout_json, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(application_id) DO UPDATE SET
            layout_json = excluded.layout_json,
            updated_at = excluded.updated_at",
        params![application_id, layout_json, now],
    )
    .map_err(|e| format!("dashboards.upsert({application_id}): {e}"))?;
    Ok(Dashboard {
        application_id,
        layout_json: layout_json.to_string(),
        updated_at: now,
    })
}

pub fn delete(conn: &Connection, application_id: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM dashboards WHERE application_id = ?",
        params![application_id],
    )
    .map_err(|e| format!("dashboards.delete({application_id}): {e}"))
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::applications;
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
    fn get_returns_none_when_unset() {
        run(|conn| {
            let app = applications::create(conn, "alpha").unwrap();
            assert!(get(conn, app.id).unwrap().is_none());
        });
    }

    #[test]
    fn upsert_then_get_round_trips_layout_json() {
        run(|conn| {
            let app = applications::create(conn, "alpha").unwrap();
            let saved = upsert(conn, app.id, r#"{"panels":[]}"#).unwrap();
            assert_eq!(saved.application_id, app.id);
            assert_eq!(saved.layout_json, r#"{"panels":[]}"#);
            let fetched = get(conn, app.id).unwrap().unwrap();
            assert_eq!(fetched.layout_json, r#"{"panels":[]}"#);
        });
    }

    #[test]
    fn upsert_overwrites_previous_layout() {
        run(|conn| {
            let app = applications::create(conn, "alpha").unwrap();
            upsert(conn, app.id, r#"{"v":1}"#).unwrap();
            let updated = upsert(conn, app.id, r#"{"v":2}"#).unwrap();
            assert!(updated.updated_at > 0);
            let fetched = get(conn, app.id).unwrap().unwrap();
            assert_eq!(fetched.layout_json, r#"{"v":2}"#);
        });
    }

    #[test]
    fn delete_removes_saved_layout() {
        run(|conn| {
            let app = applications::create(conn, "alpha").unwrap();
            upsert(conn, app.id, r#"{"v":1}"#).unwrap();
            delete(conn, app.id).unwrap();
            assert!(get(conn, app.id).unwrap().is_none());
        });
    }

    #[test]
    fn delete_unknown_id_is_noop() {
        run(|conn| {
            delete(conn, 9999).unwrap();
        });
    }
}
