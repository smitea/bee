use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub display_order: i64,
    pub created_at: i64,
}

pub fn list(conn: &Connection) -> Result<Vec<Application>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, enabled, display_order, created_at
             FROM applications
             ORDER BY display_order ASC, id ASC",
        )
        .map_err(|e| format!("applications.list prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Application {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                display_order: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| format!("applications.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("applications.list collect: {e}"))
}

pub fn next_display_order(conn: &Connection) -> Result<i64, String> {
    let order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(display_order), 0) + 1 FROM applications",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("applications.next_display_order: {e}"))?;
    Ok(order)
}

pub fn create(conn: &Connection, name: &str) -> Result<Application, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("applications.create: name must not be empty".into());
    }
    let order = next_display_order(conn)?;
    let now = now_secs();
    conn.execute(
        "INSERT INTO applications (name, enabled, display_order, created_at)
         VALUES (?, 1, ?, ?)",
        params![trimmed, order, now],
    )
    .map_err(|e| format!("applications.create({trimmed}): {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(Application {
        id,
        name: trimmed.to_string(),
        enabled: true,
        display_order: order,
        created_at: now,
    })
}

pub fn set_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<(), String> {
    let updated = conn
        .execute(
            "UPDATE applications SET enabled = ? WHERE id = ?",
            params![enabled as i64, id],
        )
        .map_err(|e| format!("applications.set_enabled({id}): {e}"))?;
    if updated == 0 {
        return Err(format!("applications.set_enabled: no row {id}"));
    }
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM applications WHERE id = ?", params![id])
        .map_err(|e| format!("applications.delete({id}): {e}"))
        .map(|_| ())
}

pub fn add_resource(
    conn: &Connection,
    application_id: i64,
    kind: &str,
    ref_id: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO application_resources (application_id, kind, ref_id) VALUES (?, ?, ?)",
        params![application_id, kind, ref_id],
    )
    .map_err(|e| format!("applications.add_resource: {e}"))
    .map(|_| ())
}

pub fn name_taken(conn: &Connection, name: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM applications WHERE name = ?",
            params![name],
            |row| row.get(0),
        )
        .map_err(|e| format!("applications.name_taken: {e}"))?;
    Ok(count > 0)
}

pub fn resources_for(
    conn: &Connection,
    application_id: i64,
) -> Result<Vec<(String, Option<String>)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, ref_id FROM application_resources
             WHERE application_id = ? ORDER BY kind, ref_id",
        )
        .map_err(|e| format!("applications.resources_for prepare: {e}"))?;
    let rows = stmt
        .query_map(params![application_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| format!("applications.resources_for query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("applications.resources_for collect: {e}"))
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
    fn create_appends_to_list_with_monotonic_order() {
        run(|conn| {
            let a = create(conn, "alpha").unwrap();
            let b = create(conn, "beta").unwrap();
            assert!(a.display_order < b.display_order);
            let all = list(conn).unwrap();
            assert_eq!(all, vec![a.clone(), b.clone()]);
        });
    }

    #[test]
    fn create_rejects_empty_name() {
        run(|conn| {
            assert!(create(conn, "   ").is_err());
        });
    }

    #[test]
    fn set_enabled_toggles_flag() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            assert!(app.enabled);
            set_enabled(conn, app.id, false).unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all[0].enabled, false);
        });
    }

    #[test]
    fn delete_removes_row_and_referenced_resources_via_cascade() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            add_resource(conn, app.id, "dashboard", None).unwrap();
            delete(conn, app.id).unwrap();
            let all = list(conn).unwrap();
            assert!(all.is_empty());
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM application_resources WHERE application_id = ?",
                    params![app.id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn resources_for_returns_listed_pairs() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            add_resource(conn, app.id, "dashboard", None).unwrap();
            add_resource(conn, app.id, "pipeline", Some("p1")).unwrap();
            let resources = resources_for(conn, app.id).unwrap();
            assert_eq!(
                resources,
                vec![
                    ("dashboard".to_string(), None),
                    ("pipeline".to_string(), Some("p1".to_string())),
                ]
            );
        });
    }
}