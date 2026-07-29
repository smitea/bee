use rusqlite::{params, Connection, OptionalExtension};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub id: i64,
    pub timestamp: i64,
    pub actor: String,
    pub action: String,
    pub result: String,
    pub summary: String,
    pub resource_kind: Option<String>,
    pub resource_id: Option<String>,
    pub application_id: Option<i64>,
    pub correlation_id: Option<String>,
    pub operation_id: Option<String>,
    pub nav_kind: Option<String>,
    pub nav_resource_id: Option<String>,
}

pub struct NewAuditEvent<'a> {
    pub actor: &'a str,
    pub action: &'a str,
    pub result: &'a str,
    pub summary: &'a str,
    pub resource_kind: Option<&'a str>,
    pub resource_id: Option<&'a str>,
    pub application_id: Option<i64>,
    pub correlation_id: Option<&'a str>,
    pub operation_id: Option<&'a str>,
    pub nav_kind: Option<&'a str>,
    pub nav_resource_id: Option<&'a str>,
}

pub fn record(conn: &Connection, ev: NewAuditEvent<'_>) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO audit_events
         (timestamp, actor, action, result, summary,
          resource_kind, resource_id, application_id,
          correlation_id, operation_id, nav_kind, nav_resource_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            now_secs(),
            ev.actor,
            ev.action,
            ev.result,
            ev.summary,
            ev.resource_kind,
            ev.resource_id,
            ev.application_id,
            ev.correlation_id,
            ev.operation_id,
            ev.nav_kind,
            ev.nav_resource_id,
        ],
    )
    .map_err(|e| format!("audit.record: {e}"))?;
    Ok(conn.last_insert_rowid())
}

pub fn latest(conn: &Connection) -> Result<Option<AuditEvent>, String> {
    conn.query_row(
        "SELECT id, timestamp, actor, action, result, summary,
                resource_kind, resource_id, application_id,
                correlation_id, operation_id, nav_kind, nav_resource_id
         FROM audit_events
         ORDER BY id DESC LIMIT 1",
        [],
        row_to_event,
    )
    .optional()
    .map_err(|e| format!("audit.latest: {e}"))
}

pub fn list(conn: &Connection, limit: i64) -> Result<Vec<AuditEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, timestamp, actor, action, result, summary,
                    resource_kind, resource_id, application_id,
                    correlation_id, operation_id, nav_kind, nav_resource_id
             FROM audit_events
             ORDER BY id DESC LIMIT ?",
        )
        .map_err(|e| format!("audit.list prepare: {e}"))?;
    let rows = stmt
        .query_map(params![limit], row_to_event)
        .map_err(|e| format!("audit.list query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("audit.list collect: {e}"))
}

pub fn query(
    conn: &Connection,
    application_id: Option<i64>,
    limit: i64,
) -> Result<Vec<AuditEvent>, String> {
    match application_id {
        Some(app_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, timestamp, actor, action, result, summary,
                            resource_kind, resource_id, application_id,
                            correlation_id, operation_id, nav_kind, nav_resource_id
                     FROM audit_events
                     WHERE application_id = ?
                     ORDER BY id DESC LIMIT ?",
                )
                .map_err(|e| format!("audit.query app prepare: {e}"))?;
            let rows = stmt
                .query_map(params![app_id, limit], row_to_event)
                .map_err(|e| format!("audit.query app query: {e}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("audit.query app collect: {e}"))
        }
        None => list(conn, limit),
    }
}

pub fn count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))
        .map_err(|e| format!("audit.count: {e}"))
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEvent> {
    Ok(AuditEvent {
        id: row.get(0)?,
        timestamp: row.get(1)?,
        actor: row.get(2)?,
        action: row.get(3)?,
        result: row.get(4)?,
        summary: row.get(5)?,
        resource_kind: row.get(6)?,
        resource_id: row.get(7)?,
        application_id: row.get(8)?,
        correlation_id: row.get(9)?,
        operation_id: row.get(10)?,
        nav_kind: row.get(11)?,
        nav_resource_id: row.get(12)?,
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

    fn ev<'a>(summary: &'a str) -> NewAuditEvent<'a> {
        NewAuditEvent {
            actor: "tester",
            action: "test",
            result: "Success",
            summary,
            resource_kind: Some("pipeline"),
            resource_id: Some("p1"),
            application_id: None,
            correlation_id: None,
            operation_id: None,
            nav_kind: Some("pipeline"),
            nav_resource_id: Some("p1"),
        }
    }

    #[test]
    fn record_persists_and_reads_back_via_latest() {
        run(|conn| {
            let id = record(conn, ev("first")).unwrap();
            let latest = latest(conn).unwrap().unwrap();
            assert_eq!(latest.id, id);
            assert_eq!(latest.summary, "first");
            assert_eq!(latest.nav_kind.as_deref(), Some("pipeline"));
        });
    }

    #[test]
    fn list_orders_newest_first() {
        run(|conn| {
            record(conn, ev("a")).unwrap();
            record(conn, ev("b")).unwrap();
            record(conn, ev("c")).unwrap();
            let all = list(conn, 10).unwrap();
            let summaries: Vec<_> = all.iter().map(|e| e.summary.as_str()).collect();
            assert_eq!(summaries, vec!["c", "b", "a"]);
        });
    }

    #[test]
    fn query_filters_by_application_id() {
        run(|conn| {
            let app_id = crate::db::applications::create(conn, "alpha").unwrap().id;
            record(conn, NewAuditEvent {
                actor: "t", action: "a", result: "Success", summary: "global",
                resource_kind: None, resource_id: None, application_id: None,
                correlation_id: None, operation_id: None,
                nav_kind: None, nav_resource_id: None,
            }).unwrap();
            record(conn, NewAuditEvent {
                actor: "t", action: "a", result: "Success", summary: "for app",
                resource_kind: None, resource_id: None, application_id: Some(app_id),
                correlation_id: None, operation_id: None,
                nav_kind: None, nav_resource_id: None,
            }).unwrap();
            let filtered = query(conn, Some(app_id), 50).unwrap();
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].summary, "for app");
        });
    }

    #[test]
    fn count_increments_per_record() {
        run(|conn| {
            assert_eq!(count(conn).unwrap(), 0);
            record(conn, ev("one")).unwrap();
            record(conn, ev("two")).unwrap();
            assert_eq!(count(conn).unwrap(), 2);
        });
    }
}