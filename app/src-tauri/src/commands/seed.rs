use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::Database;
use crate::seed;

#[derive(Debug, Serialize, Clone)]
pub struct SeedReportView {
    pub created: bool,
    pub application_id: Option<i64>,
    pub pipeline_id: Option<i64>,
    pub datasource_name: Option<String>,
    pub audit_events: usize,
}

impl From<seed::SeedReport> for SeedReportView {
    fn from(r: seed::SeedReport) -> Self {
        Self {
            created: r.created,
            application_id: r.application_id,
            pipeline_id: r.pipeline_id,
            datasource_name: r.datasource_name,
            audit_events: r.audit_events,
        }
    }
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

#[tauri::command]
pub fn seed_demo(app: AppHandle) -> CmdResult<SeedReportView> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let report = seed::seed_demo(&conn).map_err(CmdError::from)?;
    Ok(report.into())
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
    fn seed_demo_idempotent_through_command_path() {
        run(|db| {
            let conn = db.lock().unwrap();
            let report1 = seed::seed_demo(&conn).unwrap();
            assert!(report1.created);
            drop(conn);

            let conn2 = db.lock().unwrap();
            let report2 = seed::seed_demo(&conn2).unwrap();
            assert!(!report2.created);
        });
    }
}
