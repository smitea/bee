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