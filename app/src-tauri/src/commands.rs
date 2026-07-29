//! Tauri commands exposed to the React frontend.
//!
//! Each `#[tauri::command]` is callable from JS via `invoke('name', { args })`.

use bee_control::raft::{
    AdminRequest, AdminResponse, ClusterMetricsDetail, JobDetail, JobSummary,
};
use serde::Serialize;

use crate::connection::{self, ConnectionHandle};

/// Result wrapper for serializable errors (Tauri requires Serialize on
/// error returns).
#[derive(Debug, Serialize)]
pub struct CmdError {
    pub message: String,
}

impl<E: std::fmt::Display> From<E> for CmdError {
    fn from(e: E) -> Self {
        Self { message: e.to_string() }
    }
}

type CmdResult<T> = Result<T, CmdError>;

#[tauri::command]
pub async fn ping(addr: String) -> CmdResult<String> {
    let handle = ensure_handle(&addr).await?;
    let _rx = handle.call(AdminRequest::Ping).await.map_err(CmdError::from)?;
    Ok("Pong (queued)".to_string())
}

#[tauri::command]
pub async fn cluster_status(addr: String) -> CmdResult<ClusterMetricsDetail> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle
        .call(AdminRequest::ClusterStatus)
        .await
        .map_err(CmdError::from)?;
    let resp = rx
        .await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::ClusterMetrics(detail) => Ok(detail),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError {
            message: format!("unexpected: {other:?}"),
        }),
    }
}

#[tauri::command]
pub async fn list_jobs(addr: String) -> CmdResult<Vec<JobSummary>> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle
        .call(AdminRequest::ListJobs)
        .await
        .map_err(CmdError::from)?;
    let resp = rx
        .await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::JobList(jobs) => Ok(jobs),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError {
            message: format!("unexpected: {other:?}"),
        }),
    }
}

#[tauri::command]
pub async fn job_inspect(addr: String, id: u32) -> CmdResult<Option<JobDetail>> {
    let handle = ensure_handle(&addr).await?;
    let rx = handle
        .call(AdminRequest::JobInspect(id))
        .await
        .map_err(CmdError::from)?;
    let resp = rx
        .await
        .map_err(|e| CmdError { message: format!("recv: {e:?}") })?
        .map_err(|s| CmdError { message: s })?;
    match resp {
        AdminResponse::JobDetail(d) => Ok(d),
        AdminResponse::Error(msg) => Err(CmdError { message: msg }),
        other => Err(CmdError {
            message: format!("unexpected: {other:?}"),
        }),
    }
}

#[tauri::command]
pub async fn connection_state(addr: String) -> CmdResult<connection_state::StateView> {
    let handle = ensure_handle(&addr).await?;
    Ok(connection_state::StateView {
        addr: handle.addr().to_string(),
        state: handle.state().tag().to_string(),
        connected: matches!(handle.state(), connection::ConnectionState::Connected),
    })
}

pub mod connection_state {
    use serde::Serialize;
    #[derive(Debug, Serialize)]
    pub struct StateView {
        pub addr: String,
        pub state: String,
        pub connected: bool,
    }
}

/// Get a connection handle, bootstrapping one on demand if the JS frontend
/// hasn't yet called the lib's `run()`. Wrapped in a `tokio::sync::Mutex`
/// so concurrent commands don't race on `install_bundle`.
static HANDLE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn ensure_handle(addr: &str) -> Result<ConnectionHandle, CmdError> {
    if let Ok(h) = connection::with_handle(|h| Ok(h.clone())) {
        return Ok(h);
    }
    let _guard = HANDLE_LOCK.lock().await;
    // Re-check under the lock.
    if let Ok(h) = connection::with_handle(|h| Ok(h.clone())) {
        return Ok(h);
    }
    let parsed = connection::addr_parse(addr).map_err(CmdError::from)?;
    let bundle = connection::spawn(parsed);
    let handle = bundle.handle.clone();
    connection::install_bundle(bundle);
    Ok(handle)
}