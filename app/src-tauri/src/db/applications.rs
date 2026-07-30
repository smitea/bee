use rusqlite::{params, Connection};

use super::now_secs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub display_order: i64,
    pub tenant: u16,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisableSnapshot {
    pub application_id: i64,
    pub taken_at: i64,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSnapshot {
    pub application_id: i64,
    pub taken_at: i64,
    pub resource_kind: String,
    pub resource_id: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisableOutcome {
    pub snapshot_rows: Vec<ResourceSnapshot>,
    pub pipelines: Vec<String>,
    pub datasources: Vec<String>,
    pub enabled_after: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceOp {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedResource {
    pub kind: String,
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnableOutcome {
    pub outcome: String,
    pub succeeded: Vec<ResourceOp>,
    pub failed: Vec<FailedResource>,
    pub skipped: Vec<ResourceOp>,
    pub enabled_after: bool,
}

pub fn list(conn: &Connection) -> Result<Vec<Application>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, enabled, display_order, tenant, created_at
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
                tenant: row.get::<_, i64>(4)?.clamp(0, u16::MAX as i64) as u16,
                created_at: row.get(5)?,
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
        "INSERT INTO applications (name, enabled, display_order, tenant, created_at)
         VALUES (?, 1, ?, 0, ?)",
        params![trimmed, order, now],
    )
    .map_err(|e| format!("applications.create({trimmed}): {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(Application {
        id,
        name: trimmed.to_string(),
        enabled: true,
        display_order: order,
        tenant: 0,
        created_at: now,
    })
}

pub fn create_with_tenant(
    conn: &Connection,
    name: &str,
    tenant: u16,
) -> Result<Application, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("applications.create: name must not be empty".into());
    }
    let order = next_display_order(conn)?;
    let now = now_secs();
    conn.execute(
        "INSERT INTO applications (name, enabled, display_order, tenant, created_at)
         VALUES (?, 1, ?, ?, ?)",
        params![trimmed, order, tenant as i64, now],
    )
    .map_err(|e| format!("applications.create({trimmed}): {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(Application {
        id,
        name: trimmed.to_string(),
        enabled: true,
        display_order: order,
        tenant,
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

pub fn get(conn: &Connection, id: i64) -> Result<Option<Application>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, enabled, display_order, tenant, created_at
             FROM applications
             WHERE id = ?",
        )
        .map_err(|e| format!("applications.get prepare: {e}"))?;
    let mut rows = stmt
        .query_map(params![id], |row| {
            Ok(Application {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                display_order: row.get(3)?,
                tenant: row.get::<_, i64>(4)?.clamp(0, u16::MAX as i64) as u16,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("applications.get query: {e}"))?;
    match rows.next() {
        Some(Ok(a)) => Ok(Some(a)),
        Some(Err(e)) => Err(format!("applications.get next: {e}")),
        None => Ok(None),
    }
}

pub fn set_tenant(conn: &Connection, id: i64, tenant: u16) -> Result<(), String> {
    let updated = conn
        .execute(
            "UPDATE applications SET tenant = ? WHERE id = ?",
            params![tenant as i64, id],
        )
        .map_err(|e| format!("applications.set_tenant({id}): {e}"))?;
    if updated == 0 {
        return Err(format!("applications.set_tenant: no row {id}"));
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

pub fn record_disable_snapshot(
    conn: &Connection,
    application_id: i64,
    payload_json: &str,
) -> Result<DisableSnapshot, String> {
    let taken_at = now_secs();
    conn.execute(
        "INSERT INTO application_disable_snapshots (application_id, taken_at, payload_json)
         VALUES (?, ?, ?)",
        params![application_id, taken_at, payload_json],
    )
    .map_err(|e| format!("applications.record_disable_snapshot({application_id}): {e}"))?;
    Ok(DisableSnapshot {
        application_id,
        taken_at,
        payload_json: payload_json.to_string(),
    })
}

pub fn list_disable_snapshots(
    conn: &Connection,
    application_id: i64,
) -> Result<Vec<DisableSnapshot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT application_id, taken_at, payload_json
             FROM application_disable_snapshots
             WHERE application_id = ?
             ORDER BY taken_at DESC",
        )
        .map_err(|e| format!("applications.list_disable_snapshots prepare: {e}"))?;
    let rows = stmt
        .query_map(params![application_id], |row| {
            Ok(DisableSnapshot {
                application_id: row.get(0)?,
                taken_at: row.get(1)?,
                payload_json: row.get(2)?,
            })
        })
        .map_err(|e| format!("applications.list_disable_snapshots query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("applications.list_disable_snapshots collect: {e}"))
}

pub fn take_resource_snapshot(
    conn: &Connection,
    application_id: i64,
    resource_kind: &str,
    resource_id: &str,
    payload_json: &str,
) -> Result<ResourceSnapshot, String> {
    let taken_at = now_secs();
    conn.execute(
        "INSERT INTO application_resource_snapshots
            (application_id, taken_at, resource_kind, resource_id, payload_json)
         VALUES (?, ?, ?, ?, ?)",
        params![application_id, taken_at, resource_kind, resource_id, payload_json],
    )
    .map_err(|e| format!(
        "applications.take_resource_snapshot(app={application_id}, {resource_kind}={resource_id}): {e}"
    ))?;
    Ok(ResourceSnapshot {
        application_id,
        taken_at,
        resource_kind: resource_kind.to_string(),
        resource_id: resource_id.to_string(),
        payload_json: payload_json.to_string(),
    })
}

pub fn list_resource_snapshots(
    conn: &Connection,
    application_id: i64,
) -> Result<Vec<ResourceSnapshot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT application_id, taken_at, resource_kind, resource_id, payload_json
             FROM application_resource_snapshots
             WHERE application_id = ?
             ORDER BY taken_at ASC, resource_kind ASC, resource_id ASC",
        )
        .map_err(|e| format!("applications.list_resource_snapshots prepare: {e}"))?;
    let rows = stmt
        .query_map(params![application_id], |row| {
            Ok(ResourceSnapshot {
                application_id: row.get(0)?,
                taken_at: row.get(1)?,
                resource_kind: row.get(2)?,
                resource_id: row.get(3)?,
                payload_json: row.get(4)?,
            })
        })
        .map_err(|e| format!("applications.list_resource_snapshots query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("applications.list_resource_snapshots collect: {e}"))
}

pub fn application_disable(conn: &Connection, application_id: i64) -> Result<DisableOutcome, String> {
    let pipelines = crate::db::pipelines::list(conn)?;
    let datasources = crate::db::datasources::list(conn)?;

    let mut snapshot_rows: Vec<ResourceSnapshot> = Vec::new();

    for p in &pipelines {
        let payload = serde_json::to_string(&serde_json::json!({
            "id": p.id,
            "name": p.name,
            "dag_json": p.dag_json,
        }))
        .map_err(|e| format!("applications.application_disable serialize pipeline: {e}"))?;
        let row = take_resource_snapshot(conn, application_id, "pipeline", &p.name, &payload)?;
        let summary = format!("snapshotted pipeline \"{}\"", p.name);
        let _ = crate::db::audit::record(
            conn,
            crate::db::audit::NewAuditEvent {
                actor: "user",
                action: "application.disable",
                result: "Success",
                summary: &summary,
                resource_kind: Some("pipeline"),
                resource_id: Some(&p.name),
                application_id: Some(application_id),
                correlation_id: None,
                operation_id: None,
                nav_kind: Some("pipeline"),
                nav_resource_id: Some(&p.name),
            },
        );
        snapshot_rows.push(row);
    }

    for d in &datasources {
        let payload = serde_json::to_string(&serde_json::json!({
            "name": d.name,
            "plugin": d.plugin,
            "config": d.config,
            "tenant": d.tenant,
        }))
        .map_err(|e| format!("applications.application_disable serialize datasource: {e}"))?;
        let row = take_resource_snapshot(conn, application_id, "datasource", &d.name, &payload)?;
        let summary = format!("snapshotted datasource \"{}\"", d.name);
        let _ = crate::db::audit::record(
            conn,
            crate::db::audit::NewAuditEvent {
                actor: "user",
                action: "application.disable",
                result: "Success",
                summary: &summary,
                resource_kind: Some("datasource"),
                resource_id: Some(&d.name),
                application_id: Some(application_id),
                correlation_id: None,
                operation_id: None,
                nav_kind: Some("datasource"),
                nav_resource_id: Some(&d.name),
            },
        );
        snapshot_rows.push(row);
    }

    let was_enabled = get(conn, application_id)?
        .ok_or_else(|| format!("applications.application_disable: no row {application_id}"))?
        .enabled;
    if was_enabled {
        set_enabled(conn, application_id, false)?;
    }

    let summary = format!(
        "Application disabled (snapshotted {} pipelines, {} datasources)",
        pipelines.len(),
        datasources.len()
    );
    let _ = crate::db::audit::record(
        conn,
        crate::db::audit::NewAuditEvent {
            actor: "user",
            action: "application.disable",
            result: "Success",
            summary: &summary,
            resource_kind: Some("application"),
            resource_id: None,
            application_id: Some(application_id),
            correlation_id: None,
            operation_id: None,
            nav_kind: Some("application"),
            nav_resource_id: None,
        },
    );

    Ok(DisableOutcome {
        snapshot_rows,
        pipelines: pipelines.into_iter().map(|p| p.name).collect(),
        datasources: datasources.into_iter().map(|d| d.name).collect(),
        enabled_after: false,
    })
}

pub fn application_enable<F>(
    conn: &Connection,
    application_id: i64,
    mut on_resource: F,
) -> Result<EnableOutcome, String>
where
    F: FnMut(&ResourceSnapshot) -> Result<(), String>,
{
    let snapshots = list_resource_snapshots(conn, application_id)?;

    let mut succeeded: Vec<ResourceOp> = Vec::new();
    let mut failed: Vec<FailedResource> = Vec::new();
    let mut skipped: Vec<ResourceOp> = Vec::new();

    for snap in &snapshots {
        match on_resource(snap) {
            Ok(()) => {
                let summary =
                    format!("rehydrated {} \"{}\"", snap.resource_kind, snap.resource_id);
                let _ = crate::db::audit::record(
                    conn,
                    crate::db::audit::NewAuditEvent {
                        actor: "user",
                        action: "application.enable",
                        result: "Success",
                        summary: &summary,
                        resource_kind: Some(&snap.resource_kind),
                        resource_id: Some(&snap.resource_id),
                        application_id: Some(application_id),
                        correlation_id: None,
                        operation_id: None,
                        nav_kind: Some(&snap.resource_kind),
                        nav_resource_id: Some(&snap.resource_id),
                    },
                );
                succeeded.push(ResourceOp {
                    kind: snap.resource_kind.clone(),
                    id: snap.resource_id.clone(),
                });
            }
            Err(reason) => {
                let summary = format!(
                    "failed to rehydrate {} \"{}\": {}",
                    snap.resource_kind, snap.resource_id, reason
                );
                let _ = crate::db::audit::record(
                    conn,
                    crate::db::audit::NewAuditEvent {
                        actor: "user",
                        action: "application.enable",
                        result: "Failure",
                        summary: &summary,
                        resource_kind: Some(&snap.resource_kind),
                        resource_id: Some(&snap.resource_id),
                        application_id: Some(application_id),
                        correlation_id: None,
                        operation_id: None,
                        nav_kind: Some(&snap.resource_kind),
                        nav_resource_id: Some(&snap.resource_id),
                    },
                );
                failed.push(FailedResource {
                    kind: snap.resource_kind.clone(),
                    id: snap.resource_id.clone(),
                    reason,
                });
            }
        }
    }

    let outcome = if snapshots.is_empty() {
        "Failure"
    } else if failed.is_empty() {
        "Success"
    } else if succeeded.is_empty() {
        "Failure"
    } else {
        "Degraded"
    };

    let mut enabled_after = false;
    if outcome != "Failure" {
        set_enabled(conn, application_id, true)?;
        enabled_after = true;
    }

    let summary = format!(
        "Application enable: {} succeeded, {} failed, {} skipped",
        succeeded.len(),
        failed.len(),
        skipped.len()
    );
    let _ = crate::db::audit::record(
        conn,
        crate::db::audit::NewAuditEvent {
            actor: "user",
            action: "application.enable",
            result: outcome,
            summary: &summary,
            resource_kind: Some("application"),
            resource_id: None,
            application_id: Some(application_id),
            correlation_id: None,
            operation_id: None,
            nav_kind: Some("application"),
            nav_resource_id: None,
        },
    );

    Ok(EnableOutcome {
        outcome: outcome.to_string(),
        succeeded,
        failed,
        skipped,
        enabled_after,
    })
}

pub fn snapshot_payload(
    pipelines: &[crate::db::pipelines::PipelineDefinition],
    datasources: &[crate::db::datasources::Datasource],
) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        pipelines: Vec<PipelineRef<'a>>,
        datasources: Vec<DatasourceRef<'a>>,
    }
    #[derive(serde::Serialize)]
    struct PipelineRef<'a> {
        id: i64,
        name: &'a str,
    }
    #[derive(serde::Serialize)]
    struct DatasourceRef<'a> {
        name: &'a str,
        plugin: &'a str,
    }
    serde_json::to_string(&Payload {
        pipelines: pipelines
            .iter()
            .map(|p| PipelineRef {
                id: p.id,
                name: p.name.as_str(),
            })
            .collect(),
        datasources: datasources
            .iter()
            .map(|d| DatasourceRef {
                name: d.name.as_str(),
                plugin: d.plugin.as_str(),
            })
            .collect(),
    })
    .map_err(|e| format!("applications.snapshot_payload: {e}"))
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
    fn create_with_tenant_persists_tenant_field() {
        run(|conn| {
            let app = create_with_tenant(conn, "alpha", 7).unwrap();
            assert_eq!(app.tenant, 7);
            let fetched = get(conn, app.id).unwrap().unwrap();
            assert_eq!(fetched.tenant, 7);
        });
    }

    #[test]
    fn set_tenant_updates_field() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            assert_eq!(app.tenant, 0);
            set_tenant(conn, app.id, 42).unwrap();
            let fetched = get(conn, app.id).unwrap().unwrap();
            assert_eq!(fetched.tenant, 42);
        });
    }

    #[test]
    fn set_tenant_errors_on_unknown_id() {
        run(|conn| {
            assert!(set_tenant(conn, 9999, 1).is_err());
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
    fn set_enabled_is_idempotent_on_consecutive_calls() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            set_enabled(conn, app.id, true).unwrap();
            set_enabled(conn, app.id, true).unwrap();
            let all = list(conn).unwrap();
            assert!(all[0].enabled);
        });
    }

    #[test]
    fn get_returns_none_for_missing_id() {
        run(|conn| {
            assert!(get(conn, 9999).unwrap().is_none());
        });
    }

    #[test]
    fn get_round_trips_inserted_row() {
        run(|conn| {
            let a = create(conn, "alpha").unwrap();
            let fetched = get(conn, a.id).unwrap().unwrap();
            assert_eq!(fetched, a);
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

    #[test]
    fn record_disable_snapshot_appends_each_time() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            let s1 = record_disable_snapshot(conn, app.id, r#"{"v":1}"#).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
            let s2 = record_disable_snapshot(conn, app.id, r#"{"v":2}"#).unwrap();
            assert!(s2.taken_at >= s1.taken_at);
            let snapshots = list_disable_snapshots(conn, app.id).unwrap();
            assert_eq!(snapshots.len(), 2);
            assert_eq!(snapshots[0].payload_json, r#"{"v":2}"#);
            assert_eq!(snapshots[1].payload_json, r#"{"v":1}"#);
        });
    }

    #[test]
    fn record_disable_snapshot_unknown_id_errors() {
        run(|conn| {
            assert!(record_disable_snapshot(conn, 9999, "{}").is_err());
        });
    }

    #[test]
    fn snapshot_payload_lists_pipelines_and_datasources_by_id_and_name() {
        let pipelines = vec![
            crate::db::pipelines::PipelineDefinition {
                id: 1,
                name: "alpha".into(),
                dag_json: "{}".into(),
                updated_at: 0,
            },
            crate::db::pipelines::PipelineDefinition {
                id: 2,
                name: "beta".into(),
                dag_json: "{}".into(),
                updated_at: 0,
            },
        ];
        let datasources = vec![crate::db::datasources::Datasource {
            name: "binance".into(),
            plugin: "binance_subscribe".into(),
            config: "{}".into(),
            tenant: 0,
            created_at: 0,
            updated_at: 0,
        }];
        let json = snapshot_payload(&pipelines, &datasources).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ps = v.get("pipelines").unwrap().as_array().unwrap();
        let ds = v.get("datasources").unwrap().as_array().unwrap();
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].get("id").unwrap().as_i64(), Some(1));
        assert_eq!(ps[0].get("name").unwrap().as_str(), Some("alpha"));
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].get("name").unwrap().as_str(), Some("binance"));
        assert_eq!(
            ds[0].get("plugin").unwrap().as_str(),
            Some("binance_subscribe")
        );
    }

    #[test]
    fn take_resource_snapshot_writes_one_row_and_list_resource_snapshots_reads_it_back() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            let snap =
                take_resource_snapshot(conn, app.id, "pipeline", "p1", r#"{"k":1}"#).unwrap();
            assert_eq!(snap.application_id, app.id);
            assert_eq!(snap.resource_kind, "pipeline");
            assert_eq!(snap.resource_id, "p1");
            assert_eq!(snap.payload_json, r#"{"k":1}"#);
            assert!(snap.taken_at > 0);

            let rows = list_resource_snapshots(conn, app.id).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0], snap);
        });
    }

    #[test]
    fn list_resource_snapshots_returns_empty_when_no_rows() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            assert!(list_resource_snapshots(conn, app.id).unwrap().is_empty());
        });
    }

    #[test]
    fn disable_writes_per_resource_snapshot_rows() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            crate::db::pipelines::create(conn, "p1", "{}").unwrap();
            crate::db::pipelines::create(conn, "p2", "{}").unwrap();
            crate::db::datasources::create(conn, "binance", "binance_subscribe", "{}", 0).unwrap();
            crate::db::datasources::create(conn, "newsapi", "newsapi_subscribe", "{}", 0).unwrap();

            let outcome = application_disable(conn, app.id).unwrap();
            assert_eq!(outcome.snapshot_rows.len(), 4);
            assert_eq!(outcome.pipelines.len(), 2);
            assert_eq!(outcome.datasources.len(), 2);
            assert!(!outcome.enabled_after);

            let rows = list_resource_snapshots(conn, app.id).unwrap();
            assert_eq!(rows.len(), 4);

            let kinds: std::collections::HashSet<&str> =
                rows.iter().map(|r| r.resource_kind.as_str()).collect();
            assert!(kinds.contains("pipeline"));
            assert!(kinds.contains("datasource"));

            let pipeline_ids: std::collections::HashSet<&str> = rows
                .iter()
                .filter(|r| r.resource_kind == "pipeline")
                .map(|r| r.resource_id.as_str())
                .collect();
            assert!(pipeline_ids.contains("p1"));
            assert!(pipeline_ids.contains("p2"));

            let datasource_ids: std::collections::HashSet<&str> = rows
                .iter()
                .filter(|r| r.resource_kind == "datasource")
                .map(|r| r.resource_id.as_str())
                .collect();
            assert!(datasource_ids.contains("binance"));
            assert!(datasource_ids.contains("newsapi"));

            let fetched = get(conn, app.id).unwrap().unwrap();
            assert!(!fetched.enabled);

            let events = crate::db::audit::query(conn, Some(app.id), 100).unwrap();
            let disable_events: Vec<_> = events
                .iter()
                .filter(|e| e.action == "application.disable")
                .collect();
            assert!(disable_events.len() >= 4);
        });
    }

    #[test]
    fn enable_returns_failure_when_snapshot_empty() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            set_enabled(conn, app.id, false).unwrap();

            let mut called = 0;
            let outcome = application_enable(conn, app.id, |_| {
                called += 1;
                Ok(())
            })
            .unwrap();
            assert_eq!(called, 0);
            assert_eq!(outcome.outcome, "Failure");
            assert!(outcome.succeeded.is_empty());
            assert!(outcome.failed.is_empty());
            assert!(!outcome.enabled_after);

            let fetched = get(conn, app.id).unwrap().unwrap();
            assert!(!fetched.enabled, "application must stay disabled on failure");

            let events = crate::db::audit::query(conn, Some(app.id), 100).unwrap();
            assert!(
                events
                    .iter()
                    .any(|e| e.action == "application.enable" && e.result == "Failure"),
                "expected a Failure summary audit event"
            );
        });
    }

    #[test]
    fn enable_records_per_resource_audit_and_aggregates_outcome() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            set_enabled(conn, app.id, false).unwrap();
            take_resource_snapshot(conn, app.id, "pipeline", "p1", "{}").unwrap();
            take_resource_snapshot(conn, app.id, "datasource", "binance", "{}").unwrap();

            let outcome = application_enable(conn, app.id, |snap| {
                if snap.resource_id == "binance" {
                    Err("cluster unreachable".into())
                } else {
                    Ok(())
                }
            })
            .unwrap();

            assert_eq!(outcome.outcome, "Degraded");
            assert_eq!(
                outcome.succeeded,
                vec![ResourceOp {
                    kind: "pipeline".into(),
                    id: "p1".into()
                }]
            );
            assert_eq!(
                outcome.failed,
                vec![FailedResource {
                    kind: "datasource".into(),
                    id: "binance".into(),
                    reason: "cluster unreachable".into()
                }]
            );
            assert!(outcome.enabled_after);

            let fetched = get(conn, app.id).unwrap().unwrap();
            assert!(fetched.enabled);

            let events = crate::db::audit::query(conn, Some(app.id), 100).unwrap();
            let per_resource: Vec<_> = events
                .iter()
                .filter(|e| {
                    e.action == "application.enable"
                        && e.resource_kind.is_some()
                        && e.resource_id.is_some()
                })
                .collect();
            assert_eq!(per_resource.len(), 2);
            assert!(per_resource
                .iter()
                .any(|e| e.result == "Success" && e.resource_id.as_deref() == Some("p1")));
            assert!(per_resource
                .iter()
                .any(|e| e.result == "Failure" && e.resource_id.as_deref() == Some("binance")));

            let summary = events
                .iter()
                .find(|e| e.action == "application.enable"
                    && e.resource_id.is_none()
                    && e.result == "Degraded")
                .expect("Degraded summary event");
            assert!(summary.summary.contains("1 succeeded"));
            assert!(summary.summary.contains("1 failed"));
        });
    }

    #[test]
    fn enable_pure_failure_does_not_flip_enabled_flag() {
        run(|conn| {
            let app = create(conn, "alpha").unwrap();
            set_enabled(conn, app.id, false).unwrap();
            take_resource_snapshot(conn, app.id, "pipeline", "p1", "{}").unwrap();

            let outcome = application_enable(conn, app.id, |_| -> Result<(), String> {
                Err("admin refused".into())
            })
            .unwrap();
            assert_eq!(outcome.outcome, "Failure");
            assert!(outcome.succeeded.is_empty());
            assert_eq!(outcome.failed.len(), 1);
            assert!(!outcome.enabled_after);

            let fetched = get(conn, app.id).unwrap().unwrap();
            assert!(!fetched.enabled);
        });
    }
}