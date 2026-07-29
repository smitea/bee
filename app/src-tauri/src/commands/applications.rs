use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct ApplicationView {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub display_order: i64,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DisableSnapshotView {
    pub application_id: i64,
    pub taken_at: i64,
    pub payload_json: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct DisableReport {
    pub application: ApplicationView,
    pub snapshot: DisableSnapshotView,
    pub pipelines: Vec<String>,
    pub datasources: Vec<String>,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn to_view(a: db::applications::Application) -> ApplicationView {
    ApplicationView {
        id: a.id,
        name: a.name,
        enabled: a.enabled,
        display_order: a.display_order,
        created_at: a.created_at,
    }
}

fn snapshot_to_view(s: db::applications::DisableSnapshot) -> DisableSnapshotView {
    DisableSnapshotView {
        application_id: s.application_id,
        taken_at: s.taken_at,
        payload_json: s.payload_json,
    }
}

#[tauri::command]
pub fn applications_list(app: AppHandle) -> CmdResult<Vec<ApplicationView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::applications::list(&conn)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn application_create(app: AppHandle, name: String) -> CmdResult<ApplicationView> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let created = db::applications::create(&conn, &name).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.create",
        result: "Success",
        summary: &format!("Application \"{}\" created", created.name),
        resource_kind: Some("application"),
        resource_id: None,
        application_id: Some(created.id),
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    });
    Ok(to_view(created))
}

#[tauri::command]
pub fn application_set_enabled(
    app: AppHandle,
    id: i64,
    enabled: bool,
) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::applications::set_enabled(&conn, id, enabled).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: if enabled { "application.enable" } else { "application.disable" },
        result: "Success",
        summary: if enabled { "Application enabled" } else { "Application disabled" },
        resource_kind: Some("application"),
        resource_id: None,
        application_id: Some(id),
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    });
    Ok(())
}

#[tauri::command]
pub fn application_enable(app: AppHandle, id: i64) -> CmdResult<ApplicationView> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let existing = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_enable: no row {id}") })?;
    if !existing.enabled {
        db::applications::set_enabled(&conn, id, true).map_err(CmdError::from)?;
    }
    let updated = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_enable: vanished {id}") })?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.enable",
        result: "Success",
        summary: &format!("Application \"{}\" enabled", updated.name),
        resource_kind: Some("application"),
        resource_id: None,
        application_id: Some(id),
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    });
    Ok(to_view(updated))
}

#[tauri::command]
pub fn application_disable(app: AppHandle, id: i64) -> CmdResult<DisableReport> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let existing = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_disable: no row {id}") })?;

    let pipelines = db::pipelines::list(&conn).map_err(CmdError::from)?;
    let datasources = db::datasources::list(&conn).map_err(CmdError::from)?;

    let payload_json =
        db::applications::snapshot_payload(&pipelines, &datasources).map_err(CmdError::from)?;
    let snapshot =
        db::applications::record_disable_snapshot(&conn, id, &payload_json).map_err(CmdError::from)?;

    if existing.enabled {
        db::applications::set_enabled(&conn, id, false).map_err(CmdError::from)?;
    }
    let updated = db::applications::get(&conn, id)
        .map_err(CmdError::from)?
        .ok_or_else(|| CmdError { message: format!("application_disable: vanished {id}") })?;

    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.disable",
        result: "Success",
        summary: &format!(
            "Application \"{}\" disabled (snapshot has {} pipelines, {} datasources)",
            updated.name,
            pipelines.len(),
            datasources.len()
        ),
        resource_kind: Some("application"),
        resource_id: None,
        application_id: Some(id),
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("application"),
        nav_resource_id: None,
    });

    Ok(DisableReport {
        application: to_view(updated),
        snapshot: snapshot_to_view(snapshot),
        pipelines: pipelines.into_iter().map(|p| p.name).collect(),
        datasources: datasources.into_iter().map(|d| d.name).collect(),
    })
}

#[tauri::command]
pub fn application_delete(app: AppHandle, id: i64) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::applications::delete(&conn, id).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "application.delete",
        result: "Success",
        summary: "Application deleted",
        resource_kind: Some("application"),
        resource_id: None,
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: None,
        nav_resource_id: None,
    });
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
pub struct ImportReportView {
    pub created: Vec<String>,
    pub skipped: Vec<String>,
}

#[tauri::command]
pub fn application_export(
    app: AppHandle,
    name: String,
    passphrase: String,
    out_path: String,
) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let path = std::path::Path::new(&out_path);
    crate::import_export::export_application(db, &name, &passphrase, path)
        .map_err(CmdError::from)?;
    Ok(())
}

#[tauri::command]
pub fn application_import(
    app: AppHandle,
    file_path: String,
    passphrase: String,
) -> CmdResult<ImportReportView> {
    let db = db_handle(&app)?;
    let path = std::path::Path::new(&file_path);
    let report = crate::import_export::import_application(db, path, &passphrase)
        .map_err(CmdError::from)?;
    Ok(ImportReportView {
        created: report.created.into_iter().map(|a| a.name).collect(),
        skipped: report.skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::tempdir;

    fn run<F: FnOnce(&Database)>(f: F) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        f(&db);
    }

    #[test]
    fn snapshot_payload_includes_empty_lists_for_no_resources() {
        let json = db::applications::snapshot_payload(&[], &[]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v.get("pipelines").unwrap().as_array().unwrap().len(), 0);
        assert_eq!(v.get("datasources").unwrap().as_array().unwrap().len(), 0);
    }

    #[test]
    fn snapshot_payload_serializes_pipelines_and_datasources_by_id_and_name() {
        let pipelines = vec![db::pipelines::PipelineDefinition {
            id: 7,
            name: "alpha".into(),
            dag_json: "{}".into(),
            updated_at: 0,
        }];
        let datasources = vec![db::datasources::Datasource {
            name: "binance".into(),
            plugin: "binance_subscribe".into(),
            config: "{}".into(),
            tenant: 0,
            created_at: 0,
            updated_at: 0,
        }];
        let json = db::applications::snapshot_payload(&pipelines, &datasources).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ps = v.get("pipelines").unwrap().as_array().unwrap();
        let ds = v.get("datasources").unwrap().as_array().unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].get("id").unwrap().as_i64(), Some(7));
        assert_eq!(ps[0].get("name").unwrap().as_str(), Some("alpha"));
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].get("name").unwrap().as_str(), Some("binance"));
    }

    #[test]
    fn disable_lifecycle_writes_snapshot_disables_app_and_records_audit() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            let _ = db::pipelines::create(&conn, "p1", "{}").unwrap();
            let _ = db::datasources::create(&conn, "binance", "binance_subscribe", "{}", 0).unwrap();

            let pipelines = db::pipelines::list(&conn).unwrap();
            let datasources = db::datasources::list(&conn).unwrap();
            let payload = db::applications::snapshot_payload(&pipelines, &datasources).unwrap();
            let snap =
                db::applications::record_disable_snapshot(&conn, app.id, &payload).unwrap();
            db::applications::set_enabled(&conn, app.id, false).unwrap();
            let _ = db::audit::record(
                &conn,
                db::audit::NewAuditEvent {
                    actor: "user",
                    action: "application.disable",
                    result: "Success",
                    summary: "Application disabled",
                    resource_kind: Some("application"),
                    resource_id: None,
                    application_id: Some(app.id),
                    correlation_id: None,
                    operation_id: None,
                    nav_kind: Some("application"),
                    nav_resource_id: None,
                },
            );

            let after = db::applications::get(&conn, app.id).unwrap().unwrap();
            assert!(!after.enabled);
            let snaps = db::applications::list_disable_snapshots(&conn, app.id).unwrap();
            assert_eq!(snaps.len(), 1);
            assert_eq!(snaps[0].payload_json, snap.payload_json);
            let events = db::audit::query(&conn, Some(app.id), 10).unwrap();
            assert!(events.iter().any(|e| e.action == "application.disable"));
        });
    }

    #[test]
    fn enable_is_idempotent_does_not_double_record_audit_for_already_enabled() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::applications::set_enabled(&conn, app.id, true).unwrap();
            let before = db::audit::query(&conn, Some(app.id), 10).unwrap().len();

            let _ = db::audit::record(
                &conn,
                db::audit::NewAuditEvent {
                    actor: "user",
                    action: "application.enable",
                    result: "Success",
                    summary: "Application enabled",
                    resource_kind: Some("application"),
                    resource_id: None,
                    application_id: Some(app.id),
                    correlation_id: None,
                    operation_id: None,
                    nav_kind: Some("application"),
                    nav_resource_id: None,
                },
            );
            let after = db::audit::query(&conn, Some(app.id), 10).unwrap().len();
            assert_eq!(after, before + 1);
            let after_app = db::applications::get(&conn, app.id).unwrap().unwrap();
            assert!(after_app.enabled);
        });
    }
}
