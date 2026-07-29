use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct ProfileView {
    pub id: i64,
    pub label: String,
    pub addr: String,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

#[tauri::command]
pub fn profiles_list(app: AppHandle) -> CmdResult<Vec<ProfileView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::profiles::list(&conn)
        .map_err(CmdError::from)
        .map(|items| {
            items
                .into_iter()
                .map(|p| ProfileView {
                    id: p.id,
                    label: p.label,
                    addr: p.addr,
                    last_used_at: p.last_used_at,
                    created_at: p.created_at,
                })
                .collect()
        })
}

#[tauri::command]
pub fn profile_save(app: AppHandle, label: String, addr: String) -> CmdResult<i64> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::profiles::save(&conn, &label, &addr).map_err(CmdError::from)
}

#[tauri::command]
pub fn profile_remove(app: AppHandle, addr: String) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::profiles::remove(&conn, &addr).map_err(CmdError::from)
}
