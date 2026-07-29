use bee_control::raft::{
    AdminRequest, AdminResponse, ClusterMetricsDetail, JobDetail, JobSummary,
};

use crate::commands::{CmdError, CmdResult};
use crate::connection::{self, ConnectionHandle};
use crate::commands::HANDLE_LOCK;

pub(crate) async fn ensure_handle(addr: &str) -> Result<ConnectionHandle, CmdError> {
    let _guard = HANDLE_LOCK.lock().await;
    let parsed = connection::addr_parse(addr).map_err(CmdError::from)?;
    Ok(connection::ensure_bundle(parsed))
}

#[tauri::command]
pub async fn cluster_status(addr: String) -> CmdResult<ClusterMetricsDetail> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle.call(AdminRequest::ClusterStatus).await.map_err(CmdError::from)?;
    let resp = rx.await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::ClusterMetrics(d) => Ok(d),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError { message: format!("unexpected: {other:?}") }),
    }
}

#[tauri::command]
pub async fn list_jobs(addr: String) -> CmdResult<Vec<JobSummary>> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle.call(AdminRequest::ListJobs).await.map_err(CmdError::from)?;
    let resp = rx.await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::JobList(j) => Ok(j),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError { message: format!("unexpected: {other:?}") }),
    }
}

#[tauri::command]
pub async fn job_inspect(addr: String, id: u32) -> CmdResult<Option<JobDetail>> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle.call(AdminRequest::JobInspect(id)).await.map_err(CmdError::from)?;
    let resp = rx.await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::JobDetail(d) => Ok(d),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError { message: format!("unexpected: {other:?}") }),
    }
}
