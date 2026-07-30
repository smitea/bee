use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone)]
pub struct PipelineDumpView {
    pub pipeline_id: i64,
    pub dump_json: String,
    pub created_at: i64,
}

fn db_handle(app: &AppHandle) -> Result<&Database, CmdError> {
    let state = app
        .try_state::<Database>()
        .ok_or_else(|| CmdError { message: "db not initialised".into() })?;
    Ok(state.inner())
}

fn to_view(d: db::pipeline_dumps::PipelineDump) -> PipelineDumpView {
    PipelineDumpView {
        pipeline_id: d.pipeline_id,
        dump_json: d.dump_json,
        created_at: d.created_at,
    }
}

#[tauri::command]
pub fn pipeline_dump_list(
    app: AppHandle,
    pipeline_id: i64,
) -> CmdResult<Vec<PipelineDumpView>> {
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    db::pipeline_dumps::list_for_pipeline(&conn, pipeline_id)
        .map_err(CmdError::from)
        .map(|xs| xs.into_iter().map(to_view).collect())
}

#[tauri::command]
pub fn pipeline_dump_record(
    app: AppHandle,
    pipeline_id: i64,
    dump_json: String,
) -> CmdResult<PipelineDumpView> {
    if dump_json.len() > 1024 * 1024 {
        return Err(CmdError {
            message: "pipeline dump_json exceeds 1 MiB limit".into(),
        });
    }
    let db = db_handle(&app)?;
    let conn = db.lock().map_err(CmdError::from)?;
    let saved = db::pipeline_dumps::record(&conn, pipeline_id, &dump_json)
        .map_err(CmdError::from)?;
    Ok(to_view(saved))
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
    fn record_then_list_round_trips_via_db_repo() {
        run(|db| {
            let conn = db.lock().unwrap();
            db::pipeline_dumps::record(&conn, 1, r#"{"v":1}"#).unwrap();
            let xs = db::pipeline_dumps::list_for_pipeline(&conn, 1).unwrap();
            assert_eq!(xs.len(), 1);
            assert_eq!(xs[0].dump_json, r#"{"v":1}"#);
        });
    }

    #[test]
    fn to_view_extracts_all_fields() {
        let d = db::pipeline_dumps::PipelineDump {
            pipeline_id: 9,
            dump_json: "{}".into(),
            created_at: 42,
        };
        let v = to_view(d);
        assert_eq!(v.pipeline_id, 9);
        assert_eq!(v.dump_json, "{}");
        assert_eq!(v.created_at, 42);
    }
}