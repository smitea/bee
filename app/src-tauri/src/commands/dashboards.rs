use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

#[derive(Debug, Serialize, Clone)]
pub struct DashboardView {
    pub application_id: i64,
    pub layout_json: String,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct DashboardSaveInput {
    pub application_id: i64,
    pub layout_json: String,
}

fn to_view(d: db::dashboards::Dashboard) -> DashboardView {
    DashboardView {
        application_id: d.application_id,
        layout_json: d.layout_json,
        updated_at: d.updated_at,
    }
}

#[tauri::command]
pub fn dashboard_get(
    app: AppHandle,
    application_id: i64,
) -> CmdResult<Option<DashboardView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::dashboards::get(&conn, application_id)
        .map_err(CmdError::from)
        .map(|opt| opt.map(to_view))
}

#[tauri::command]
pub fn dashboard_save(
    app: AppHandle,
    application_id: i64,
    layout_json: String,
) -> CmdResult<DashboardView> {
    if layout_json.len() > 256 * 1024 {
        return Err(CmdError {
            message: "dashboard layout_json exceeds 256 KiB limit".into(),
        });
    }
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::dashboards::upsert(&conn, application_id, &layout_json)
        .map_err(CmdError::from)
        .map(to_view)
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
    fn save_returns_view_with_application_id_and_updated_at() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            let view = to_view(
                db::dashboards::upsert(&conn, app.id, r#"{"p":1}"#).unwrap(),
            );
            assert_eq!(view.application_id, app.id);
            assert_eq!(view.layout_json, r#"{"p":1}"#);
            assert!(view.updated_at > 0);
        });
    }

    #[test]
    fn get_returns_none_when_unset() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            let result = db::dashboards::get(&conn, app.id).unwrap();
            assert!(result.is_none());
        });
    }

    #[test]
    fn save_overwrites_existing_layout() {
        run(|db| {
            let conn = db.lock().unwrap();
            let app = db::applications::create(&conn, "alpha").unwrap();
            db::dashboards::upsert(&conn, app.id, r#"{"v":1}"#).unwrap();
            let after = db::dashboards::upsert(&conn, app.id, r#"{"v":2}"#).unwrap();
            assert_eq!(after.layout_json, r#"{"v":2}"#);
        });
    }
}
