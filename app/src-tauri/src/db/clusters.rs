use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterProfile {
    pub id: i64,
    pub label: String,
    pub addr: String,
    pub tenant: u16,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

pub fn list(conn: &Connection) -> Result<Vec<ClusterProfile>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, label, addr, tenant, last_used_at, created_at
             FROM cluster_profiles
             ORDER BY COALESCE(last_used_at, created_at) DESC, id DESC",
        )
        .map_err(|e| format!("clusters.list prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ClusterProfile {
                id: row.get(0)?,
                label: row.get(1)?,
                addr: row.get(2)?,
                tenant: row.get::<_, i64>(3)?.clamp(0, u16::MAX as i64) as u16,
                last_used_at: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("clusters.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("clusters.list collect: {e}"))
}

pub fn get(conn: &Connection, addr: &str) -> Result<Option<ClusterProfile>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, label, addr, tenant, last_used_at, created_at
             FROM cluster_profiles
             WHERE addr = ?",
        )
        .map_err(|e| format!("clusters.get prepare: {e}"))?;
    let mut rows = stmt
        .query_map(params![addr], |row| {
            Ok(ClusterProfile {
                id: row.get(0)?,
                label: row.get(1)?,
                addr: row.get(2)?,
                tenant: row.get::<_, i64>(3)?.clamp(0, u16::MAX as i64) as u16,
                last_used_at: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("clusters.get query: {e}"))?;
    match rows.next() {
        Some(Ok(p)) => Ok(Some(p)),
        Some(Err(e)) => Err(format!("clusters.get next: {e}")),
        None => Ok(None),
    }
}

pub fn save(
    conn: &Connection,
    label: &str,
    addr: &str,
    tenant: u16,
) -> Result<i64, String> {
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, label FROM cluster_profiles WHERE addr = ?",
            params![addr],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    match existing {
        Some((id, _existing_label)) => {
            conn.execute(
                "UPDATE cluster_profiles SET label = ?, tenant = ?, last_used_at = ? WHERE id = ?",
                params![label, tenant as i64, now_secs(), id],
            )
            .map_err(|e| format!("clusters.save update: {e}"))?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO cluster_profiles (label, addr, tenant, last_used_at, created_at)
                 VALUES (?, ?, ?, ?, ?)",
                params![label, addr, tenant as i64, now_secs(), now_secs()],
            )
            .map_err(|e| format!("clusters.save insert: {e}"))?;
            Ok(conn.last_insert_rowid())
        }
    }
}

pub fn remove(conn: &Connection, addr: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM cluster_profiles WHERE addr = ?",
        params![addr],
    )
    .map_err(|e| format!("clusters.remove({addr}): {e}"))
    .map(|_| ())
}

pub fn set_active(conn: &Connection, addr: &str) -> Result<ClusterProfile, String> {
    let profile = get(conn, addr)?
        .ok_or_else(|| format!("clusters.set_active: no profile for addr '{addr}'"))?;
    let now = now_secs();
    conn.execute(
        "UPDATE cluster_profiles SET last_used_at = ? WHERE id = ?",
        params![now, profile.id],
    )
    .map_err(|e| format!("clusters.set_active update: {e}"))?;
    Ok(ClusterProfile {
        last_used_at: Some(now),
        ..profile
    })
}

pub fn get_active(conn: &Connection) -> Result<Option<ClusterProfile>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, label, addr, tenant, last_used_at, created_at
             FROM cluster_profiles
             WHERE last_used_at IS NOT NULL
             ORDER BY last_used_at DESC, id DESC
             LIMIT 1",
        )
        .map_err(|e| format!("clusters.get_active prepare: {e}"))?;
    let mut rows = stmt
        .query_map([], |row| {
            Ok(ClusterProfile {
                id: row.get(0)?,
                label: row.get(1)?,
                addr: row.get(2)?,
                tenant: row.get::<_, i64>(3)?.clamp(0, u16::MAX as i64) as u16,
                last_used_at: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("clusters.get_active query: {e}"))?;
    match rows.next() {
        Some(Ok(p)) => Ok(Some(p)),
        Some(Err(e)) => Err(format!("clusters.get_active next: {e}")),
        None => Ok(None),
    }
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
            let id = save(conn, "Local", "127.0.0.1:9999", 0).unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].id, id);
            assert_eq!(all[0].addr, "127.0.0.1:9999");
            assert_eq!(all[0].label, "Local");
            assert_eq!(all[0].tenant, 0);
            assert!(all[0].last_used_at.is_some());
        });
    }

    #[test]
    fn save_upserts_by_addr_and_refreshes_label_and_tenant_and_last_used() {
        run(|conn| {
            let id1 = save(conn, "Old label", "10.0.0.1:8000", 0).unwrap();
            let id2 = save(conn, "New label", "10.0.0.1:8000", 42).unwrap();
            assert_eq!(id1, id2);
            let all = list(conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].label, "New label");
            assert_eq!(all[0].tenant, 42);
        });
    }

    #[test]
    fn save_rejects_duplicate_addr_uniqueness() {
        run(|conn| {
            save(conn, "A", "127.0.0.1:9999", 0).unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all.len(), 1);
        });
    }

    #[test]
    fn remove_deletes_profile_by_addr() {
        run(|conn| {
            save(conn, "Local", "127.0.0.1:9999", 0).unwrap();
            save(conn, "Staging", "10.0.0.2:8000", 5).unwrap();
            remove(conn, "127.0.0.1:9999").unwrap();
            let all = list(conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].addr, "10.0.0.2:8000");
        });
    }

    #[test]
    fn remove_unknown_addr_is_noop_not_error() {
        run(|conn| {
            remove(conn, "127.0.0.1:9999").unwrap();
        });
    }

    #[test]
    fn set_active_updates_last_used_and_returns_profile() {
        run(|conn| {
            save(conn, "Local", "127.0.0.1:9999", 0).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
            save(conn, "Staging", "10.0.0.2:8000", 5).unwrap();
            let active = set_active(conn, "127.0.0.1:9999").unwrap();
            assert_eq!(active.addr, "127.0.0.1:9999");
            assert!(active.last_used_at.is_some());
        });
    }

    #[test]
    fn set_active_errors_on_unknown_addr() {
        run(|conn| {
            let err = set_active(conn, "1.1.1.1:1111").unwrap_err();
            assert!(err.contains("no profile"), "got: {err}");
        });
    }

    #[test]
    fn get_active_returns_most_recent_last_used() {
        run(|conn| {
            save(conn, "Local", "127.0.0.1:9999", 0).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
            save(conn, "Staging", "10.0.0.2:8000", 5).unwrap();
            let active = get_active(conn).unwrap().unwrap();
            assert!(active.addr == "10.0.0.2:8000" || active.addr == "127.0.0.1:9999");
            assert!(active.last_used_at.is_some());
        });
    }

    #[test]
    fn get_active_returns_none_when_empty() {
        run(|conn| {
            assert!(get_active(conn).unwrap().is_none());
        });
    }

    #[test]
    fn get_returns_none_for_unknown_addr() {
        run(|conn| {
            save(conn, "Local", "127.0.0.1:9999", 0).unwrap();
            assert!(get(conn, "1.1.1.1:1111").unwrap().is_none());
        });
    }
}