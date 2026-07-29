use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Datasource {
    pub name: String,
    pub plugin: String,
    pub config: String,
    pub tenant: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn list(conn: &Connection) -> Result<Vec<Datasource>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT name, plugin, config, tenant, created_at, updated_at
             FROM datasources
             ORDER BY name ASC",
        )
        .map_err(|e| format!("datasources.list prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Datasource {
                name: row.get(0)?,
                plugin: row.get(1)?,
                config: row.get(2)?,
                tenant: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("datasources.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("datasources.list collect: {e}"))
}

pub fn get(conn: &Connection, name: &str) -> Result<Option<Datasource>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT name, plugin, config, tenant, created_at, updated_at
             FROM datasources
             WHERE name = ?",
        )
        .map_err(|e| format!("datasources.get prepare: {e}"))?;
    let mut rows = stmt
        .query_map(params![name], |row| {
            Ok(Datasource {
                name: row.get(0)?,
                plugin: row.get(1)?,
                config: row.get(2)?,
                tenant: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("datasources.get query: {e}"))?;
    match rows.next() {
        Some(Ok(d)) => Ok(Some(d)),
        Some(Err(e)) => Err(format!("datasources.get next: {e}")),
        None => Ok(None),
    }
}

pub fn create(
    conn: &Connection,
    name: &str,
    plugin: &str,
    config: &str,
    tenant: i64,
) -> Result<Datasource, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("datasources.create: name must not be empty".into());
    }
    let now = now_secs();
    let outcome = conn.execute(
        "INSERT INTO datasources (name, plugin, config, tenant, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        params![trimmed, plugin, config, tenant, now, now],
    );
    match outcome {
        Ok(_) => {}
        Err(e) => {
            let msg = format!("{e}");
            if msg.contains("UNIQUE constraint failed: datasources.name") {
                return Err(format!("datasources.create: name '{trimmed}' already exists"));
            }
            return Err(format!("datasources.create({trimmed}): {e}"));
        }
    }
    Ok(Datasource {
        name: trimmed.to_string(),
        plugin: plugin.to_string(),
        config: config.to_string(),
        tenant,
        created_at: now,
        updated_at: now,
    })
}

pub fn delete(conn: &Connection, name: &str) -> Result<(), String> {
    conn.execute("DELETE FROM datasources WHERE name = ?", params![name])
        .map_err(|e| format!("datasources.delete({name}): {e}"))
        .map(|_| ())
}

pub fn name_taken(conn: &Connection, name: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM datasources WHERE name = ?",
            params![name],
            |row| row.get(0),
        )
        .map_err(|e| format!("datasources.name_taken: {e}"))?;
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
    fn create_persists_row_with_returned_name() {
        run(|conn| {
            let ds = create(conn, "binance", "binance_subscribe", r#"{"url":"wss://x"}"#, 0).unwrap();
            assert_eq!(ds.name, "binance");
            assert_eq!(ds.plugin, "binance_subscribe");
            assert_eq!(ds.tenant, 0);
            assert!(ds.created_at > 0);
            assert!(ds.updated_at > 0);
        });
    }

    #[test]
    fn get_returns_none_when_name_missing() {
        run(|conn| {
            assert!(get(conn, "nope").unwrap().is_none());
        });
    }

    #[test]
    fn get_round_trips_inserted_row() {
        run(|conn| {
            let ds = create(conn, "binance", "binance_subscribe", "{}", 0).unwrap();
            let fetched = get(conn, "binance").unwrap().unwrap();
            assert_eq!(fetched, ds);
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
            let b = create(conn, "bravo", "p", "{}", 0).unwrap();
            let a = create(conn, "alpha", "p", "{}", 0).unwrap();
            let c = create(conn, "charlie", "p", "{}", 0).unwrap();
            let all = list(conn).unwrap();
            assert_eq!(
                all.iter().map(|d| d.name.clone()).collect::<Vec<_>>(),
                vec!["alpha".to_string(), "bravo".to_string(), "charlie".to_string()]
            );
            assert_eq!(all, vec![a, b, c]);
        });
    }

    #[test]
    fn create_rejects_empty_name() {
        run(|conn| {
            assert!(create(conn, "", "p", "{}", 0).is_err());
            assert!(create(conn, "   ", "p", "{}", 0).is_err());
        });
    }

    #[test]
    fn name_uniqueness_is_enforced() {
        run(|conn| {
            create(conn, "alpha", "p", "{}", 0).unwrap();
            let err = create(conn, "alpha", "p", "{}", 0).unwrap_err();
            assert!(err.contains("already exists"), "got: {err}");
            assert!(name_taken(conn, "alpha").unwrap());
            assert!(!name_taken(conn, "beta").unwrap());
        });
    }

    #[test]
    fn delete_removes_row_and_list_omits_it() {
        run(|conn| {
            let _a = create(conn, "alpha", "p", "{}", 0).unwrap();
            let b = create(conn, "bravo", "p", "{}", 0).unwrap();
            delete(conn, "alpha").unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all, vec![b]);
            assert!(get(conn, "alpha").unwrap().is_none());
        });
    }

    #[test]
    fn delete_unknown_name_is_noop_not_error() {
        run(|conn| {
            delete(conn, "nope").unwrap();
        });
    }

    #[test]
    fn create_with_tenant_persists_tenant_field() {
        run(|conn| {
            let ds = create(conn, "alpha", "p", "{}", 42).unwrap();
            assert_eq!(ds.tenant, 42);
        });
    }
}