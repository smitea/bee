use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct DatasourceView {
    pub name: String,
    pub plugin: String,
    pub config: String,
    pub tenant: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

pub fn to_view(d: db::datasources::Datasource) -> DatasourceView {
    DatasourceView {
        name: d.name,
        plugin: d.plugin,
        config: d.config,
        tenant: d.tenant,
        created_at: d.created_at,
        updated_at: d.updated_at,
    }
}

#[tauri::command]
pub fn datasource_list(app: AppHandle) -> CmdResult<Vec<DatasourceView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::datasources::list(&conn)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn datasource_create(
    app: AppHandle,
    name: String,
    plugin: String,
    config_json: String,
    tenant: i64,
) -> CmdResult<DatasourceView> {
    let tenant_u16 = if tenant < 0 || tenant > u16::MAX as i64 {
        return Err(CmdError {
            message: crate::tenant::validate_tenant(0)
                .err()
                .unwrap_or_else(|| "datasource tenant out of range".into()),
        });
    } else {
        tenant as u16
    };
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let created = db::datasources::create(&conn, &name, &plugin, &config_json, tenant_u16 as i64)
        .map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "datasource.create",
        result: "Success",
        summary: &format!(
            "Datasource \"{}\" created (tenant={})",
            created.name, created.tenant
        ),
        resource_kind: Some("datasource"),
        resource_id: Some(&created.name),
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("datasource"),
        nav_resource_id: Some(&created.name),
    });
    Ok(to_view(created))
}

#[tauri::command]
pub fn datasource_delete(app: AppHandle, name: String) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::datasources::delete(&conn, &name).map_err(CmdError::from)?;
    let _ = db::audit::record(&conn, db::audit::NewAuditEvent {
        actor: "user",
        action: "datasource.delete",
        result: "Success",
        summary: &format!("Datasource \"{name}\" deleted"),
        resource_kind: Some("datasource"),
        resource_id: Some(&name),
        application_id: None,
        correlation_id: None,
        operation_id: None,
        nav_kind: Some("datasource"),
        nav_resource_id: Some(&name),
    });
    Ok(())
}