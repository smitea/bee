use rusqlite::{params, Connection, OptionalExtension};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRow {
    pub id: i64,
    pub kind: String,
    pub resource_id: Option<String>,
    pub title: String,
    pub pinned: bool,
    pub position: i64,
    pub opened_at: i64,
    pub closed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTab {
    pub id: i64,
    pub kind: String,
    pub resource_id: Option<String>,
    pub title: String,
    pub pinned: bool,
    pub position: i64,
}

pub fn list_open(conn: &Connection) -> Result<Vec<OpenTab>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, resource_id, title, pinned, position
             FROM workspace_tabs
             WHERE closed_at IS NULL
             ORDER BY position ASC",
        )
        .map_err(|e| format!("tabs.list_open prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(OpenTab {
                id: row.get(0)?,
                kind: row.get(1)?,
                resource_id: row.get(2)?,
                title: row.get(3)?,
                pinned: row.get::<_, i64>(4)? != 0,
                position: row.get(5)?,
            })
        })
        .map_err(|e| format!("tabs.list_open query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("tabs.list_open collect: {e}"))
}

pub fn get_active(conn: &Connection) -> Result<Option<i64>, String> {
    let value: Option<i64> = conn
        .query_row(
            "SELECT active_tab_id FROM workspace_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("tabs.get_active: {e}"))?;
    Ok(value)
}

pub fn set_active(conn: &Connection, tab_id: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO workspace_state (id, active_tab_id) VALUES (1, ?)
         ON CONFLICT(id) DO UPDATE SET active_tab_id = excluded.active_tab_id",
        params![tab_id],
    )
    .map_err(|e| format!("tabs.set_active: {e}"))
    .map(|_| ())
}

pub fn open(
    conn: &Connection,
    kind: &str,
    resource_id: Option<&str>,
    title: &str,
) -> Result<i64, String> {
    if let Some(existing) = find_open(conn, kind, resource_id)? {
        return Ok(existing);
    }
    let max_position: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), 0) FROM workspace_tabs WHERE closed_at IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("tabs.open max(position): {e}"))?;
    let next_position = max_position + 1;
    conn.execute(
        "INSERT INTO workspace_tabs (kind, resource_id, title, pinned, position, opened_at)
         VALUES (?, ?, ?, 0, ?, ?)",
        params![kind, resource_id, title, next_position, now_secs()],
    )
    .map_err(|e| format!("tabs.open insert: {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(id)
}

fn find_open(
    conn: &Connection,
    kind: &str,
    resource_id: Option<&str>,
) -> Result<Option<i64>, String> {
    let row: Option<i64> = match resource_id {
        Some(rid) => conn
            .query_row(
                "SELECT id FROM workspace_tabs
                 WHERE kind = ? AND resource_id = ? AND closed_at IS NULL",
                params![kind, rid],
                |row| row.get(0),
            )
            .ok(),
        None => conn
            .query_row(
                "SELECT id FROM workspace_tabs
                 WHERE kind = ? AND closed_at IS NULL
                 ORDER BY position ASC LIMIT 1",
                params![kind],
                |row| row.get(0),
            )
            .ok(),
    };
    Ok(row)
}

pub fn close(conn: &Connection, tab_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE workspace_tabs SET closed_at = ? WHERE id = ? AND closed_at IS NULL",
        params![now_secs(), tab_id],
    )
    .map_err(|e| format!("tabs.close({tab_id}): {e}"))
    .map(|_| ())
}

pub fn set_pinned(conn: &Connection, tab_id: i64, pinned: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE workspace_tabs SET pinned = ? WHERE id = ?",
        params![pinned as i64, tab_id],
    )
    .map_err(|e| format!("tabs.set_pinned({tab_id}): {e}"))
    .map(|_| ())
}

pub fn close_others(conn: &Connection, keep_id: i64) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM workspace_tabs
             WHERE closed_at IS NULL AND id != ? AND pinned = 0
             ORDER BY id",
        )
        .map_err(|e| format!("tabs.close_others prepare: {e}"))?;
    let ids: Vec<i64> = stmt
        .query_map(params![keep_id], |row| row.get(0))
        .map_err(|e| format!("tabs.close_others query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("tabs.close_others collect: {e}"))?;
    if !ids.is_empty() {
        let placeholders: String = std::iter::repeat("?")
            .take(ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE workspace_tabs SET closed_at = ? WHERE id IN ({placeholders}) AND closed_at IS NULL"
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(ids.len() + 1);
        let now_val = now_secs();
        params_vec.push(&now_val);
        for id in &ids {
            params_vec.push(id);
        }
        conn.execute(&sql, params_vec.as_slice())
            .map_err(|e| format!("tabs.close_others update: {e}"))?;
    }
    Ok(ids)
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
    fn open_dedupes_by_kind_and_resource_id() {
        run(|conn| {
            let a = open(conn, "Cluster", None, "Cluster").unwrap();
            let b = open(conn, "Cluster", None, "Cluster").unwrap();
            assert_eq!(a, b);
            let c = open(conn, "Cluster", Some("x"), "Cluster x").unwrap();
            assert_ne!(a, c);
        });
    }

    #[test]
    fn close_marks_closed_at_and_excludes_from_list_open() {
        run(|conn| {
            let id = open(conn, "Cluster", None, "Cluster").unwrap();
            assert_eq!(list_open(conn).unwrap().len(), 1);
            close(conn, id).unwrap();
            assert!(list_open(conn).unwrap().is_empty());
        });
    }

    #[test]
    fn set_active_round_trips() {
        run(|conn| {
            let id = open(conn, "Cluster", None, "Cluster").unwrap();
            set_active(conn, id).unwrap();
            assert_eq!(get_active(conn).unwrap(), Some(id));
        });
    }

    #[test]
    fn close_others_keeps_pinned_and_target() {
        run(|conn| {
            let a = open(conn, "Pipeline", Some("a"), "Pipeline a").unwrap();
            let b = open(conn, "Pipeline", Some("b"), "Pipeline b").unwrap();
            let c = open(conn, "Pipeline", Some("c"), "Pipeline c").unwrap();
            set_pinned(conn, b, true).unwrap();
            let closed = close_others(conn, a).unwrap();
            assert_eq!(closed, vec![c]);
            let open_now: Vec<i64> = list_open(conn).unwrap().into_iter().map(|t| t.id).collect();
            assert_eq!(open_now, vec![a, b]);
        });
    }

    #[test]
    fn open_positions_are_monotonic() {
        run(|conn| {
            let a = open(conn, "X", None, "X").unwrap();
            let b = open(conn, "Y", None, "Y").unwrap();
            let tabs = list_open(conn).unwrap();
            let pos_a = tabs.iter().find(|t| t.id == a).unwrap().position;
            let pos_b = tabs.iter().find(|t| t.id == b).unwrap().position;
            assert!(pos_a < pos_b);
        });
    }
}
