use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct AuditEventView {
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

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn to_view(e: db::audit::AuditEvent) -> AuditEventView {
    AuditEventView {
        id: e.id,
        timestamp: e.timestamp,
        actor: e.actor,
        action: e.action,
        result: e.result,
        summary: e.summary,
        resource_kind: e.resource_kind,
        resource_id: e.resource_id,
        application_id: e.application_id,
        correlation_id: e.correlation_id,
        operation_id: e.operation_id,
        nav_kind: e.nav_kind,
        nav_resource_id: e.nav_resource_id,
    }
}

#[tauri::command]
pub fn audit_list(app: AppHandle, limit: Option<i64>) -> CmdResult<Vec<AuditEventView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let n = limit.unwrap_or(100).clamp(1, 1000);
    db::audit::list(&conn, n)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn audit_query(
    app: AppHandle,
    application_id: Option<i64>,
    limit: Option<i64>,
) -> CmdResult<Vec<AuditEventView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let n = limit.unwrap_or(100).clamp(1, 1000);
    db::audit::query(&conn, application_id, n)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn audit_latest(app: AppHandle) -> CmdResult<Option<AuditEventView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::audit::latest(&conn)
        .map_err(CmdError::from)
        .map(|opt| opt.map(to_view))
}

#[tauri::command]
pub fn audit_record(
    app: AppHandle,
    actor: String,
    action: String,
    result: String,
    summary: String,
    resource_kind: Option<String>,
    resource_id: Option<String>,
    application_id: Option<i64>,
    correlation_id: Option<String>,
    operation_id: Option<String>,
    nav_kind: Option<String>,
    nav_resource_id: Option<String>,
) -> CmdResult<i64> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let id = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: &actor,
        action: &action,
        result: &result,
        summary: &summary,
        resource_kind: resource_kind.as_deref(),
        resource_id: resource_id.as_deref(),
        application_id,
        correlation_id: correlation_id.as_deref(),
        operation_id: operation_id.as_deref(),
        nav_kind: nav_kind.as_deref(),
        nav_resource_id: nav_resource_id.as_deref(),
    }).map_err(CmdError::from)?;
    Ok(id)
}