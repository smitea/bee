use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::connection;
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct PipelineSummary {
    pub id: String,
    pub name: String,
    pub dag_hash: String,
    pub lifecycle: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct PipelineView {
    pub id: i64,
    pub name: String,
    pub dag_json: String,
    pub updated_at: i64,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn to_view(p: db::pipelines::PipelineDefinition) -> PipelineView {
    PipelineView {
        id: p.id,
        name: p.name,
        dag_json: p.dag_json,
        updated_at: p.updated_at,
    }
}

#[tauri::command]
pub async fn pipelines_list(addr: String) -> CmdResult<Vec<PipelineSummary>> {
    let parsed = match connection::addr_parse(&addr) {
        Ok(p) => p,
        Err(_) => return Ok(Vec::new()),
    };
    let handle = match connection::with_handle(|h| Ok(h.clone())) {
        Ok(h) if matches!(h.state(), connection::ConnectionState::Connected) => h,
        _ => return Ok(Vec::new()),
    };
    let _ = parsed;
    let _ = handle.call(bee_control::raft::AdminRequest::ListJobs).await;
    Ok(Vec::new())
}

#[tauri::command]
pub fn pipeline_list(app: AppHandle) -> CmdResult<Vec<PipelineView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::pipelines::list(&conn)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn pipeline_create(app: AppHandle, name: String, dag_json: String) -> CmdResult<PipelineView> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::pipelines::create(&conn, &name, &dag_json)
        .map(to_view)
        .map_err(CmdError::from)
}

#[tauri::command]
pub fn pipeline_get(app: AppHandle, id: i64) -> CmdResult<Option<PipelineView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::pipelines::get(&conn, id)
        .map(|opt| opt.map(to_view))
        .map_err(CmdError::from)
}

#[tauri::command]
pub fn pipeline_delete(app: AppHandle, id: i64) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::pipelines::delete(&conn, id).map_err(CmdError::from)
}
