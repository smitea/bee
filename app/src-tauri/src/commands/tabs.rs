use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct TabView {
    pub id: i64,
    pub kind: String,
    pub resource_id: Option<String>,
    pub title: String,
    pub pinned: bool,
    pub position: i64,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceState {
    pub active_tab_id: Option<i64>,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

#[tauri::command]
pub fn tabs_list(app: AppHandle) -> CmdResult<Vec<TabView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::tabs::list_open(&conn)
        .map_err(CmdError::from)
        .map(|tabs| {
            tabs.into_iter()
                .map(|t| TabView {
                    id: t.id,
                    kind: t.kind,
                    resource_id: t.resource_id,
                    title: t.title,
                    pinned: t.pinned,
                    position: t.position,
                })
                .collect()
        })
}

#[tauri::command]
pub fn tab_open(
    app: AppHandle,
    kind: String,
    resource_id: Option<String>,
    title: String,
) -> CmdResult<i64> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::tabs::open(&conn, &kind, resource_id.as_deref(), &title)
        .map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_close(app: AppHandle, id: i64) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::tabs::close(&conn, id).map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_close_others(app: AppHandle, keep_id: i64) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::tabs::close_others(&conn, keep_id).map_err(|_| CmdError {
        message: "close_others failed".into(),
    })?;
    Ok(())
}

#[tauri::command]
pub fn tab_pin(app: AppHandle, id: i64, pinned: bool) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::tabs::set_pinned(&conn, id, pinned).map_err(CmdError::from)
}

#[tauri::command]
pub fn tab_set_active(app: AppHandle, id: Option<i64>) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    match id {
        Some(tab_id) => db::tabs::set_active(&conn, tab_id).map_err(CmdError::from),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn workspace_state(app: AppHandle) -> CmdResult<WorkspaceState> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let active_tab_id = db::tabs::get_active(&conn).map_err(CmdError::from)?;
    Ok(WorkspaceState { active_tab_id })
}
