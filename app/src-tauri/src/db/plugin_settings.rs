use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSetting {
    pub plugin_name: String,
    pub enabled: bool,
    pub config_json: String,
    pub updated_at: i64,
}

pub fn get(conn: &Connection, plugin_name: &str) -> Result<Option<PluginSetting>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT plugin_name, enabled, config_json, updated_at
             FROM plugin_settings WHERE plugin_name = ?",
        )
        .map_err(|e| format!("plugin_settings.get prepare: {e}"))?;
    let mut rows = stmt
        .query_map(params![plugin_name], |row| {
            Ok(PluginSetting {
                plugin_name: row.get(0)?,
                enabled: row.get::<_, i64>(1)? != 0,
                config_json: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .map_err(|e| format!("plugin_settings.get query: {e}"))?;
    match rows.next() {
        Some(Ok(p)) => Ok(Some(p)),
        Some(Err(e)) => Err(format!("plugin_settings.get next: {e}")),
        None => Ok(None),
    }
}

pub fn list(conn: &Connection) -> Result<Vec<PluginSetting>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT plugin_name, enabled, config_json, updated_at
             FROM plugin_settings ORDER BY plugin_name ASC",
        )
        .map_err(|e| format!("plugin_settings.list prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PluginSetting {
                plugin_name: row.get(0)?,
                enabled: row.get::<_, i64>(1)? != 0,
                config_json: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .map_err(|e| format!("plugin_settings.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("plugin_settings.list collect: {e}"))
}

pub fn upsert(
    conn: &Connection,
    plugin_name: &str,
    enabled: bool,
    config_json: &str,
) -> Result<PluginSetting, String> {
    let now = now_secs();
    conn.execute(
        "INSERT INTO plugin_settings (plugin_name, enabled, config_json, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(plugin_name) DO UPDATE SET
            enabled = excluded.enabled,
            config_json = excluded.config_json,
            updated_at = excluded.updated_at",
        params![plugin_name, enabled as i64, config_json, now],
    )
    .map_err(|e| format!("plugin_settings.upsert({plugin_name}): {e}"))?;
    Ok(PluginSetting {
        plugin_name: plugin_name.to_string(),
        enabled,
        config_json: config_json.to_string(),
        updated_at: now,
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
    fn get_returns_none_for_unknown_plugin() {
        run(|conn| {
            assert!(get(conn, "missing").unwrap().is_none());
        });
    }

    #[test]
    fn upsert_then_get_round_trips() {
        run(|conn| {
            let saved = upsert(conn, "binance", true, r#"{"url":"wss://example"}"#).unwrap();
            assert_eq!(saved.plugin_name, "binance");
            assert!(saved.enabled);
            assert_eq!(saved.config_json, r#"{"url":"wss://example"}"#);
            let fetched = get(conn, "binance").unwrap().unwrap();
            assert_eq!(fetched, saved);
        });
    }

    #[test]
    fn upsert_overwrites_existing_enabled_and_config() {
        run(|conn| {
            upsert(conn, "binance", true, "{}").unwrap();
            let updated = upsert(conn, "binance", false, r#"{"rate":2}"#).unwrap();
            assert!(!updated.enabled);
            assert_eq!(updated.config_json, r#"{"rate":2}"#);
        });
    }

    #[test]
    fn list_returns_all_rows_alphabetically() {
        run(|conn| {
            upsert(conn, "binance", true, "{}").unwrap();
            upsert(conn, "alpha", true, "{}").unwrap();
            upsert(conn, "zeta", false, "{}").unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all.len(), 3);
            assert_eq!(all[0].plugin_name, "alpha");
            assert_eq!(all[1].plugin_name, "binance");
            assert_eq!(all[2].plugin_name, "zeta");
        });
    }
}