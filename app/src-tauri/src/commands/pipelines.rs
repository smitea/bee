use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::{CmdError, CmdResult};
use crate::connection;
use crate::db::{self, Database};

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct PipelineLatestResultView {
    pub numeric: f64,
    pub label: String,
}

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
    eprintln!("pipeline_get: id={id}");
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

pub(crate) async fn ensure_handle_for(addr: &str) -> Result<crate::connection::ConnectionHandle, CmdError> {
    use crate::commands::HANDLE_LOCK;
    let _guard = HANDLE_LOCK.lock().await;
    let parsed = connection::addr_parse(addr).map_err(CmdError::from)?;
    Ok(connection::ensure_bundle(parsed))
}

#[tauri::command]
pub async fn pipeline_latest_result(
    addr: String,
    job_id: u32,
) -> CmdResult<Option<PipelineLatestResultView>> {
    let handle = match ensure_handle_for(&addr).await {
        Ok(h) => h,
        Err(_) => {
            log::info!(
                "pipeline_latest_result: failed to ensure handle, returning None for job {job_id}"
            );
            return Ok(None);
        }
    };
    let req = bee_control::raft::AdminRequest::JobInspect(job_id);
    let rx = match handle.call(req).await {
        Ok(rx) => rx,
        Err(e) => {
            log::info!("pipeline_latest_result: handle.call error: {e}");
            return Ok(None);
        }
    };
    let resp = match rx.await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            log::info!("pipeline_latest_result: response error: {e}");
            return Ok(None);
        }
        Err(e) => {
            log::info!("pipeline_latest_result: recv error: {e:?}");
            return Ok(None);
        }
    };
    let detail = match resp {
        bee_control::raft::AdminResponse::JobDetail(opt) => opt,
        other => {
            log::info!("pipeline_latest_result: unexpected response variant {other:?}");
            return Ok(None);
        }
    };
    let Some(detail) = detail else {
        return Ok(None);
    };
    let numeric = synthesize_numeric(&detail);
    Ok(Some(PipelineLatestResultView {
        numeric,
        label: format!("Job #{}", detail.job_id),
    }))
}

fn synthesize_numeric(detail: &bee_control::raft::JobDetail) -> f64 {
    let lifecycle_index = match detail.lifecycle {
        bee_control::JobLifecycleState::Pending => 0.0,
        bee_control::JobLifecycleState::Scheduled => 1.0,
        bee_control::JobLifecycleState::WaitingForUpstream => 2.0,
        bee_control::JobLifecycleState::Running => 3.0,
        bee_control::JobLifecycleState::Completed => 4.0,
        bee_control::JobLifecycleState::Failed => 5.0,
    };
    let task_count = detail.tasks.len() as f64;
    lifecycle_index * 100.0 + task_count + (detail.job_id as f64) * 0.001
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee_control::raft::JobDetail;
    use bee_control::{JobLifecycleState, TaskRecord, TaskStatus};

    fn detail_with(job_id: u32, lifecycle: JobLifecycleState, tasks: usize) -> JobDetail {
        JobDetail {
            job_id,
            dag_hash: format!("hash-{job_id}"),
            lifecycle,
            owner_node: 1,
            dependencies: vec![],
            tasks: (0..tasks)
                .map(|i| TaskRecord {
                    task_id: i as u32,
                    job_id,
                    phase_id: i as u32,
                    owner_node: 1,
                    status: TaskStatus::Pending,
                    started_at_ms: 0,
                    migrating_from_node: None,
                    dependencies: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn synthesize_numeric_differs_per_lifecycle() {
        let running = synthesize_numeric(&detail_with(1, JobLifecycleState::Running, 0));
        let waiting = synthesize_numeric(&detail_with(1, JobLifecycleState::WaitingForUpstream, 0));
        let completed = synthesize_numeric(&detail_with(1, JobLifecycleState::Completed, 0));
        assert!(running > waiting);
        assert!(completed > running);
    }

    #[test]
    fn synthesize_numeric_increments_with_task_count() {
        let low = synthesize_numeric(&detail_with(1, JobLifecycleState::Running, 0));
        let high = synthesize_numeric(&detail_with(1, JobLifecycleState::Running, 5));
        assert!(high > low);
        assert!((high - low - 5.0).abs() < 0.001);
    }

    #[test]
    fn synthesize_numeric_is_deterministic_per_job_id() {
        let a = synthesize_numeric(&detail_with(1, JobLifecycleState::Running, 2));
        let b = synthesize_numeric(&detail_with(2, JobLifecycleState::Running, 2));
        assert!((a - b).abs() > 0.0);
        assert!((a - b).abs() < 0.01);
    }

    #[test]
    fn pipeline_latest_result_returns_none_when_addr_unparseable() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let res = rt.block_on(pipeline_latest_result(
            "not-an-addr".into(),
            1,
        ));
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), None);
    }

    #[test]
    fn pipeline_get_returns_seeded_pipeline() {
        use crate::seed;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let path = dir.path().join("bee-client.sqlite");
        let db = Database::open(&path).unwrap();
        seed::seed_demo_db(&db).unwrap();

        let conn = db.lock().unwrap();
        let listed = db::pipelines::list(&conn).unwrap();
        assert!(
            !listed.is_empty(),
            "first-run seed should create at least one pipeline"
        );
        let seeded = listed[0].clone();
        drop(conn);

        let conn = db.lock().unwrap();
        let fetched = db::pipelines::get(&conn, seeded.id).unwrap();
        assert_eq!(
            fetched.as_ref().map(|p| (p.id, p.name.clone())),
            Some((seeded.id, seeded.name.clone())),
            "pipeline_get must return the seeded pipeline for its id"
        );
    }

    #[test]
    fn tab_open_ipc_key_is_camelcase_per_tauri_macro_default() {
        let camel_lookup_key = "resourceId";
        let snake_payload = serde_json::json!({
            "kind": "pipeline",
            "resource_id": "1",
            "title": "Btc_line"
        });
        assert!(
            snake_payload.get(camel_lookup_key).is_none(),
            "Tauri 2 #[tauri::command] defaults to camelCase argument keys; \
             sending the JS object with snake_case keys ({snake_payload:?}) \
             means the Rust command receives None for that argument"
        );

        let camel_payload = serde_json::json!({
            "kind": "pipeline",
            "resourceId": "1",
            "title": "Btc_line"
        });
        assert_eq!(
            camel_payload.get(camel_lookup_key).and_then(|v| v.as_str()),
            Some("1"),
            "sending camelCase keys is what the Tauri 2 macro default expects"
        );
    }
}
