use rusqlite::{params, Connection, OptionalExtension};

use super::now_secs;

pub struct ClientSetting<'a> {
    pub key: &'a str,
    pub value: &'a str,
}

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM client_settings WHERE key = ?",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("settings.get({key}): {e}"))
}

pub fn put(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO client_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now_secs()],
    )
    .map_err(|e| format!("settings.put({key}): {e}"))
    .map(|_| ())
}

pub fn list(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM client_settings ORDER BY key")
        .map_err(|e| format!("settings.list prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| format!("settings.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("settings.list collect: {e}"))
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
    fn put_then_get_round_trips() {
        run(|conn| {
            put(conn, "addr", "127.0.0.1:9999").unwrap();
            let value = get(conn, "addr").unwrap();
            assert_eq!(value.as_deref(), Some("127.0.0.1:9999"));
        });
    }

    #[test]
    fn put_overwrites_existing_value() {
        run(|conn| {
            put(conn, "addr", "127.0.0.1:9999").unwrap();
            put(conn, "addr", "10.0.0.5:8888").unwrap();
            let value = get(conn, "addr").unwrap();
            assert_eq!(value.as_deref(), Some("10.0.0.5:8888"));
        });
    }

    #[test]
    fn list_returns_all_keys_sorted() {
        run(|conn| {
            put(conn, "b", "2").unwrap();
            put(conn, "a", "1").unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all, vec![("a".into(), "1".into()), ("b".into(), "2".into())]);
        });
    }

    #[test]
    fn get_missing_key_returns_none() {
        run(|conn| {
            assert!(get(conn, "nope").unwrap().is_none());
        });
    }
}
