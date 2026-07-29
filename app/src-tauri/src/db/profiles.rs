use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: i64,
    pub label: String,
    pub addr: String,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

pub fn list(conn: &Connection) -> Result<Vec<Profile>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, label, addr, last_used_at, created_at
             FROM connection_profiles
             ORDER BY COALESCE(last_used_at, created_at) DESC, id DESC",
        )
        .map_err(|e| format!("profiles.list prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Profile {
                id: row.get(0)?,
                label: row.get(1)?,
                addr: row.get(2)?,
                last_used_at: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| format!("profiles.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("profiles.list collect: {e}"))
}

pub fn save(conn: &Connection, label: &str, addr: &str) -> Result<i64, String> {
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, label FROM connection_profiles WHERE addr = ?",
            params![addr],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    match existing {
        Some((id, _existing_label)) => {
            conn.execute(
                "UPDATE connection_profiles SET label = ?, last_used_at = ? WHERE id = ?",
                params![label, now_secs(), id],
            )
            .map_err(|e| format!("profiles.save update: {e}"))?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO connection_profiles (label, addr, last_used_at, created_at)
                 VALUES (?, ?, ?, ?)",
                params![label, addr, now_secs(), now_secs()],
            )
            .map_err(|e| format!("profiles.save insert: {e}"))?;
            Ok(conn.last_insert_rowid())
        }
    }
}

pub fn remove(conn: &Connection, addr: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM connection_profiles WHERE addr = ?",
        params![addr],
    )
    .map_err(|e| format!("profiles.remove({addr}): {e}"))
    .map(|_| ())
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
    fn save_inserts_then_list_returns_it() {
        run(|conn| {
            let id = save(conn, "Local", "127.0.0.1:9999").unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].id, id);
            assert_eq!(all[0].addr, "127.0.0.1:9999");
            assert_eq!(all[0].label, "Local");
            assert!(all[0].last_used_at.is_some());
        });
    }

    #[test]
    fn save_upserts_by_addr_and_refreshes_label_and_last_used() {
        run(|conn| {
            let id1 = save(conn, "Old label", "10.0.0.1:8000").unwrap();
            let id2 = save(conn, "New label", "10.0.0.1:8000").unwrap();
            assert_eq!(id1, id2);
            let all = list(conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].label, "New label");
        });
    }

    #[test]
    fn remove_deletes_profile_by_addr() {
        run(|conn| {
            save(conn, "Local", "127.0.0.1:9999").unwrap();
            save(conn, "Staging", "10.0.0.2:8000").unwrap();
            remove(conn, "127.0.0.1:9999").unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].addr, "10.0.0.2:8000");
        });
    }
}
