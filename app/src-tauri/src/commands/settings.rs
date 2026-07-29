use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, serde::Serialize)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

#[tauri::command]
pub fn settings_get(app: AppHandle, key: String) -> CmdResult<Option<String>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::settings::get(&conn, &key).map_err(CmdError::from)
}

#[tauri::command]
pub fn settings_put(app: AppHandle, key: String, value: String) -> CmdResult<()> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::settings::put(&conn, &key, &value).map_err(CmdError::from)
}

#[tauri::command]
pub fn settings_list(app: AppHandle) -> CmdResult<Vec<Setting>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let pairs = db::settings::list(&conn).map_err(CmdError::from)?;
    Ok(pairs
        .into_iter()
        .map(|(key, value)| Setting { key, value })
        .collect())
}
