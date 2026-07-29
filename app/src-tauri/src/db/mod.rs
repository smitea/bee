use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

pub mod settings;
pub mod tabs;
pub mod profiles;

pub struct Database {
    conn: Mutex<Connection>,
    path: PathBuf,
}

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "client_settings",
        sql: r#"
            CREATE TABLE client_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
        "#,
    },
    Migration {
        version: 2,
        name: "workspace_tabs",
        sql: r#"
            CREATE TABLE workspace_tabs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                resource_id TEXT,
                title TEXT NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0,
                position INTEGER NOT NULL,
                opened_at INTEGER NOT NULL,
                closed_at INTEGER
            );
            CREATE UNIQUE INDEX idx_workspace_tabs_open_unique
                ON workspace_tabs(kind, resource_id) WHERE closed_at IS NULL;
            CREATE TABLE workspace_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                active_tab_id INTEGER REFERENCES workspace_tabs(id)
            );
        "#,
    },
    Migration {
        version: 3,
        name: "connection_profiles",
        sql: r#"
            CREATE TABLE connection_profiles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                label TEXT NOT NULL,
                addr TEXT NOT NULL UNIQUE,
                last_used_at INTEGER,
                created_at INTEGER NOT NULL
            );
        "#,
    },
];

impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("open: {e}"))?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();
        let db = Self { conn: Mutex::new(conn), path: path.to_path_buf() };
        db.apply_migrations()?;
        Ok(db)
    }

    pub fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.conn.lock().map_err(|e| format!("db lock poisoned: {e}"))
    }

    pub fn path(&self) -> &Path { &self.path }

    pub fn apply_migrations(&self) -> Result<(), String> {
        let next: u32 = {
            let conn = self.lock()?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at INTEGER NOT NULL
                );",
            ).map_err(|e| format!("create migrations table: {e}"))?;
            let applied: Option<i32> = conn
                .query_row(
                    "SELECT version FROM migrations ORDER BY version DESC LIMIT 1",
                    [],
                    |row| row.get::<_, i32>(0),
                )
                .ok();
            applied.map(|v| (v as u32) + 1).unwrap_or(1)
        };
        for m in MIGRATIONS.iter().filter(|m| m.version >= next) {
            let conn = self.lock()?;
            let tx = conn.unchecked_transaction()
                .map_err(|e| format!("begin tx v{}: {e}", m.version))?;
            tx.execute_batch(m.sql)
                .map_err(|e| format!("apply v{} ({}): {e}", m.version, m.name))?;
            tx.execute(
                "INSERT INTO migrations (version, name, applied_at) VALUES (?, ?, ?)",
                params![m.version, m.name, now_secs()],
            ).map_err(|e| format!("record v{}: {e}", m.version))?;
            tx.commit().map_err(|e| format!("commit v{}: {e}", m.version))?;
            drop(conn);
        }
        Ok(())
    }

    pub fn applied_versions(&self) -> Result<Vec<u32>, String> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT version FROM migrations ORDER BY version")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt.query_map([], |row| row.get::<_, u32>(0))
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("collect: {e}"))
    }
}

pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_file_and_applies_migrations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        assert!(path.exists());
        let applied = db.applied_versions().unwrap();
        assert_eq!(applied, vec![1, 2, 3]);
    }

    #[test]
    fn apply_migrations_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        let first = db.applied_versions().unwrap();
        let db2 = Database::open(&path).unwrap();
        let second = db2.applied_versions().unwrap();
        assert_eq!(first, second);
        assert_eq!(first, vec![1, 2, 3]);
    }

    #[test]
    fn applied_versions_empty_when_db_just_created_with_no_migrations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL);").unwrap();
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(db.applied_versions().unwrap(), vec![1, 2, 3]);
    }
}
