use crate::db::{self, audit::{NewAuditEvent, AuditEvent}, Database};

pub fn seed(db: &Database) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("audit_seed.lock: {e}"))?;
    let existing = db::audit::count(&conn)?;
    if existing > 0 {
        return Ok(());
    }
    db::audit::record(&conn, NewAuditEvent {
        actor: "bee-client",
        action: "startup",
        result: "Success",
        summary: "Bee Client started",
        resource_kind: None,
        resource_id: None,
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("cluster"),
        nav_resource_id: None,
    })?;
    db::audit::record(&conn, NewAuditEvent {
        actor: "bee-client",
        action: "workspace.restore",
        result: "Success",
        summary: "Workspace tabs restored from local store",
        resource_kind: None,
        resource_id: None,
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: None,
        nav_resource_id: None,
    })?;
    db::audit::record(&conn, NewAuditEvent {
        actor: "bee-client",
        action: "application.create",
        result: "Success",
        summary: "Application \"Demo\" created",
        resource_kind: Some("application"),
        resource_id: None,
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    })?;
    db::audit::record(&conn, NewAuditEvent {
        actor: "bee-client",
        action: "audit.seed",
        result: "Success",
        summary: "Seeded 4 sample audit events",
        resource_kind: None,
        resource_id: None,
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: None,
        nav_resource_id: None,
    })?;
    Ok(())
}

pub fn latest(db: &Database) -> Result<Option<AuditEvent>, String> {
    let conn = db.lock().map_err(|e| format!("audit_seed.latest.lock: {e}"))?;
    db::audit::latest(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn seed_inserts_four_events_on_empty_db() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        seed(&db).unwrap();
        let conn = db.lock().unwrap();
        assert!(db::audit::count(&conn).unwrap() >= 4);
    }

    #[test]
    fn seed_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        seed(&db).unwrap();
        let first = {
            let conn = db.lock().unwrap();
            db::audit::count(&conn).unwrap()
        };
        seed(&db).unwrap();
        let second = {
            let conn = db.lock().unwrap();
            db::audit::count(&conn).unwrap()
        };
        assert_eq!(first, second);
    }
}